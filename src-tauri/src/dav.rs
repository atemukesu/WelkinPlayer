// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Minimal WebDAV client built on `reqwest`.
//!
//! Wraps the handful of methods Welkin needs (PROPFIND, GET, HEAD, PUT, MOVE)
//! with Basic Auth, plus a PROPFIND multistatus parser and a recursive audio
//! listing helper. Remote paths use URL semantics; every [`RemoteEntry`] also
//! carries a decoded, POSIX-style `local_path` relative to the collection root
//! so the frontend never has to deal with Windows/POSIX separator mixes.

use std::collections::HashSet;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use futures_util::stream::{self, StreamExt};
use percent_encoding::percent_decode_str;
use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::Reader;
use reqwest::{header, Client, Method, StatusCode, Url};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::AppError;

const AUDIO_EXTENSIONS: [&str; 8] = ["mp3", "flac", "m4a", "ogg", "wav", "aac", "opus", "wma"];

/// Maximum number of PROPFIND requests kept in flight while walking a library.
const LIST_CONCURRENCY: usize = 8;

/// How long a connection attempt may take before giving up.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// How long a request may stall without receiving any bytes before giving up.
///
/// This is an *inactivity* timeout, not a total-transfer deadline, so a long
/// but healthy upload/download is never aborted mid-flight.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// The `Depth` header for a PROPFIND request (RFC 4918 §10.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    /// Only the resource named by the request URI.
    Zero,
    /// The resource and its immediate children.
    One,
    /// The resource and all of its descendants (servers may refuse this).
    #[allow(dead_code)] // Part of the RFC 4918 surface; not needed by the walk.
    Infinity,
}

impl Depth {
    fn as_header(self) -> &'static str {
        match self {
            Depth::Zero => "0",
            Depth::One => "1",
            Depth::Infinity => "infinity",
        }
    }
}

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

/// The properties read from a single `<D:prop>` block.
///
/// Held separately until its `<D:propstat>` closes so a non-2xx status (for
/// example a `404` block listing the properties the server could not provide)
/// can be discarded instead of being merged into the entry.
#[derive(Debug, Default)]
struct PropValues {
    name: Option<String>,
    size: Option<u64>,
    modified: Option<String>,
    content_type: Option<String>,
    is_dir: bool,
}

/// A lightweight WebDAV client bound to a single collection + credentials.
pub struct WebDavClient {
    base: Url,
    /// Percent-decoded, non-empty segments of `base.path()`, precomputed so
    /// converting each `href` never has to re-parse or re-join the base URL.
    base_segments: Vec<String>,
    username: String,
    password: Zeroizing<String>,
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
        let base_segments = decoded_segments(base.path());

        // No global request timeout: a fixed total deadline would abort a long
        // but healthy upload/download. A connect timeout plus an inactivity
        // (read) timeout bound failures without capping the transfer size.
        let http = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(IDLE_TIMEOUT)
            .build()?;

