//! Minimal WebDAV client built on `reqwest`.
//!
//! Wraps the handful of methods Welkin needs (PROPFIND, GET, HEAD, PUT, MOVE)
//! with Basic Auth, plus a PROPFIND multistatus parser and a recursive audio
//! listing helper. Remote paths use URL semantics; every [`RemoteEntry`] also
//! carries a decoded, POSIX-style `local_path` relative to the collection root
//! so the frontend never has to deal with Windows/POSIX separator mixes.

use std::path::Path;
use std::time::Duration;

use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::Reader;
use reqwest::{header, Client, Method, StatusCode, Url};
use serde::{Deserialize, Serialize};

use crate::error::AppError;

const AUDIO_EXTENSIONS: [&str; 8] = ["mp3", "flac", "m4a", "ogg", "wav", "aac", "opus", "wma"];
const MAX_DIRECTORIES: usize = 1024;

/// A single file or collection returned by a PROPFIND listing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteEntry {
    /// Display name (falls back to the file name when the server omits it).
    pub name: String,
    /// Remote path as it appears in the `href` (percent-encoded).
    pub path: String,
    /// Decoded path relative to the collection root, using `/` separators.
    pub local_path: String,
    /// Whether the entry is a collection (directory).
    pub is_dir: bool,
    /// `getcontentlength` in bytes, when provided.
    pub size: Option<u64>,
    /// `getlastmodified` as returned by the server (RFC 1123 string).
    pub modified: Option<String>,
    /// `getcontenttype`, when provided.
    pub content_type: Option<String>,
}

impl RemoteEntry {
    fn from_raw(raw: RawEntry) -> Self {
        RemoteEntry {
            name: raw.name,
            path: raw.href,
            local_path: String::new(),
            is_dir: raw.is_dir,
            size: raw.size,
            modified: raw.modified,
            content_type: raw.content_type,
        }
    }
}

#[derive(Debug, Default)]
struct RawEntry {
    href: String,
    name: String,
    is_dir: bool,
    size: Option<u64>,
    modified: Option<String>,
    content_type: Option<String>,
}

/// A lightweight WebDAV client bound to a single collection + credentials.
pub struct WebDavClient {
    base: Url,
    username: String,
    password: String,
    http: Client,
}

// GET/HEAD/PUT/MOVE are part of the WebDAV surface but not yet called by the
// current commands; keep them available for upcoming upload/download features.
#[allow(dead_code)]
impl WebDavClient {
    /// Build a client for `base` (e.g. `https://dav.example.com/music`).
    pub fn new(base: &str, username: &str, password: &str) -> Result<Self, AppError> {
        let base = base.trim();
        if base.is_empty() {
            return Err(AppError::invalid_argument("url", "服务器地址不能为空"));
        }

        let mut normalized = base.to_string();
        if !normalized.ends_with('/') {
            normalized.push('/');
        }

        let base = Url::parse(&normalized).map_err(|error| {
            AppError::invalid_argument("url", format!("无效的服务器地址：{error}"))
        })?;

        let http = Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(30))
            .build()?;

