//! Loopback streaming proxy.
//!
//! A tiny HTTP server bound to `127.0.0.1:<random port>` streams audio from a
//! configured source to the webview. For a WebDAV source it proxies the upstream
//! response (keeping Basic Auth in Rust); for a local source it serves the file
//! from disk. Both honour `Range` requests, so the browser gets native
//! progressive playback and seeking. Requests are gated by a per-launch token.

use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use percent_encoding::percent_decode_str;
use reqwest::blocking::Client;
use serde::Serialize;
use tauri::AppHandle;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

use crate::backend::{content_type_for, safe_join};
use crate::commands::webdav::{enforce_url_policy, source_password};
use crate::dav::resolve_under_base;
use crate::sources::find_source;

/// Shared state handed to the frontend via the `stream_endpoint` command.
pub struct StreamProxy {
    pub port: u16,
    pub token: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamEndpoint {
    pub url: String,
    pub token: String,
}

/// Start the proxy and return its address + token. The server runs for the
/// lifetime of the process on a background thread.
pub fn start(app: AppHandle) -> Result<StreamProxy, String> {
    let server = Server::http("127.0.0.1:0").map_err(|error| error.to_string())?;
    let port = server
        .server_addr()
        .to_ip()
        .map(|addr| addr.port())
        .ok_or_else(|| "could not determine proxy port".to_string())?;
    let token = generate_token();

    let thread_token = token.clone();
    std::thread::spawn(move || {
        let client = match Client::builder().build() {
            Ok(client) => Arc::new(client),
            Err(error) => {
                log::error!("streaming proxy client failed: {error}");
                return;
            }
        };

        for request in server.incoming_requests() {
            let app = app.clone();
            let token = thread_token.clone();
            let client = client.clone();
            std::thread::spawn(move || handle(request, &app, &token, &client));
        }
    });

    Ok(StreamProxy { port, token })
}

/// Address of the streaming proxy for the frontend.
#[tauri::command]
pub fn stream_endpoint(proxy: tauri::State<StreamProxy>) -> StreamEndpoint {
    StreamEndpoint {
        url: format!("http://127.0.0.1:{}/stream", proxy.port),
        token: proxy.token.clone(),
    }
}

fn handle(request: Request, app: &AppHandle, token: &str, client: &Client) {
    if request.method() == &Method::Options {
        let _ = request.respond(empty(204));
        return;
    }

    let params = parse_query(request.url());
    let provided = params
        .iter()
        .find(|(key, _)| key == "token")
        .map(|(_, value)| value.as_str());
    if provided != Some(token) {
        let _ = request.respond(text(403, "forbidden"));
        return;
    }

    let remote_path = params
        .iter()
        .find(|(key, _)| key == "path")
        .map(|(_, value)| value.clone())
        .unwrap_or_default();
    let source_id = params
        .iter()
        .find(|(key, _)| key == "source")
        .map(|(_, value)| value.clone())
        .unwrap_or_default();
    let range = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("Range"))
        .map(|header| header.value.as_str().to_string());

    let source = match find_source(app, &source_id) {
        Ok(source) => source,
        Err(error) => {
            let _ = request.respond(text(404, &error.to_string()));
            return;
        }
    };

    if source.is_local() {
        match local_root(&source) {
            Ok(root) => serve_local(request, &root, &remote_path, range.as_deref()),
            Err(error) => {
                let _ = request.respond(text(400, &error));
            }
        }
        return;
    }

    serve_webdav(request, app, client, &source, &remote_path, range.as_deref());
}

/// The directory of a local source, validated to exist.
fn local_root(source: &crate::sources::SourceConfig) -> Result<PathBuf, String> {
    let root = source.root_path.clone().unwrap_or_default();
    if root.trim().is_empty() {
        return Err("local source has no directory".to_string());
    }
    let root = PathBuf::from(root);
    if !root.is_dir() {
        return Err(format!("本地来源目录不存在：{}", root.display()));
    }
    Ok(root)
}

/// Serve a local file, honouring a single `Range` header.
fn serve_local(request: Request, root: &Path, remote_path: &str, range: Option<&str>) {
    let file_path = match safe_join(root, remote_path) {
        Ok(path) => path,
        Err(error) => {
            let _ = request.respond(text(400, &error.to_string()));
            return;
        }
    };
    let mut file = match std::fs::File::open(&file_path) {
        Ok(file) => file,
        Err(_) => {
            let _ = request.respond(text(404, "not found"));
            return;
        }
    };
    let total = match file.seek(SeekFrom::End(0)) {
        Ok(total) => total,
        Err(error) => {
            let _ = request.respond(text(500, &error.to_string()));
            return;
        }
    };
    let content_type = content_type_for(remote_path)
        .unwrap_or_else(|| "application/octet-stream".to_string());

    let mut headers = vec![Header::from_bytes("Accept-Ranges", "bytes").unwrap()];
    if let Ok(header) = Header::from_bytes("Content-Type", content_type.as_bytes()) {
        headers.push(header);
    }

    match range.and_then(|value| parse_range(value, total)) {
        Some((start, end)) => {
            let length = end.saturating_sub(start) + 1;
            if file.seek(SeekFrom::Start(start)).is_err() {
                let _ = request.respond(text(500, "seek failed"));
                return;
            }
            if let Ok(header) = Header::from_bytes(
                "Content-Range",
                format!("bytes {start}-{end}/{total}").as_bytes(),
            ) {
                headers.push(header);
            }
            let reader = file.take(length);
            let _ = request.respond(Response::new(
                StatusCode(206),
                headers,
                reader,
                Some(length as usize),
                None,
            ));
        }
        None => {
            let reader = file.take(total);
            let _ = request.respond(Response::new(
                StatusCode(200),
                headers,
                reader,
                Some(total as usize),
                None,
            ));
        }
    }
}