        Ok(WebDavClient {
            base,
            base_segments,
            username: username.to_string(),
            password: Zeroizing::new(password.to_string()),
            http,
        })
    }

    fn resolve(&self, path: &str) -> Result<Url, AppError> {
        resolve_under_base(&self.base, path)
    }

    fn request(&self, method: Method, url: Url) -> reqwest::RequestBuilder {
        self.http
            .request(method, url)
            .basic_auth(&self.username, Some(self.password.as_str()))
    }

    /// Issue a `PROPFIND` request and return the raw multistatus XML body.
    pub async fn propfind(&self, path: &str, depth: Depth) -> Result<String, AppError> {
        let url = self.resolve(path)?;
        let body = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
            <d:propfind xmlns:d=\"DAV:\"><d:prop>\
            <d:displayname/><d:getcontentlength/><d:getlastmodified/>\
            <d:getcontenttype/><d:resourcetype/>\
            </d:prop></d:propfind>";

        let response = self
            .request(propfind_method(), url)
            .header("Depth", depth.as_header())
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
            .request(move_method(), url)
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
            // Never trust the server's `href`: resolve it once and keep only a
            // same-origin path, so a hostile listing cannot make us send
            // credentials to another host.
            let resolved = match resolve_under_base(&self.base, &entry.href) {
                Ok(url) => url,
                Err(_) => {
                    log::warn!(
                        "ignoring WebDAV entry outside the configured server: {}",
                        entry.href
                    );
                    continue;
                }
            };

            let mut normalized = RemoteEntry::from_raw(entry);
            normalized.local_path = self.local_path_for(&resolved);
            normalized.path = path_with_query(&resolved);
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

    /// Recursively list audio files under `root` (one PROPFIND per directory).
    ///
    /// Directories are walked level by level with a bounded number of PROPFIND
    /// requests in flight, so a wide or deep library lists far faster than the
    /// old one-request-at-a-time walk without flooding a small server.
    ///
    /// Every directory is listed exactly once: `seen` records what has already
    /// been queued so a server that reports a cycle (or the same collection
    /// twice) cannot spin forever. There is deliberately no cap on how many
    /// directories may be visited — a large library must list in full, and a
    /// truncated listing would look like mass deletion to the caller.
    pub async fn list_audio_files(&self, root: &str) -> Result<Vec<RemoteEntry>, AppError> {
        let mut files = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut frontier = vec![root.trim().trim_matches('/').to_string()];
        seen.insert(frontier[0].clone());

        while !frontier.is_empty() {
            // Consume this level's directories so each future owns its path.
            // Mapping a borrowed iterator to an async block would force the
            // closure to be higher-ranked over the borrow, which Rust rejects.
            let directory_batch = std::mem::take(&mut frontier);
            let listings = stream::iter(directory_batch)
                .map(|directory| async move {
                    let xml = self.propfind(&directory, Depth::One).await?;
                    let entries = self.parse_multistatus(&xml)?;
                    Ok::<_, AppError>((directory, entries))
                })
                .buffer_unordered(LIST_CONCURRENCY)
                .collect::<Vec<_>>()
                .await;

            let mut next = Vec::new();
            for listing in listings {
                let (directory, entries) = listing?;
                let directory_key = directory.trim_matches('/');
                for entry in entries {
                    let key = entry.local_path.trim_matches('/').to_string();
                    if key == directory_key {
                        continue;
                    }

                    if entry.is_dir {
                        if seen.insert(key.clone()) {
                            next.push(key);
                        }
                    } else if is_audio_file(&entry.name) {
                        files.push(entry);
                    }
                }
            }

            frontier = next;
        }

        files.sort_by(|a, b| a.local_path.cmp(&b.local_path));
        Ok(files)
    }

    /// Strip the collection prefix from an already-resolved URL and
    /// percent-decode the remainder.
    fn local_path_for(&self, url: &Url) -> String {
        let full = decoded_segments(url.path());
        let prefix = &self.base_segments;

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

/// A cached, well-known WebDAV method token.
fn propfind_method() -> Method {
    static METHOD: OnceLock<Method> = OnceLock::new();
    METHOD
        .get_or_init(|| Method::from_bytes(b"PROPFIND").expect("well-known WebDAV method token"))
        .clone()
}

/// A cached `MOVE` method token.
fn move_method() -> Method {
    static METHOD: OnceLock<Method> = OnceLock::new();
    METHOD
        .get_or_init(|| Method::from_bytes(b"MOVE").expect("well-known WebDAV method token"))
        .clone()
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
    let inside_root = path_within_root(resolved.path(), base.path());
    let no_credentials = resolved.username().is_empty() && resolved.password().is_none();

    if !same_origin || !inside_root || !no_credentials {
        return Err(AppError::invalid_argument(
            "path",
            "解析结果超出了已配置的服务器或音乐库目录",
        ));
    }

    Ok(resolved)
}

/// Whether `path` is the collection root itself or a descendant of it.
///
/// A plain `starts_with` is not enough: with a root of `/webdav/` the sibling
/// `/webdav-evil/…` also starts with `/webdav`, and a root `href` without a
/// trailing slash (`/webdav`) would fail to match. Requiring a path-segment
/// boundary fixes both.
fn path_within_root(path: &str, base_path: &str) -> bool {
    let root = base_path.trim_end_matches('/');
    if root.is_empty() {
        return true;
    }

    match path.strip_prefix(root) {
        Some(rest) => rest.is_empty() || rest.starts_with('/'),
        None => false,
    }
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

/// The percent-decoded, non-empty path segments of a URL path.
fn decoded_segments(path: &str) -> Vec<String> {
    path.split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| percent_decode_str(segment).decode_utf8_lossy().into_owned())
        .collect()
}

/// The percent-encoded path (plus query, when present) of a resolved URL.
fn path_with_query(url: &Url) -> String {
    let mut path = url.path().to_string();
    if let Some(query) = url.query() {
        path.push('?');
        path.push_str(query);
    }
    path
}

/// Whether a `DAV:status` value like `HTTP/1.1 200 OK` denotes success.
///
/// A missing or unparseable status is treated as success: dropping properties
/// because of a malformed status line would silently lose media entries.
fn propstat_is_success(status: &str) -> bool {
    status
        .split_whitespace()
        .find_map(|token| token.parse::<u16>().ok())
        .map(|code| (200..300).contains(&code))
        .unwrap_or(true)
}

fn parse_raw(xml: &str) -> Result<Vec<RawEntry>, AppError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut entries = Vec::new();
    let mut current: Option<RawEntry> = None;
    let mut pending = PropValues::default();
    let mut in_prop = false;
    let mut in_propstat = false;
    let mut propstat_ok = true;
    let mut tag = String::new();

    loop {
        match reader
            .read_event()
            .map_err(|error| AppError::Xml(error.to_string()))?
        {
            Event::Start(element) => {
                let name = local_name(&element);
                match name.as_str() {
                    "response" => {
                        current = Some(RawEntry::default());
                        pending = PropValues::default();
                        in_prop = false;
                        in_propstat = false;
                        propstat_ok = true;
                    }
                    "propstat" => {
                        in_propstat = true;
                        // Assume success until a `<status>` says otherwise; a
                        // propstat without a status must not lose its values.
                        propstat_ok = true;
                        pending = PropValues::default();
                    }
                    "prop" if in_propstat => in_prop = true,
                    "collection" if in_prop => pending.is_dir = true,
                    _ => {}
                }
                tag = name;
            }
            Event::Empty(element) => {
                if local_name(&element) == "collection" && in_prop {
                    pending.is_dir = true;
                }
            }
            Event::Text(text) => {
                let value = text
                    .unescape()
                    .map_err(|error| AppError::Xml(error.to_string()))?
                    .into_owned();
                record_value(
                    current.as_mut(),
                    &mut pending,
                    &tag,
                    in_prop,
                    in_propstat,
                    &mut propstat_ok,
                    value,
                );
            }
            Event::CData(cdata) => {
                let value = cdata
                    .decode()
                    .map_err(|error| AppError::Xml(error.to_string()))?
                    .into_owned();
                record_value(
                    current.as_mut(),
                    &mut pending,
                    &tag,
                    in_prop,
                    in_propstat,
                    &mut propstat_ok,
                    value,
                );
            }
            Event::End(element) => {
                match end_local_name(&element).as_str() {
                    "response" => {
                        if let Some(entry) = current.take() {
                            entries.push(entry);
                        }
                    }
                    "prop" => in_prop = false,
                    "propstat" => {
                        in_propstat = false;
                        if propstat_ok {
                            if let Some(entry) = current.as_mut() {
                                apply_props(entry, std::mem::take(&mut pending));
                            }
                        }
                    }
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

/// Route a text/CDATA value to the field named by the current element.
fn record_value(
    entry: Option<&mut RawEntry>,
    pending: &mut PropValues,
    tag: &str,
    in_prop: bool,
    in_propstat: bool,
    propstat_ok: &mut bool,
    value: String,
) {
    let Some(entry) = entry else {
        return;
    };

    if tag == "status" && in_propstat && !in_prop {
        *propstat_ok = propstat_is_success(&value);
        return;
    }

    if !in_prop {
        // `href` is a direct child of `<response>`, not of `<prop>`.
        if tag == "href" {
            entry.href = value;
        }
        return;
    }

    match tag {
        "displayname" => pending.name = Some(value),
        "getcontentlength" => pending.size = value.trim().parse().ok(),
        "getlastmodified" => pending.modified = Some(value),
        "getcontenttype" => pending.content_type = Some(value),
        _ => {}
    }
}

/// Merge the buffered properties of a successful propstat into the entry.
fn apply_props(entry: &mut RawEntry, props: PropValues) {
    if let Some(name) = props.name {
        entry.name = name;
    }
    if let Some(size) = props.size {
        entry.size = Some(size);
    }
    if let Some(modified) = props.modified {
        entry.modified = Some(modified);
    }
    if let Some(content_type) = props.content_type {
        entry.content_type = Some(content_type);
    }
    if props.is_dir {
        entry.is_dir = true;
    }
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
    fn refuses_sibling_that_only_shares_a_name_prefix() {
        let client = WebDavClient::new("https://host/webdav", "user", "pass").unwrap();
        assert!(client.resolve("/webdav-evil/x.mp3").is_err());
    }

    #[test]
    fn accepts_collection_root_without_trailing_slash() {
        let client = WebDavClient::new("https://host/webdav", "user", "pass").unwrap();
        assert_eq!(client.resolve("/webdav").unwrap().as_str(), "https://host/webdav");
    }

    #[test]
    fn maps_root_href_without_trailing_slash_to_empty_local_path() {
        let client = WebDavClient::new("https://host/webdav", "user", "pass").unwrap();
        let xml = r#"<?xml version="1.0"?><D:multistatus xmlns:D="DAV:">
          <D:response><D:href>/webdav</D:href><D:propstat><D:prop>
            <D:resourcetype><D:collection/></D:resourcetype>
          </D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>
        </D:multistatus>"#;
        let entries = client.parse_multistatus(xml).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].local_path, "");
        assert!(entries[0].is_dir);
    }

    #[test]
    fn ignores_properties_from_non_success_propstat() {
        let client = WebDavClient::new("https://host/webdav", "user", "pass").unwrap();
        let xml = r#"<?xml version="1.0"?><D:multistatus xmlns:D="DAV:">
          <D:response><D:href>/webdav/a.mp3</D:href>
            <D:propstat><D:prop><D:displayname>a.mp3</D:displayname></D:prop>
              <D:status>HTTP/1.1 404 Not Found</D:status></D:propstat>
            <D:propstat><D:prop><D:getcontentlength>10</D:getcontentlength></D:prop>
              <D:status>HTTP/1.1 200 OK</D:status></D:propstat>
          </D:response>
        </D:multistatus>"#;
        let entries = client.parse_multistatus(xml).unwrap();
        assert_eq!(entries.len(), 1);
        // The displayname lives in the 404 propstat, so it must be ignored and
        // the name must fall back to the file name.
        assert_eq!(entries[0].name, "a.mp3");
        assert_eq!(entries[0].size, Some(10));
    }

    #[test]
    fn reads_cdata_text() {
        let client = WebDavClient::new("https://host/webdav", "user", "pass").unwrap();
        let xml = r#"<?xml version="1.0"?><D:multistatus xmlns:D="DAV:">
          <D:response><D:href>/webdav/a.mp3</D:href><D:propstat><D:prop>
            <D:displayname><![CDATA[Weird <Name> & Co]]></D:displayname>
          </D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>
        </D:multistatus>"#;
        let entries = client.parse_multistatus(xml).unwrap();
        assert_eq!(entries[0].name, "Weird <Name> & Co");
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
