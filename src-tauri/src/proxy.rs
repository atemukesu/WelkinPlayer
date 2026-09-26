//! Loopback streaming proxy.
//!
//! A tiny HTTP server bound to `127.0.0.1:<random port>` streams audio from the
//! configured WebDAV server to the webview. Unlike Tauri's custom protocols
//! (which must return a fully-buffered body), this pipes the upstream response
//! straight to the client, so the browser gets native progressive streaming and
//! Range-based seeking. Basic Auth stays in Rust; requests are gated by a
//! per-launch token.

use std::io::Cursor;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use percent_encoding::percent_decode_str;
use reqwest::blocking::Client;
use serde::Serialize;
use tauri::AppHandle;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

use crate::commands::webdav::require_credentials;
use crate::dav::resolve_under_base;

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
        // Built inside the plain OS thread (never in an async context).
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
    let range = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("Range"))
        .map(|header| header.value.as_str().to_string());

    let credentials = match require_credentials(app) {
        Ok(credentials) => credentials,
        Err(error) => {
            let _ = request.respond(text(401, &error.to_string()));
            return;
        }
    };

    let url = match build_url(&credentials.url, &remote_path) {
        Ok(url) => url,
        Err(error) => {
            let _ = request.respond(text(400, &error));
            return;
        }
    };

    let mut upstream_request = client
        .get(url)
        .basic_auth(&credentials.username, Some(&credentials.password));
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

    // `upstream` implements `Read`; `respond` streams it to the socket.
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
    // Same-origin guard: never forward Basic Auth to another host.
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
            // Should be unreachable; log loudly and fall back to a clock/pid mix
            // rather than running with a predictable token.
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
