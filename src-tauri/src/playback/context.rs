// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Playback context: the queue, shuffle/repeat, and track selection.
//!
//! Before this module existed, the play queue and every next/previous/select
//! decision lived in the webview's Pinia store. That made playback depend on the
//! webview staying alive: a backgrounded Android webview stops its timers, so the
//! native MediaSession could queue a command the webview never polled, and
//! `audio:ended` auto-advance (which the webview handled) froze.
//!
//! The context now owns all of that on the Rust side. The engine worker keeps
//! decoding the single loaded track; this context decides *which* track the
//! worker should load, and drives the engine directly. The webview is reduced to
//! a mirror: it sends intent commands and renders the `playback:changed` event.

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::playback::engine::{Command, Engine};

/// Separator between a source id and a track path inside a track key.
pub const KEY_SEPARATOR: &str = "::";

/// A track the context can hand straight to the engine: its source plus the
/// in-source path. This is the minimal identity `audio_load` needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueItem {
    pub source_id: String,
    pub path: String,
}

/// Repeat modes, mirroring the frontend's `RepeatMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RepeatMode {
    Off,
    All,
    One,
}

impl Default for RepeatMode {
    fn default() -> Self {
        RepeatMode::Off
    }
}

impl RepeatMode {
    fn cycle(self) -> Self {
        match self {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        }
    }
}

/// Serializable snapshot pushed to the webview on every change.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextSnapshot {
    /// Track keys (`sourceId::path`) of the active, possibly-shuffled queue.
    pub queue: Vec<String>,
    /// Index of the current item within `queue`, or -1 when nothing is queued.
    pub queue_index: i64,
    /// Shuffle on/off.
    pub shuffle: bool,
    /// Repeat mode.
    pub repeat: RepeatMode,
    /// Whether `current` resolved to a real item.
    pub has_track: bool,
    /// Source id of the current item (for the UI's metadata lookup), or null.
    pub current_source_id: Option<String>,
    /// Path of the current item, or null.
    pub current_path: Option<String>,
    /// Whether the engine is currently playing.
    pub is_playing: bool,
}

/// The mutable playback context, guarded by a single mutex. All intent commands
/// take the lock, mutate, then emit. The engine is driven synchronously so a
/// `next` that lands a playable track always loads it before the caller returns.
pub struct PlaybackContext {
    engine: Engine,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    /// Active, possibly-shuffled play order.
    queue: Vec<QueueItem>,
    /// Unshuffled source order, so toggling shuffle off restores it.
    base_queue: Vec<QueueItem>,
    /// Position within `queue`.
    index: i64,
    shuffle: bool,
    repeat: RepeatMode,
    /// Whether the engine should be playing once a track is loaded.
    playing: bool,
    /// Bumped when the same item is reselected, to force an engine restart.
    nonce: u64,
}

impl PlaybackContext {
    pub fn new(engine: Engine) -> Self {
        Self {
            engine,
            inner: Mutex::new(Inner::default()),
        }
    }

    /// Build a `QueueItem` list from track keys (`sourceId::path`). Keys that
    /// lack a separator are dropped (they are not playable).
    fn items_from_keys(keys: &[String]) -> Vec<QueueItem> {
        keys.iter()
            .filter_map(|key| {
                let sep = key.find(KEY_SEPARATOR)?;
                if sep == 0 || sep + KEY_SEPARATOR.len() >= key.len() {
                    return None;
                }
                Some(QueueItem {
                    source_id: key[..sep].to_string(),
                    path: key[sep + KEY_SEPARATOR.len()..].to_string(),
                })
            })
            .collect()
    }

    /// Split a key into its parts, returning None for malformed keys.
    fn split_key(key: &str) -> Option<(String, String)> {
        let sep = key.find(KEY_SEPARATOR)?;
        if sep == 0 || sep + KEY_SEPARATOR.len() >= key.len() {
            return None;
        }
        Some((
            key[..sep].to_string(),
            key[sep + KEY_SEPARATOR.len()..].to_string(),
        ))
    }

