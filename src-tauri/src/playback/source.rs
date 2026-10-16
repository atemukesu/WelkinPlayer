// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Seekable byte sources for the playback engine.
//!
//! The decoder needs `Read + Seek` over a whole track. A local file is opened
//! directly; a WebDAV track is served from disk — the complete "smart" cache
//! when present, otherwise the read-ahead stream cache, whose background
//! downloader is rebased on every seek. No loopback HTTP is involved: the
//! decoder reads the cache file in-process.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tauri::AppHandle;

use crate::commands::media::asset_hash;
use crate::core::error::AppError;
use crate::data::backend::safe_join;
use crate::data::sources::{find_source, SourceConfig};
use crate::streaming::smart_cache::SmartCache;
use crate::streaming::stream_cache::{CacheReader, Entry, StreamCache, TotalState};

/// Blanket trait so the decoder can be handed any boxed, thread-safe reader.
pub trait ReadSeek: Read + Seek + Send + Sync {}
impl<T: Read + Seek + Send + Sync> ReadSeek for T {}

/// Open a seekable byte source for `path` inside `source_id`.
///
/// Returns the reader and the resource's byte length when known. The length is
/// handed to the decoder so Symphonia can binary-search for a seek target
/// instead of reporting the stream as unseekable.
pub fn open_track(
    app: &AppHandle,
    cache: &Arc<StreamCache>,
    smart: &Arc<SmartCache>,
    source_id: &str,
    path: &str,
) -> Result<(Box<dyn ReadSeek>, Option<u64>), AppError> {
    let source = find_source(app, source_id)?;

    if source.is_local() {
        let root = source.root_path.clone().unwrap_or_default();
        if root.trim().is_empty() {
            return Err(AppError::Store("本地来源缺少目录".to_string()));
        }
        let file = safe_join(&PathBuf::from(root), path)?;
        let file = File::open(file)?;
        let len = file.metadata().ok().map(|meta| meta.len());
        return Ok((Box::new(file), len));
    }

    let key = asset_hash(source_id, path);
    // A fully cached track is served straight from disk: perfect random access.
    if let Some(file) = smart.complete_file(&key) {
        log::debug!("playback source: smart-cache file for {path}");
        let file = File::open(file)?;
        let len = file.metadata().ok().map(|meta| meta.len());
        return Ok((Box::new(file), len));
    }

    // Once the transient cache is complete, use it as a normal file. Passing a
    // growing CacheSource to Symphonia makes some format readers report
    // `Unseekable`, even though every byte is already available.
    let entry = cache.ensure(&key);
    if let Some(file) = cache.complete_file(&entry) {
        log::debug!("playback source: complete stream-cache file for {path}");
        let file = File::open(file)?;
        let len = file.metadata().ok().map(|meta| meta.len());
        return Ok((Box::new(file), len));
    }

    // Otherwise the read-ahead cache writes the bytes to disk as they arrive and
    // this reader blocks on them. Kicking the downloader off at offset 0 makes
    // playback start on the first byte. The total may already be known from the
    // upstream headers, which is enough to let the decoder seek.
    log::debug!("playback source: streaming {path} via read-ahead cache");
    cache.set_active(&key);
    cache.touch(&key);
    cache.ensure_stream(app, &source, path, &entry, 0, None);
    let reader = cache.reader_from(&entry, 0);
    let len = cache.cached_total(&entry);
    Ok((
        Box::new(CacheSource {
            app: app.clone(),
            cache: cache.clone(),
            source,
            path: path.to_string(),
            entry,
            reader,
            offset: 0,
        }),
        len,
    ))
}

struct CacheSource {
    app: AppHandle,
    cache: Arc<StreamCache>,
    source: SourceConfig,
    path: String,
    entry: Arc<Entry>,
    reader: CacheReader,
    offset: u64,
}

impl Read for CacheSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.reader.read(buf)?;
        self.offset += read as u64;
        Ok(read)
    }
}

impl Seek for CacheSource {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        log::debug!(
            "playback source: CacheSource seek request {pos:?} (offset {})",
            self.offset
        );
        let target = match pos {
            SeekFrom::Start(offset) => offset,
            SeekFrom::Current(delta) => (self.offset as i64 + delta).max(0) as u64,
            SeekFrom::End(delta) => {
                let sizing_started = std::time::Instant::now();
                let total = self.cache.cached_total(&self.entry).or_else(|| {
                    match self.cache.wait_total(&self.entry, Duration::from_secs(20)) {
                        TotalState::Known(total) => Some(total),
                        TotalState::Failed(status, message) => {
                            log::warn!(
                                "cache download failed while sizing track ({status}): {message}"
                            );
                            None
                        }
                        TotalState::Unknown => None,
                    }
                });
                log::debug!(
                    "playback source: SeekFrom::End sizing took {:?} -> {:?}",
                    sizing_started.elapsed(),
                    total,
                );
                let Some(total) = total else {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "resource size unknown",
                    ));
                };
                (total as i64 + delta).max(0) as u64
            }
        };
        // Rebase the downloader onto the new offset so bytes are fetched from
        // there, then reopen the blocking reader at the same position.
        log::debug!("playback source: CacheSource seek -> {target} (generation rebase)");
        self.cache.ensure_stream(
            &self.app,
            &self.source,
            &self.path,
            &self.entry,
            target,
            None,
        );
        self.reader = self.cache.reader_from(&self.entry, target);
        self.offset = target;
        Ok(target)
    }
}