        Ok(WebDavClient {
            base,
            username: username.to_string(),
            password: password.to_string(),
            http,
        })
    }

    fn resolve(&self, path: &str) -> Result<Url, AppError> {
        resolve_under_base(&self.base, path)
    }

    fn request(&self, method: Method, url: Url) -> reqwest::RequestBuilder {
        self.http
            .request(method, url)
            .basic_auth(&self.username, Some(&self.password))
    }

    /// Issue a `PROPFIND` request and return the raw multistatus XML body.
    pub async fn propfind(&self, path: &str, depth: u32) -> Result<String, AppError> {
        let url = self.resolve(path)?;
        let body = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
            <d:propfind xmlns:d=\"DAV:\"><d:prop>\
            <d:displayname/><d:getcontentlength/><d:getlastmodified/>\
            <d:getcontenttype/><d:resourcetype/>\
            </d:prop></d:propfind>";

        let response = self
            .request(dav_method(b"PROPFIND"), url)
            .header("Depth", depth.to_string())
            .header(header::CONTENT_TYPE, "application/xml; charset=utf-8")
            .body(body)
            .send()
            .await?;

        Ok(self.finish(response).await?.text().await?)
    }

    /// `GET` a resource; the caller decides how to consume the body.
    pub async fn get(&self, path: &str) -> Result<reqwest::Response, AppError> {
        let url = self.resolve(path)?;
        let response = self.request(Method::GET, url).send().await?;
        self.finish(response).await
    }

    /// `HEAD` a resource.
    pub async fn head(&self, path: &str) -> Result<reqwest::Response, AppError> {
        let url = self.resolve(path)?;
        let response = self.request(Method::HEAD, url).send().await?;
        self.finish(response).await
    }

    /// Fetch at most `max_bytes` from the start of a resource using a single
    /// HTTP `Range` request.
    ///
    /// Used to read audio file headers for metadata without downloading the
    /// whole file. If a server ignores `Range` and streams the full body, the
    /// response is still only read up to the cap.
    pub async fn get_range(&self, path: &str, max_bytes: u64) -> Result<Vec<u8>, AppError> {
        let url = self.resolve(path)?;
        let mut response = self
            .request(Method::GET, url)
            .header(
                header::RANGE,
                format!("bytes=0-{}", max_bytes.saturating_sub(1)),
            )
            .send()
            .await?;

        let status = response.status();
        if !(status.is_success() || status == StatusCode::PARTIAL_CONTENT) {
            let message = response.text().await.unwrap_or_default();
            return Err(classify_status(status, &message));
        }

        let cap = max_bytes as usize;
        let mut buffer: Vec<u8> = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            let remaining = cap - buffer.len();
            if chunk.len() >= remaining {
                buffer.extend_from_slice(&chunk[..remaining]);
                break;
            }
            buffer.extend_from_slice(&chunk);
        }
        Ok(buffer)
    }

    /// `PUT` bytes to a resource.
    pub async fn put(&self, path: &str, content_type: &str, body: Vec<u8>) -> Result<(), AppError> {
        let url = self.resolve(path)?;
        let response = self
            .request(Method::PUT, url)
            .header(header::CONTENT_TYPE, content_type.to_string())
            .body(body)
            .send()
            .await?;
        self.finish(response).await?;
        Ok(())
    }

    /// `MOVE` a resource to a new path within the same server.
    pub async fn move_to(&self, from: &str, to: &str) -> Result<(), AppError> {
        let url = self.resolve(from)?;
        let destination = self.resolve(to)?;
        let response = self
            .request(dav_method(b"MOVE"), url)
            .header("Destination", destination.as_str())
            .header("Overwrite", "T")
            .send()
            .await?;
        self.finish(response).await?;
        Ok(())
    }

    /// `DELETE` a resource.
    pub async fn delete(&self, path: &str) -> Result<(), AppError> {
        let url = self.resolve(path)?;
        let response = self.request(Method::DELETE, url).send().await?;
        self.finish(response).await?;
        Ok(())
    }

    /// Parse a PROPFIND multistatus body into normalized entries.
    pub fn parse_multistatus(&self, xml: &str) -> Result<Vec<RemoteEntry>, AppError> {
        let raw = parse_raw(xml)?;
        let mut entries = Vec::with_capacity(raw.len());

        for entry in raw {
            let mut normalized = RemoteEntry::from_raw(entry);
            // Never trust the server's `href`: keep only a same-origin path so a
            // hostile listing can't make us send credentials to another host.
            match self.normalize_href(&normalized.path) {
                Some(path) => normalized.path = path,
                None => {
                    log::warn!(
                        "ignoring WebDAV entry outside the configured server: {}",
                        normalized.path
                    );
                    continue;
                }
            }
            normalized.local_path = self.local_path_for(&normalized.path);
            if normalized.name.trim().is_empty() {
                normalized.name = normalized
                    .local_path
                    .rsplit('/')
                    .next()
                    .unwrap_or_default()
                    .to_string();
            }
            entries.push(normalized);
        }

        Ok(entries)
    }

    /// Reduce a server-provided `href` to a same-origin path + query. Returns
    /// `None` when it points outside the configured server or collection root.
    fn normalize_href(&self, href: &str) -> Option<String> {
        let resolved = resolve_under_base(&self.base, href).ok()?;
        let mut safe = resolved.path().to_string();
        if let Some(query) = resolved.query() {
            safe.push('?');
            safe.push_str(query);
        }
        Some(safe)
    }

    /// Recursively list audio files under `root` (one PROPFIND per directory).
    pub async fn list_audio_files(&self, root: &str) -> Result<Vec<RemoteEntry>, AppError> {
        let mut files = Vec::new();
        let mut queue = vec![root.trim().trim_matches('/').to_string()];
        let mut visited = 0usize;

        while let Some(directory) = queue.pop() {
            if visited >= MAX_DIRECTORIES {
                break;
            }
            visited += 1;

            let xml = self.propfind(&directory, 1).await?;
            for entry in self.parse_multistatus(&xml)? {
                let key = entry.local_path.trim_matches('/');
                if key == directory.trim_matches('/') {
                    continue;
                }

                if entry.is_dir {
                    queue.push(key.to_string());
                } else if is_audio_file(&entry.name) {
                    files.push(entry);
                }
            }
        }

        files.sort_by(|a, b| a.local_path.cmp(&b.local_path));
        Ok(files)
    }

    /// Strip the collection prefix from a URL and percent-decode the remainder.
    fn local_path_for(&self, href: &str) -> String {
        let Ok(url) = self.base.join(href) else {
            return href.trim_start_matches('/').to_string();
        };

        let segments = |url: &Url| -> Vec<String> {
            url.path_segments()
                .map(|segments| {
                    segments
                        .filter(|segment| !segment.is_empty())
                        .map(|segment| {
                            percent_encoding::percent_decode_str(segment)
                                .decode_utf8_lossy()
                                .into_owned()
                        })
                        .collect()
                })
                .unwrap_or_default()
        };

        let full = segments(&url);
        let prefix = segments(&self.base);

        let rest = if full.len() >= prefix.len() && full[..prefix.len()] == prefix[..] {
            &full[prefix.len()..]
        } else {
            &full[..]
        };

        rest.join("/")
    }

    async fn finish(&self, response: reqwest::Response) -> Result<reqwest::Response, AppError> {
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }

        let message = response.text().await.unwrap_or_default();
        Err(classify_status(status, &message))
    }
}