    /// Replace the whole queue (e.g. "play these tracks starting here").
    pub fn set_queue(&self, keys: Vec<String>, start_key: Option<&str>) {
        let items = Self::items_from_keys(&keys);
        let mut inner = self.inner.lock().unwrap();
        inner.base_queue = items.clone();
        let (queue, index) = if inner.shuffle && !items.is_empty() {
            // Keep the chosen start first, then shuffle the remainder.
            if let Some(start) = start_key.and_then(Self::split_key) {
                let start_item = QueueItem {
                    source_id: start.0,
                    path: start.1,
                };
                let rest: Vec<QueueItem> = items
                    .iter()
                    .filter(|item| item != &&start_item)
                    .cloned()
                    .collect();
                let mut shuffled = Self::shuffled(rest);
                shuffled.insert(0, start_item);
                (shuffled, 0i64)
            } else {
                let shuffled = Self::shuffled(items);
                (shuffled, 0)
            }
        } else {
            let index = start_key
                .and_then(Self::split_key)
                .and_then(|start| {
                    items
                        .iter()
                        .position(|item| item.source_id == start.0 && item.path == start.1)
                })
                .map(|i| i as i64)
                .unwrap_or(0);
            (items, index)
        };
        inner.queue = queue;
        inner.index = if inner.queue.is_empty() { -1 } else { index };
        let snapshot = self.snapshot_locked(&inner);
        drop(inner);
        self.apply_current(true);
        self.emit(snapshot);
    }

    /// Reselect the current queue item (same track) without changing order.
    pub fn select_index(&self, index: i64) {
        let mut inner = self.inner.lock().unwrap();
        if inner.queue.is_empty() || index < 0 || index as usize >= inner.queue.len() {
            return;
        }
        let reselecting = inner.index >= 0
            && (index as usize) < inner.queue.len()
            && inner.queue[index as usize]
                == (if inner.index >= 0 {
                    inner.queue[inner.index as usize].clone()
                } else {
                    inner.queue[index as usize].clone()
                });
        inner.index = index;
        let snapshot = self.snapshot_locked(&inner);
        drop(inner);
        if reselecting {
            // Same media: bump the nonce so the engine restarts cleanly.
            self.bump_nonce();
        } else {
            self.apply_current(true);
        }
        self.emit(snapshot);
    }

    /// Toggle play/pause.
    pub fn toggle(&self) {
        let mut inner = self.inner.lock().unwrap();
        let was_playing = inner.playing;
        inner.playing = !inner.playing;
        let snapshot = self.snapshot_locked(&inner);
        drop(inner);
        if was_playing {
            let _ = self.engine.send(Command::Pause);
        } else {
            // If nothing is loaded yet, load the current item; otherwise just
            // resume the already-loaded track.
            if self.current_loaded() {
                let _ = self.engine.send(Command::Play);
            } else {
                self.apply_current(true);
            }
        }
        self.emit(snapshot);
    }

    /// Explicit play (resume).
    pub fn play(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.playing = true;
        let snapshot = self.snapshot_locked(&inner);
        drop(inner);
        if self.current_loaded() {
            let _ = self.engine.send(Command::Play);
        } else {
            self.apply_current(true);
        }
        self.emit(snapshot);
    }

    /// Explicit pause.
    pub fn pause(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.playing = false;
        let snapshot = self.snapshot_locked(&inner);
        drop(inner);
        let _ = self.engine.send(Command::Pause);
        self.emit(snapshot);
    }

    /// Advance to the next track (honouring repeat-one / repeat-all / end).
    pub fn next(&self) {
        let mut inner = self.inner.lock().unwrap();
        if inner.queue.is_empty() {
            return;
        }
        if inner.repeat == RepeatMode::One {
            // Restart the current track; no index change.
            let snapshot = self.snapshot_locked(&inner);
            drop(inner);
            self.bump_nonce();
            self.emit(snapshot);
            return;
        }
        let mut next_index = inner.index + 1;
        if next_index >= inner.queue.len() as i64 {
            if inner.repeat != RepeatMode::All {
                // End of queue: stop, do not advance.
                inner.playing = false;
                let snapshot = self.snapshot_locked(&inner);
                drop(inner);
                let _ = self.engine.send(Command::Pause);
                self.emit(snapshot);
                return;
            }
            if inner.shuffle && inner.queue.len() > 1 {
                let current = inner.queue[inner.index as usize].clone();
                let mut reshuffled = Self::shuffled(inner.queue.clone());
                // Avoid starting the new cycle on the same track.
                if reshuffled[0] == current && reshuffled.len() > 1 {
                    reshuffled.swap(0, 1);
                }
                inner.queue = reshuffled;
            }
            next_index = 0;
        }
        inner.index = next_index;
        let snapshot = self.snapshot_locked(&inner);
        drop(inner);
        self.apply_current(true);
        self.emit(snapshot);
    }