/// Parse a `Range: bytes=start-end` header into an inclusive range.
fn parse_range(value: &str, total: u64) -> Option<(u64, u64)> {
    if total == 0 {
        return None;
    }
    let spec = value.trim().strip_prefix("bytes=")?;
    let (start_text, end_text) = spec.split_once('-')?;

    if start_text.is_empty() {
        // Suffix range: the last N bytes.
        let suffix: u64 = end_text.trim().parse().ok()?;
        if suffix == 0 {
            return None;
        }
        let start = total.saturating_sub(suffix);
        return Some((start, total - 1));
    }

    let start: u64 = start_text.trim().parse().ok()?;
    if start >= total {
        return None;
    }
    let end = if end_text.trim().is_empty() {
        total - 1
    } else {
        end_text.trim().parse::<u64>().ok()?.min(total - 1)
    };
    (start <= end).then_some((start, end))
}

/// Proxy a `Range` request to a WebDAV source.
fn serve_webdav(
    request: Request,
    app: &AppHandle,
    client: &Client,
    source: &crate::sources::SourceConfig,
    remote_path: &str,
    range: Option<&str>,
) {
    let url = source.url.clone().unwrap_or_default();
    let username = source.username.clone().unwrap_or_default();
    if let Err(error) = enforce_url_policy(app, &url, source.allow_insecure) {
        let _ = request.respond(text(403, &error.to_string()));
        return;
    }
    let password = match source_password(app, &source.id) {
        Ok(password) => password,
        Err(error) => {
            let _ = request.respond(text(401, &error.to_string()));
            return;
        }
    };
    let url = match build_url(&url, remote_path) {
        Ok(url) => url,
        Err(error) => {
            let _ = request.respond(text(400, &error));
            return;
        }
    };

    let mut upstream_request = client.get(url).basic_auth(&username, Some(&password));
    if let Some(range) = range {
        upstream_request = upstream_request.header(reqwest::header::RANGE, range);
    }

    let upstream = match upstream_request.send() {
        Ok(response) => response,
        Err(error) => {
            let _ = request.respond(text(502, &error.to_string()));
            return;
        }
    };

    let status = upstream.status();
    if !(status.is_success() || status == reqwest::StatusCode::PARTIAL_CONTENT) {
        let message = upstream.text().unwrap_or_default();
        let _ = request.respond(text(status.as_u16(), &message));
        return;
    }

    let mut headers = vec![Header::from_bytes("Accept-Ranges", "bytes").unwrap()];
    for (name, value) in upstream.headers() {
        let name = name.as_str();
        if name == "content-type" || name == "content-range" {
            if let Ok(header) = Header::from_bytes(name.as_bytes(), value.as_bytes()) {
                headers.push(header);
            }
        }
    }

    let length = upstream.content_length().map(|value| value as usize);
    let _ = request.respond(Response::new(
        StatusCode(status.as_u16()),
        headers,
        upstream,
        length,
        None,
    ));
}

fn build_url(base: &str, remote_path: &str) -> Result<reqwest::Url, String> {
    let mut normalized = base.trim().to_string();
    if normalized.is_empty() {
        return Err("server URL is not configured".to_string());
    }
    if !normalized.ends_with('/') {
        normalized.push('/');
    }
    let base = reqwest::Url::parse(&normalized).map_err(|error| error.to_string())?;
    resolve_under_base(&base, remote_path).map_err(|error| error.to_string())
}

fn parse_query(url: &str) -> Vec<(String, String)> {
    let Some(query) = url.split_once('?').map(|(_, query)| query) else {
        return Vec::new();
    };
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (
                key.to_string(),
                percent_decode_str(value).decode_utf8_lossy().to_string(),
            )
        })
        .collect()
}

fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    match getrandom::getrandom(&mut bytes) {
        Ok(()) => bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
        Err(error) => {
            log::error!("CSPRNG unavailable for proxy token: {error}");
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0);
            let mix = nanos ^ ((std::process::id() as u128) << 48);
            format!("{mix:032x}")
        }
    }
}

fn empty(status: u16) -> Response<Cursor<Vec<u8>>> {
    Response::new(
        StatusCode(status),
        vec![Header::from_bytes("Allow", "GET, OPTIONS").unwrap()],
        Cursor::new(Vec::new()),
        Some(0),
        None,
    )
}

fn text(status: u16, message: &str) -> Response<Cursor<Vec<u8>>> {
    Response::new(
        StatusCode(status),
        vec![Header::from_bytes("Content-Type", "text/plain; charset=utf-8").unwrap()],
        Cursor::new(message.as_bytes().to_vec()),
        Some(message.len()),
        None,
    )
}