fn dav_method(name: &[u8]) -> Method {
    Method::from_bytes(name).expect("well-known WebDAV method token")
}

/// Resolve `path` against `base`, refusing to leave the configured origin or
/// escape the collection root.
///
/// This is the guard that keeps Basic Auth from being sent to another host: an
/// absolute URL (`http://evil/…`), a protocol-relative path (`//evil/…`) or a
/// `..` traversal all resolve to a different origin or a path outside `base`,
/// and are rejected. The `path` ultimately comes from the remote server's
/// `href` and the webview, so neither is trusted.
pub(crate) fn resolve_under_base(base: &Url, path: &str) -> Result<Url, AppError> {
    let resolved = base
        .join(path.trim())
        .map_err(|error| AppError::invalid_argument("path", format!("无效的路径：{error}")))?;

    let same_origin = resolved.scheme() == base.scheme()
        && resolved.host_str() == base.host_str()
        && resolved.port_or_known_default() == base.port_or_known_default();
    let inside_root = resolved.path().starts_with(base.path());
    let no_credentials = resolved.username().is_empty() && resolved.password().is_none();

    if !same_origin || !inside_root || !no_credentials {
        return Err(AppError::invalid_argument(
            "path",
            "解析结果超出了已配置的服务器或音乐库目录",
        ));
    }

    Ok(resolved)
}

fn classify_status(status: StatusCode, message: &str) -> AppError {
    match status.as_u16() {
        401 => AppError::Unauthorized,
        403 => AppError::Forbidden,
        404 => AppError::WebdavNotFound("远端路径不存在".to_string()),
        code => AppError::Http {
            status: code,
            message: message.chars().take(200).collect(),
        },
    }
}

/// Whether a file name has one of the supported audio extensions.
pub fn is_audio_file(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| AUDIO_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn parse_raw(xml: &str) -> Result<Vec<RawEntry>, AppError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut entries = Vec::new();
    let mut current: Option<RawEntry> = None;
    let mut in_prop = false;
    let mut tag = String::new();

    loop {
        match reader
            .read_event()
            .map_err(|error| AppError::Xml(error.to_string()))?
        {
            Event::Start(element) => {
                let name = local_name(&element);
                match name.as_str() {
                    "response" => current = Some(RawEntry::default()),
                    "prop" => in_prop = true,
                    "collection" if in_prop => {
                        if let Some(entry) = current.as_mut() {
                            entry.is_dir = true;
                        }
                    }
                    _ => {}
                }
                tag = name;
            }
            Event::Empty(element) => {
                if local_name(&element) == "collection" && in_prop {
                    if let Some(entry) = current.as_mut() {
                        entry.is_dir = true;
                    }
                }
            }
            Event::Text(text) => {
                let Some(entry) = current.as_mut() else {
                    continue;
                };
                let value = text
                    .unescape()
                    .map_err(|error| AppError::Xml(error.to_string()))?
                    .into_owned();

                match tag.as_str() {
                    "href" if !in_prop => entry.href = value,
                    "displayname" if in_prop => entry.name = value,
                    "getcontentlength" if in_prop => entry.size = value.trim().parse().ok(),
                    "getlastmodified" if in_prop => entry.modified = Some(value),
                    "getcontenttype" if in_prop => entry.content_type = Some(value),
                    _ => {}
                }
            }
            Event::End(element) => {
                match end_local_name(&element).as_str() {
                    "response" => {
                        if let Some(entry) = current.take() {
                            entries.push(entry);
                        }
                    }
                    "prop" => in_prop = false,
                    _ => {}
                }
                tag.clear();
            }
            Event::Eof => break,
            _ => {}
        }
    }

    Ok(entries)
}