    /// Go to the previous track.
    pub fn previous(&self) {
        let mut inner = self.inner.lock().unwrap();
        if inner.queue.is_empty() {
            return;
        }
        if inner.repeat == RepeatMode::One {
            let snapshot = self.snapshot_locked(&inner);
            drop(inner);
            self.bump_nonce();
            self.emit(snapshot);
            return;
        }
        let prev_index = if inner.index <= 0 {
            inner.queue.len() as i64 - 1
        } else {
            inner.index - 1
        };
        inner.index = prev_index;
        let snapshot = self.snapshot_locked(&inner);
        drop(inner);
        self.apply_current(true);
        self.emit(snapshot);
    }

    /// Toggle shuffle. When turning on, reshuffle keeping the current track
    /// first; when off, restore the base order and re-locate the current track.
    pub fn toggle_shuffle(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.shuffle = !inner.shuffle;
        let current = inner
            .index
            .try_into()
            .ok()
            .filter(|&i: &usize| i < inner.queue.len())
            .map(|i| inner.queue[i].clone());
        if inner.shuffle {
            if let Some(current) = current {
                let rest: Vec<QueueItem> = inner
                    .queue
                    .iter()
                    .filter(|item| item != &&current)
                    .cloned()
                    .collect();
                let mut shuffled = Self::shuffled(rest);
                shuffled.insert(0, current);
                inner.queue = shuffled;
                inner.index = 0;
            } else {
                inner.queue = Self::shuffled(inner.queue.clone());
                inner.index = -1;
            }
        } else {
            let base = if inner.base_queue.is_empty() {
                inner.queue.clone()
            } else {
                inner.base_queue.clone()
            };
            inner.queue = base;
            inner.index = match &current {
                Some(current) => inner
                    .queue
                    .iter()
                    .position(|item| item == current)
                    .map(|i| i as i64)
                    .unwrap_or(0),
                None => 0,
            };
        }
        let snapshot = self.snapshot_locked(&inner);
        drop(inner);
        self.emit(snapshot);
    }

    /// Cycle repeat mode (off -> all -> one -> off).
    pub fn cycle_repeat(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.repeat = inner.repeat.cycle();
        // One implies shuffle off for a sane cycle, matching the frontend.
        if inner.repeat == RepeatMode::One && inner.shuffle {
            inner.shuffle = false;
            let base = if inner.base_queue.is_empty() {
                inner.queue.clone()
            } else {
                inner.base_queue.clone()
            };
            let current = inner
                .index
                .try_into()
                .ok()
                .filter(|&i: &usize| i < inner.queue.len())
                .map(|i| inner.queue[i].clone());
            inner.queue = base;
            inner.index = match &current {
                Some(current) => inner
                    .queue
                    .iter()
                    .position(|item| item == current)
                    .map(|i| i as i64)
                    .unwrap_or(0),
                None => 0,
            };
        }
        let snapshot = self.snapshot_locked(&inner);
        drop(inner);
        self.emit(snapshot);
    }

    /// Set shuffle explicitly (used for restore / settings).
    pub fn set_shuffle(&self, on: bool) {
        let mut inner = self.inner.lock().unwrap();
        if inner.shuffle == on {
            return;
        }
        inner.shuffle = on;
        let current = inner
            .index
            .try_into()
            .ok()
            .filter(|&i: &usize| i < inner.queue.len())
            .map(|i| inner.queue[i].clone());
        if on {
            if let Some(current) = current {
                let rest: Vec<QueueItem> = inner
                    .queue
                    .iter()
                    .filter(|item| item != &&current)
                    .cloned()
                    .collect();
                let mut shuffled = Self::shuffled(rest);
                shuffled.insert(0, current);
                inner.queue = shuffled;
                inner.index = 0;
            }
        } else {
            let base = if inner.base_queue.is_empty() {
                inner.queue.clone()
            } else {
                inner.base_queue.clone()
            };
            inner.queue = base;
            inner.index = match &current {
                Some(current) => inner
                    .queue
                    .iter()
                    .position(|item| item == current)
                    .map(|i| i as i64)
                    .unwrap_or(0),
                None => 0,
            };
        }
        let snapshot = self.snapshot_locked(&inner);
        drop(inner);
        self.emit(snapshot);
    }

    /// Restore the queue + position from persisted state. `keys` is the saved
    /// queue; `index` is the saved index; `current_key` anchors the position
    /// when present (more reliable than a stale index). Returns false when the
    /// queue could not be rebuilt.
    pub fn restore(&self, keys: &[String], index: i64, current_key: Option<&str>) -> bool {
        let items = Self::items_from_keys(keys);
        if items.is_empty() {
            return false;
        }
        let mut inner = self.inner.lock().unwrap();
        inner.base_queue = items.clone();
        // Restore preserves the stored (possibly shuffled) order verbatim; we do
        // not re-shuffle on restore, because the saved order *is* the user's
        // order. Shuffle state itself is restored separately via set_shuffle.
        inner.queue = items;
        let anchor = current_key
            .and_then(Self::split_key)
            .and_then(|current| {
                inner
                    .queue
                    .iter()
                    .position(|item| item.source_id == current.0 && item.path == current.1)
            })
            .map(|i| i as i64);
        inner.index = anchor
            .filter(|_| anchor.is_some())
            .or_else(|| {
                if index >= 0 && (index as usize) < inner.queue.len() {
                    Some(index)
                } else {
                    Some(0)
                }
            })
            .unwrap_or(0);
        let snapshot = self.snapshot_locked(&inner);
        drop(inner);
        // Do not auto-load on restore; the engine state (and play intent) is
        // applied lazily once the UI requests playback or the resume playhead.
        self.emit(snapshot);
        true
    }

    /// Set the play intent only (used to reflect engine pause/play coming from
    /// the engine's own state, e.g. an error that stopped playback). Does not
    /// touch the queue.
    pub fn set_playing(&self, playing: bool) {
        let mut inner = self.inner.lock().unwrap();
        if inner.playing == playing {
            return;
        }
        inner.playing = playing;
        let snapshot = self.snapshot_locked(&inner);
        drop(inner);
        self.emit(snapshot);
    }

    /// Called by the engine events thread when the current track ends. Advances
    /// the queue the same way a user "next" would, then loads. This is what lets
    /// auto-advance survive a dead webview.
    pub fn on_ended(&self) {
        let mut inner = self.inner.lock().unwrap();
        if inner.queue.is_empty() {
            return;
        }
        if inner.repeat == RepeatMode::One {
            let snapshot = self.snapshot_locked(&inner);
            drop(inner);
            // Restart the same track.
            self.bump_nonce();
            self.emit(snapshot);
            return;
        }
        let mut next_index = inner.index + 1;
        if next_index >= inner.queue.len() as i64 {
            if inner.repeat != RepeatMode::All {
                inner.playing = false;
                let snapshot = self.snapshot_locked(&inner);
                drop(inner);
                self.emit(snapshot);
                return;
            }
            if inner.shuffle && inner.queue.len() > 1 {
                let current = inner.queue[inner.index as usize].clone();
                let mut reshuffled = Self::shuffled(inner.queue.clone());
                if reshuffled[0] == current && reshuffled.len() > 1 {
                    reshuffled.swap(0, 1);
                }
                inner.queue = reshuffled;
            }
            next_index = 0;
        }
        inner.index = next_index;
        let snapshot = self.snapshot_locked(&inner);
        drop(inner);
        self.apply_current(true);
        self.emit(snapshot);
    }

    /// Load the current item into the engine, matching the play intent.
    fn apply_current(&self, follow_intent: bool) {
        let inner = self.inner.lock().unwrap();
        let item = inner
            .index
            .try_into()
            .ok()
            .filter(|&i: &usize| i < inner.queue.len())
            .map(|i| inner.queue[i].clone());
        let autoplay = if follow_intent { inner.playing } else { false };
        drop(inner);
        match item {
            Some(item) => {
                let _ = self.engine.send(Command::Load {
                    source_id: item.source_id,
                    path: item.path,
                    autoplay,
                });
            }
            None => {
                let _ = self.engine.send(Command::Stop);
            }
        }
    }