fn local_name(element: &BytesStart) -> String {
    String::from_utf8_lossy(element.local_name().as_ref()).into_owned()
}

fn end_local_name(element: &BytesEnd) -> String {
    String::from_utf8_lossy(element.local_name().as_ref()).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/music/</D:href>
    <D:propstat>
      <D:prop>
        <D:displayname>music</D:displayname>
        <D:resourcetype><D:collection/></D:resourcetype>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/music/Artist%20Name/</D:href>
    <D:propstat>
      <D:prop>
        <D:resourcetype><D:collection/></D:resourcetype>
      </D:prop>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/music/Artist%20Name/Song.mp3</D:href>
    <D:propstat>
      <D:prop>
        <D:displayname>Song.mp3</D:displayname>
        <D:getcontentlength>4096</D:getcontentlength>
        <D:getlastmodified>Mon, 01 Jan 2024 00:00:00 GMT</D:getlastmodified>
        <D:getcontenttype>audio/mpeg</D:getcontenttype>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
</D:multistatus>"#;

    #[test]
    fn parses_multistatus_entries() {
        let client = WebDavClient::new("https://dav.example.com/music", "user", "pass").unwrap();
        let entries = client.parse_multistatus(SAMPLE).unwrap();
        assert_eq!(entries.len(), 3);

        assert_eq!(entries[0].local_path, "");
        assert!(entries[0].is_dir);

        assert_eq!(entries[1].local_path, "Artist Name");
        assert!(entries[1].is_dir);

        assert_eq!(entries[2].local_path, "Artist Name/Song.mp3");
        assert_eq!(entries[2].size, Some(4096));
        assert_eq!(entries[2].content_type.as_deref(), Some("audio/mpeg"));
        assert!(!entries[2].is_dir);
    }

    #[test]
    fn detects_audio_extensions() {
        assert!(is_audio_file("a.mp3"));
        assert!(is_audio_file("A.FLAC"));
        assert!(is_audio_file("track.m4a"));
        assert!(!is_audio_file("cover.jpg"));
        assert!(!is_audio_file("noext"));
    }

    #[test]
    fn resolves_absolute_href_without_doubling_base_path() {
        let client = WebDavClient::new("https://host/webdav", "user", "pass").unwrap();
        let url = client.resolve("/webdav/%E9%AB%98.flac").unwrap();
        assert_eq!(url.as_str(), "https://host/webdav/%E9%AB%98.flac");
    }

    #[test]
    fn resolves_relative_directory_against_base() {
        let client = WebDavClient::new("https://host/webdav", "user", "pass").unwrap();
        let url = client.resolve("Artist Name").unwrap();
        assert_eq!(url.as_str(), "https://host/webdav/Artist%20Name");
    }

    #[test]
    fn refuses_paths_that_leave_the_configured_origin() {
        let client = WebDavClient::new("https://host/webdav", "user", "pass").unwrap();
        for path in [
            "http://evil.example/x",
            "//evil.example/x",
            "https://evil.example/x",
        ] {
            assert!(client.resolve(path).is_err(), "{path} should be rejected");
        }
    }

    #[test]
    fn refuses_paths_outside_the_collection_root() {
        let client = WebDavClient::new("https://host/webdav", "user", "pass").unwrap();
        assert!(client.resolve("../secret").is_err());
        assert!(client.resolve("%2e%2e/%2e%2e/secret").is_err());
    }

    #[test]
    fn drops_multistatus_entries_outside_the_server() {
        let client = WebDavClient::new("https://host/webdav", "user", "pass").unwrap();
        let xml = r#"<?xml version="1.0"?><D:multistatus xmlns:D="DAV:">
          <D:response><D:href>http://evil.example/x.mp3</D:href><D:propstat><D:prop>
            <D:displayname>x.mp3</D:displayname></D:prop></D:propstat></D:response>
          <D:response><D:href>/webdav/ok.mp3</D:href><D:propstat><D:prop>
            <D:displayname>ok.mp3</D:displayname></D:prop></D:propstat></D:response>
        </D:multistatus>"#;
        let entries = client.parse_multistatus(xml).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "/webdav/ok.mp3");
    }
}