    /// Whether the engine currently holds the item we consider "current".
    fn current_loaded(&self) -> bool {
        let inner = self.inner.lock().unwrap();
        let item = inner
            .index
            .try_into()
            .ok()
            .filter(|&i: &usize| i < inner.queue.len())
            .map(|i| inner.queue[i].clone());
        drop(inner);
        match item {
            Some(item) => self.engine.shared().is_loaded(&item.source_id, &item.path),
            None => false,
        }
    }

    fn bump_nonce(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.nonce += 1;
        let nonce = inner.nonce;
        drop(inner);
        // A same-track restart: reload with the current play intent. The engine
        // treats a fresh Load as a full restart of the decoder.
        self.apply_current(true);
        // Surface the nonce so the webview can mirror it if it cares; we do not
        // depend on it for engine behaviour any more (apply_current reloads).
        let _ = nonce;
    }

    fn snapshot_locked(&self, inner: &Inner) -> ContextSnapshot {
        let current = inner
            .index
            .try_into()
            .ok()
            .filter(|&i: &usize| i < inner.queue.len())
            .map(|i| inner.queue[i].clone());
        ContextSnapshot {
            queue: inner
                .queue
                .iter()
                .map(|item| format!("{}{}{}", item.source_id, KEY_SEPARATOR, item.path))
                .collect(),
            queue_index: inner.index,
            shuffle: inner.shuffle,
            repeat: inner.repeat,
            has_track: current.is_some(),
            current_source_id: current.as_ref().map(|item| item.source_id.clone()),
            current_path: current.as_ref().map(|item| item.path.clone()),
            is_playing: inner.playing,
        }
    }

    /// Build a snapshot without holding the lock (used by the command layer).
    pub fn snapshot(&self) -> ContextSnapshot {
        let inner = self.inner.lock().unwrap();
        self.snapshot_locked(&inner)
    }

    fn shuffled<T: Clone>(mut items: Vec<T>) -> Vec<T> {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        // Deterministic-ish shuffle using a simple Fisher-Yates with a
        // time-seeded RNG; exact reproducibility is not required here.
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let mut hasher = DefaultHasher::new();
        seed.hash(&mut hasher);
        let mut rng = hasher.finish();
        let len = items.len();
        for i in (1..len).rev() {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let j = (rng >> 33) as usize % (i + 1);
            items.swap(i, j);
        }
        items
    }

    fn emit(&self, snapshot: ContextSnapshot) {
        if let Some(app) = self.engine.shared().app_handle() {
            let _ = app.emit("playback:changed", snapshot);
        }
    }
}

/// Poll the native Android MediaSession action queue and drive the context.
///
/// On Android the webview cannot be relied upon to poll (it is throttled or
/// killed in the background), so the Rust event thread does it instead. It is a
/// no-op on non-Android targets (the native bridge is not compiled in).
#[cfg(target_os = "android")]
pub fn drain_media_control(app: &AppHandle) {
    let action = match crate::platform::android::media_control_take_control() {
        Ok(Some(action)) => action,
        Ok(None) => return,
        Err(_) => return,
    };
    let context = match app.try_state::<PlaybackContext>() {
        Some(context) => context,
        None => return,
    };
    match action.as_str() {
        "play" => context.play(),
        "pause" => context.pause(),
        "toggle" => context.toggle(),
        "next" => context.next(),
        "previous" => context.previous(),
        "stop" => context.pause(),
        _ => {}
    }
}

/// Begin a lightweight timer that drains the Android MediaSession queue.
///
/// Spawned once at startup; only does work on Android. Lives for the process.
#[cfg(target_os = "android")]
pub fn start_media_control_poller(app: AppHandle) {
    std::thread::Builder::new()
        .name("welkin-media-control".to_string())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_millis(250));
            drain_media_control(&app);
        })
        .unwrap_or_else(|error| log::error!("failed to start media-control poller: {error}"));
}

/// On non-Android targets there is nothing to poll; the OS media controls feed
/// the webview (desktop) through souvlaki events, which the webview forwards to
/// the Rust context commands. Provide a no-op so callers need no cfg.
#[cfg(not(target_os = "android"))]
pub fn start_media_control_poller(_app: AppHandle) {}
