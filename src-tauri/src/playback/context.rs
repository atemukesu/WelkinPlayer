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
// `Duration` is only needed by the Android media-control poller below.
#[cfg(target_os = "android")]
use std::time::Duration;
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::playback::engine::{AudioState, Command, CrossfadeState, Engine};

/// Separator between a source id and a track path inside a track key.
pub const KEY_SEPARATOR: &str = "::";

/// A track the context can hand straight to the engine: its source plus the
/// in-source path. This is the minimal identity `audio_load` needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueItem {
    pub source_id: String,
    pub path: String,
}

/// One transition handed to the context for scheduling.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackTransition {
    /// `sourceId::path` of the outgoing track.
    pub from: String,
    /// `sourceId::path` of the incoming track.
    pub to: String,
    /// Absolute seconds in the outgoing track where the fade starts.
    pub out_at: f64,
    /// Absolute seconds in the incoming track to drop the playhead at, so the
    /// new track enters on its musical Mix In point.
    #[serde(default)]
    pub in_at: f64,
    /// Fade length in seconds.
    pub blend_secs: f64,
}

/// How long before the Mix Out point the incoming deck is opened and prefilled.
const CROSSFADE_PREPARE_LEAD_SECS: f64 = 8.0;
/// Plans discovered later than this fall back to the sequential fade instead of
/// trying to open a remote stream at the last moment.
const CROSSFADE_MIN_LEAD_SECS: f64 = 0.4;

/// The transition envelope currently in flight.
enum FadePhase {
    Idle,
    /// The incoming deck is being opened/prefilled on the engine's helper
    /// thread; the outgoing track keeps playing untouched.
    Preparing {
        from: String,
        to: String,
        out_at: f64,
        in_at: f64,
        blend_secs: f64,
        token: u64,
        failed: bool,
    },
    /// Both decks are mixed by the engine's equal-power envelope.
    Crossfading {
        from: String,
        to: String,
        in_at: f64,
        blend_secs: f64,
        token: u64,
    },
    /// Fading the outgoing track down to silence.
    Out {
        from: String,
        to: String,
        started: Instant,
        duration: f64,
        /// Mix In point of the incoming track, applied after the advance.
        in_at: f64,
    },
    /// Fading the incoming track up from silence. `started` is `None` while the
    /// queue is still waiting for the outgoing track to end and advance.
    In {
        from: String,
        to: String,
        started: Option<Instant>,
        duration: f64,
    },
}

impl Default for FadePhase {
    fn default() -> Self {
        FadePhase::Idle
    }
}

#[derive(Default)]
struct TransitionRuntime {
    plans: Vec<PlaybackTransition>,
    phase: FadePhase,
    /// Monotonic token so a late preparation result cannot start a stale fade.
    crossfade_token: u64,
}

fn track_key(item: &QueueItem) -> String {
    format!("{}{}{}", item.source_id, KEY_SEPARATOR, item.path)
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
    /// When true the queue is pinned to the analysed smart-transition order:
    /// shuffle is ignored and the order never reshuffles.
    pub order_locked: bool,
}

/// The mutable playback context, guarded by a single mutex. All intent commands
/// take the lock, mutate, then emit. The engine is driven synchronously so a
/// `next` that lands a playable track always loads it before the caller returns.
pub struct PlaybackContext {
    engine: Engine,
    /// Used to resolve sources and the manual cache when filtering a
    /// cached-only queue; the context may outlive the webview that set it.
    app: AppHandle,
    inner: Mutex<Inner>,
    /// Smart-transition plans and the fade envelope currently in flight.
    transition: Mutex<TransitionRuntime>,
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
    /// When true the queue was narrowed to cached/local tracks because the
    /// user declined cloud playback on a metered connection.
    cached_only: bool,
    /// Bumped when the same item is reselected, to force an engine restart.
    nonce: u64,
    /// Pinned to the analysed order while smart transitions are enabled.
    order_locked: bool,
}

impl PlaybackContext {
    pub fn new(engine: Engine, app: AppHandle) -> Self {
        Self {
            engine,
            app,
            inner: Mutex::new(Inner::default()),
            transition: Mutex::new(TransitionRuntime::default()),
        }
    }

    /// Whether a track can play without fetching bytes over the network: it is
    /// either in a local source or fully downloaded into the manual cache.
    /// Local source ids and the manual cache are resolved once per queue so a
    /// large library does not re-read the settings store per track.
    fn cached_items(&self, items: Vec<QueueItem>) -> Vec<QueueItem> {
        let local: std::collections::HashSet<String> =
            crate::data::sources::load_sources(&self.app)
                .into_iter()
                .filter(|source| source.is_local())
                .map(|source| source.id)
                .collect();
        let manual = self.app.try_state::<crate::streaming::state::StreamState>();
        items
            .into_iter()
            .filter(|item| {
                local.contains(&item.source_id)
                    || manual
                        .as_ref()
                        .map(|state| {
                            state
                                .manual
                                .is_complete(&crate::commands::media::asset_hash(
                                    &item.source_id,
                                    &item.path,
                                ))
                        })
                        .unwrap_or(false)
            })
            .collect()
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
    ///
    /// `cached_only` narrows the request to tracks that can play without the
    /// network (local files or completed manual cache entries). When
    /// `lock_order` is set the queue is the analysed smart-transition order and
    /// shuffle is ignored for its whole lifetime.
    pub fn set_queue(
        &self,
        keys: Vec<String>,
        start_key: Option<&str>,
        cached_only: bool,
        lock_order: bool,
    ) {
        // A new queue invalidates the previous playlist's plans; the caller arms
        // the new ones after the queue is installed.
        self.clear_transition_plans();
        let mut items = Self::items_from_keys(&keys);
        if cached_only {
            items = self.cached_items(items);
        }
        let mut inner = self.inner.lock().unwrap();
        inner.cached_only = cached_only;
        inner.order_locked = lock_order;
        inner.base_queue = items.clone();
        let (queue, index) = if inner.shuffle && !lock_order && !items.is_empty() {
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
        // Starting a queue is an explicit play request from the user, so load and
        // begin playback immediately. A cached-only queue that filtered down to
        // nothing must not start the engine.
        inner.playing = !inner.queue.is_empty();
        let snapshot = self.snapshot_locked(&inner);
        drop(inner);
        self.apply_current(true);
        self.emit(snapshot);
    }

    /// Reselect the current queue item (same track) without changing order.
    pub fn select_index(&self, index: i64) {
        self.reset_transition();
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
        // Selecting a track (from the queue / a list) is an explicit play request.
        inner.playing = true;
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
        self.reset_transition();
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
        self.reset_transition();
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
        self.reset_transition();
        let mut inner = self.inner.lock().unwrap();
        inner.playing = false;
        let snapshot = self.snapshot_locked(&inner);
        drop(inner);
        let _ = self.engine.send(Command::Pause);
        self.emit(snapshot);
    }

    /// Seek to an absolute position (seconds). Drives the engine directly so the
    /// native MediaSession's scrub bar works without the webview.
    pub fn seek(&self, position_secs: f64) {
        self.reset_transition();
        if !position_secs.is_finite() || position_secs < 0.0 {
            return;
        }
        let _ = self.engine.send(Command::Seek {
            position_secs,
        });
    }

    /// Apply a transport action coming from an OS media surface (desktop souvlaki
    /// or the native Android MediaSession). Both platforms funnel through here so
    /// the control logic lives in exactly one place.
    pub fn apply_control(&self, action: &str, value: Option<f64>) {
        match action {
            "play" => self.play(),
            "pause" => self.pause(),
            "toggle" => self.toggle(),
            "next" => self.next(),
            "previous" => self.previous(),
            "stop" => self.pause(),
            "seek" => {
                if let Some(millis) = value {
                    self.seek(millis / 1000.0);
                }
            }
            "seekBy" => {
                let delta = value.unwrap_or(0.0);
                let current = self.engine.shared().position_ms() as f64 / 1000.0;
                self.seek((current + delta).max(0.0));
            }
            "volume" => {
                let v = value.unwrap_or(0.0).clamp(0.0, 1.0);
                self.engine.shared().set_volume_percent(v * 100.0);
            }
            _ => {}
        }
    }

    /// Push the current playback to the OS media controls. Called by the engine
    /// events thread so reporting no longer depends on the webview. Metadata is
    /// derived from `source_id::path` by the media-control module.
    pub fn apply_to_media_control(&self) {
        let app = match self.engine.shared().app_handle() {
            Some(app) => app,
            None => return,
        };
        let inner = self.inner.lock().unwrap();
        let item = inner
            .index
            .try_into()
            .ok()
            .filter(|&i: &usize| i < inner.queue.len())
            .map(|i| inner.queue[i].clone());
        let playing = inner.playing;
        drop(inner);
        let shared = self.engine.shared();
        let state = shared.snapshot();
        match item {
            Some(item) => crate::platform::media_control::push_update(
                &app,
                &item.source_id,
                &item.path,
                state.duration_ms,
                state.position_ms,
                state.playing && playing,
                // MediaControlPayload uses the frontend's 0..100 convention.
                shared.volume_percent() as f64,
            ),
            None => crate::platform::media_control::push_update(
                &app,
                "",
                "",
                0.0,
                0.0,
                false,
                0.0,
            ),
        }
    }

    /// Advance to the next track (honouring repeat-one / repeat-all / end).
    pub fn next(&self) {
        self.reset_transition();
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
            if inner.shuffle && !inner.order_locked && inner.queue.len() > 1 {
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
        self.reset_transition();
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
        // Smart transitions pin the played order, so shuffle cannot reorder it.
        if self.inner.lock().unwrap().order_locked {
            return;
        }
        self.reset_transition();
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
        self.reset_transition();
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
        if self.inner.lock().unwrap().order_locked {
            return;
        }
        self.reset_transition();
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
        self.reset_transition();
        let items = Self::items_from_keys(keys);
        if items.is_empty() {
            return false;
        }
        let mut inner = self.inner.lock().unwrap();
        inner.cached_only = false;
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

    /// Replace the scheduled smart-transition plans. Passing an empty list
    /// disables transitions. The envelope is reset so a stale fade never leaks
    /// into the next queue.
    pub fn set_transition_plans(&self, plans: Vec<PlaybackTransition>) {
        // Plans describe the analysed order, so possessing them pins the queue
        // to that order; clearing them releases the lock.
        let order_locked = !plans.is_empty();
        let changed = {
            let mut inner = self.inner.lock().unwrap();
            let changed = inner.order_locked != order_locked;
            inner.order_locked = order_locked;
            changed
        };
        if changed {
            // Surface the lock state so the UI can disable/enable shuffle.
            let snapshot = {
                let inner = self.inner.lock().unwrap();
                self.snapshot_locked(&inner)
            };
            self.emit(snapshot);
        }
        // Replacing the plans invalidates any prepared or live crossfade.
        self.reset_transition();
        let mut runtime = self.transition.lock().unwrap();
        runtime.plans = plans;
    }

    /// Clear all transition plans and restore unity gain.
    pub fn clear_transition_plans(&self) {
        self.set_transition_plans(Vec::new());
    }

    /// Reorder the current queue in place, keeping the playing track.
    ///
    /// Used when smart transitions are switched on mid-playback: the queue was
    /// possibly shuffled or sorted, and must become the analysed order without
    /// restarting the current track. Items that are not in `keys` (for example
    /// cloud tracks filtered out on a metered connection) keep their relative
    /// order at the end. The order is pinned so shuffle cannot undo it.
    pub fn reorder(&self, keys: &[String]) {
        let mut inner = self.inner.lock().unwrap();
        if inner.queue.is_empty() {
            return;
        }
        let current = inner
            .index
            .try_into()
            .ok()
            .filter(|&i: &usize| i < inner.queue.len())
            .map(|i| inner.queue[i].clone());
        let base = if inner.base_queue.is_empty() {
            inner.queue.clone()
        } else {
            inner.base_queue.clone()
        };
        let mut ordered: Vec<QueueItem> = Vec::with_capacity(base.len());
        for key in keys {
            let Some((source_id, path)) = Self::split_key(key) else {
                continue;
            };
            let item = QueueItem { source_id, path };
            if !ordered.contains(&item) && base.contains(&item) {
                ordered.push(item);
            }
        }
        for item in base {
            if !ordered.contains(&item) {
                ordered.push(item);
            }
        }
        if ordered.is_empty() {
            return;
        }
        inner.index = current
            .and_then(|current| ordered.iter().position(|item| item == &current))
            .map(|i| i as i64)
            .unwrap_or(0);
        inner.queue = ordered.clone();
        inner.base_queue = ordered;
        inner.order_locked = true;
        let snapshot = self.snapshot_locked(&inner);
        drop(inner);
        // Keep the currently playing track untouched; only the upcoming order
        // changes. The caller re-arms plans for the new adjacency afterwards.
        self.emit(snapshot);
    }

    fn reset_transition(&self) {
        let mut runtime = self.transition.lock().unwrap();
        runtime.phase = FadePhase::Idle;
        // Invalidate any in-flight preparation and stop a live crossfade; the
        // engine restores the audible deck to unity through its gain ramp.
        runtime.crossfade_token = runtime.crossfade_token.wrapping_add(1);
        drop(runtime);
        let _ = self.engine.send(Command::CancelCrossfade);
        self.engine.shared().set_transition_gain(1.0);
    }

    /// Drive the smart-transition envelope. Called from the engine events thread
    /// every 50 ms, so the fade stays smooth without another timer.
    pub fn on_tick(&self, state: &AudioState) {
        if !state.playing {
            return;
        }
        let (current, next) = {
            let inner = self.inner.lock().unwrap();
            let index: usize = match inner.index.try_into() {
                Ok(index) => index,
                Err(_) => return,
            };
            let current = inner.queue.get(index).map(track_key);
            let next = if inner.repeat == RepeatMode::One {
                None
            } else if index + 1 < inner.queue.len() {
                Some(track_key(&inner.queue[index + 1]))
            } else if inner.repeat == RepeatMode::All && !inner.queue.is_empty() {
                Some(track_key(&inner.queue[0]))
            } else {
                None
            };
            (current, next)
        };
        let position = state.position_ms / 1000.0;
        let track_duration = state.duration_ms / 1000.0;
        // When the fade completes early (musical Mix Out before the file's
        // final samples) we advance ourselves and then drop the next track at
        // its Mix In point. This is recorded here and applied after the lock is
        // released so queue loading never runs under the transition mutex.
        let mut advance_to: Option<f64> = None;

        let mut runtime = self.transition.lock().unwrap();
        let phase = std::mem::replace(&mut runtime.phase, FadePhase::Idle);
        let shared = self.engine.shared();
        let mut handover: Option<String> = None;
        runtime.phase = match phase {
            FadePhase::Idle => {
                let trigger = current.as_deref().and_then(|current| {
                    next.as_deref().and_then(|next| {
                        runtime
                            .plans
                            .iter()
                            .find(|plan| plan.from == current && plan.to == next)
                            .cloned()
                    })
                });
                match trigger {
                    Some(plan) => {
                        let start = plan.out_at.max(0.0);
                        let remaining = start - position;
                        let duration = if track_duration > 0.5 {
                            plan.blend_secs
                                .min((track_duration - position).max(0.0))
                                .clamp(0.35, 30.0)
                        } else {
                            plan.blend_secs.clamp(0.35, 30.0)
                        };
                        if position + 0.02 >= start {
                            // If the user seeks past the Mix Out point, shrink
                            // the fade to whatever time is actually left so it
                            // never runs past the file's end.
                            FadePhase::Out {
                                from: plan.from,
                                to: plan.to,
                                started: Instant::now(),
                                duration,
                                in_at: plan.in_at.max(0.0),
                            }
                        } else if remaining > CROSSFADE_MIN_LEAD_SECS {
                            // Request the second deck only once the lead window
                            // is reached; token 0 means "not requested yet".
                            let token = if remaining <= CROSSFADE_PREPARE_LEAD_SECS {
                                let token = runtime.crossfade_token.wrapping_add(1);
                                runtime.crossfade_token = token;
                                if let Some((source_id, path)) =
                                    Self::split_key(&plan.to)
                                {
                                    let _ = self.engine.send(Command::PrepareCrossfade {
                                        token,
                                        source_id,
                                        path,
                                        in_at: plan.in_at.max(0.0),
                                    });
                                }
                                token
                            } else {
                                0
                            };
                            FadePhase::Preparing {
                                from: plan.from,
                                to: plan.to,
                                out_at: start,
                                in_at: plan.in_at.max(0.0),
                                blend_secs: plan.blend_secs,
                                token,
                                failed: false,
                            }
                        } else {
                            // Too late to open a second deck safely.
                            FadePhase::Preparing {
                                from: plan.from,
                                to: plan.to,
                                out_at: start,
                                in_at: plan.in_at.max(0.0),
                                blend_secs: plan.blend_secs,
                                token: 0,
                                failed: true,
                            }
                        }
                    }
                    None => FadePhase::Idle,
                }
            }
            FadePhase::Preparing {
                from,
                to,
                out_at,
                in_at,
                blend_secs,
                token,
                mut failed,
            } => {
                if current.as_deref() != Some(from.as_str()) {
                    let _ = self.engine.send(Command::CancelCrossfade);
                    shared.set_transition_gain(1.0);
                    FadePhase::Idle
                } else {
                    let state = shared.crossfade_state();
                    if state == CrossfadeState::Failed {
                        shared.take_crossfade_state(CrossfadeState::Failed);
                        failed = true;
                    }
                    if position + 0.02 >= out_at {
                        if !failed && state == CrossfadeState::Ready {
                            let _ = self.engine.send(Command::StartCrossfade {
                                token,
                                blend_secs,
                            });
                            FadePhase::Crossfading {
                                from,
                                to,
                                in_at,
                                blend_secs,
                                token,
                            }
                        } else {
                            // No usable second deck: cancel it and run the old
                            // sequential fade from the current position.
                            let _ = self.engine.send(Command::CancelCrossfade);
                            shared.set_transition_gain(1.0);
                            let duration = if track_duration > 0.5 {
                                blend_secs
                                    .min((track_duration - position).max(0.0))
                                    .clamp(0.35, 30.0)
                            } else {
                                blend_secs.clamp(0.35, 30.0)
                            };
                            FadePhase::Out {
                                from,
                                to,
                                started: Instant::now(),
                                duration,
                                in_at,
                            }
                        }
                    } else if position > out_at + 2.0 {
                        let _ = self.engine.send(Command::CancelCrossfade);
                        shared.set_transition_gain(1.0);
                        FadePhase::Idle
                    } else if !failed && token == 0 {
                        let remaining = out_at - position;
                        if remaining > CROSSFADE_MIN_LEAD_SECS
                            && remaining <= CROSSFADE_PREPARE_LEAD_SECS
                        {
                            // Entered the lead window: request the second deck.
                            let token = runtime.crossfade_token.wrapping_add(1);
                            runtime.crossfade_token = token;
                            if let Some((source_id, path)) = Self::split_key(&to) {
                                let _ = self.engine.send(Command::PrepareCrossfade {
                                    token,
                                    source_id,
                                    path,
                                    in_at,
                                });
                            }
                            FadePhase::Preparing {
                                from,
                                to,
                                out_at,
                                in_at,
                                blend_secs,
                                token,
                                failed: false,
                            }
                        } else if remaining <= CROSSFADE_MIN_LEAD_SECS {
                            // Too late now: use the sequential fallback.
                            FadePhase::Preparing {
                                from,
                                to,
                                out_at,
                                in_at,
                                blend_secs,
                                token: 0,
                                failed: true,
                            }
                        } else {
                            FadePhase::Preparing {
                                from,
                                to,
                                out_at,
                                in_at,
                                blend_secs,
                                token: 0,
                                failed: false,
                            }
                        }
                    } else {
                        FadePhase::Preparing {
                            from,
                            to,
                            out_at,
                            in_at,
                            blend_secs,
                            token,
                            failed,
                        }
                    }
                }
            }
            FadePhase::Crossfading {
                from,
                to,
                in_at,
                blend_secs,
                token,
            } => {
                if current.as_deref() != Some(from.as_str()) {
                    let _ = self.engine.send(Command::CancelCrossfade);
                    shared.set_transition_gain(1.0);
                    FadePhase::Idle
                } else {
                    match shared.crossfade_state() {
                        CrossfadeState::Done => {
                            shared.take_crossfade_state(CrossfadeState::Done);
                            shared.set_transition_gain(1.0);
                            handover = Some(to.clone());
                            FadePhase::Idle
                        }
                        CrossfadeState::Failed | CrossfadeState::None => {
                            shared.take_crossfade_state(CrossfadeState::Failed);
                            // Drop the half-built second deck before falling back
                            // so it cannot keep consuming CPU or download budget.
                            let _ = self.engine.send(Command::CancelCrossfade);
                            shared.set_transition_gain(1.0);
                            // The second deck could not take over: finish with
                            // the sequential fade so the next track still
                            // enters at its Mix In point.
                            let duration = if track_duration > 0.5 {
                                blend_secs
                                    .min((track_duration - position).max(0.0))
                                    .clamp(0.35, 30.0)
                            } else {
                                blend_secs.clamp(0.35, 30.0)
                            };
                            FadePhase::Out {
                                from,
                                to,
                                started: Instant::now(),
                                duration,
                                in_at,
                            }
                        }
                        _ => FadePhase::Crossfading {
                            from,
                            to,
                            in_at,
                            blend_secs,
                            token,
                        },
                    }
                }
            }
            FadePhase::Out {
                from,
                to,
                started,
                duration,
                in_at,
            } => {
                if current.as_deref() == Some(to.as_str()) {
                    // The track ended and the queue advanced before the envelope
                    // finished ticking: start the new track silent so the fade-in
                    // still happens instead of unmuting abruptly.
                    shared.set_transition_gain(0.0);
                    FadePhase::In {
                        from,
                        to,
                        started: None,
                        duration: 1.2,
                    }
                } else if current.as_deref() != Some(from.as_str()) {
                    // The user moved to some other track: cancel the fade.
                    shared.set_transition_gain(1.0);
                    FadePhase::Idle
                } else {
                    let t = (started.elapsed().as_secs_f64() / duration).clamp(0.0, 1.0);
                    if t >= 1.0 {
                        // The outgoing track is now silent. Advance immediately
                        // instead of waiting for its natural end, so the next
                        // track enters on the musical Mix In point.
                        shared.set_transition_gain(0.0);
                        advance_to = Some(in_at);
                        FadePhase::In {
                            from,
                            to,
                            started: None,
                            duration: 1.2,
                        }
                    } else {
                        // Equal-power fade-out: cos(t * pi/2).
                        let gain = ((1.0 - t) * std::f64::consts::FRAC_PI_2).cos() as f32;
                        shared.set_transition_gain(gain);
                        FadePhase::Out {
                            from,
                            to,
                            started,
                            duration,
                            in_at,
                        }
                    }
                }
            }
            FadePhase::In {
                from,
                to,
                started,
                duration,
            } => {
                if current.as_deref() == Some(to.as_str()) {
                    // The queue has advanced: fade the new track up.
                    let started = started.unwrap_or_else(Instant::now);
                    let t = (started.elapsed().as_secs_f64() / duration).clamp(0.0, 1.0);
                    if t >= 1.0 {
                        shared.set_transition_gain(1.0);
                        FadePhase::Idle
                    } else {
                        // Equal-power fade-in: sin(t * pi/2).
                        let gain = (t * std::f64::consts::FRAC_PI_2).sin() as f32;
                        shared.set_transition_gain(gain);
                        FadePhase::In {
                            from,
                            to,
                            started: Some(started),
                            duration,
                        }
                    }
                } else if current.as_deref() == Some(from.as_str()) {
                    // Still on the outgoing track for a tick or two: stay silent
                    // until the natural end advances the queue, instead of
                    // jumping back to full volume.
                    shared.set_transition_gain(0.0);
                    FadePhase::In {
                        from,
                        to,
                        started,
                        duration,
                    }
                } else {
                    // The user moved to some other track: restore unity gain.
                    shared.set_transition_gain(1.0);
                    FadePhase::Idle
                }
            }
        };
        drop(runtime);

        if let Some(to) = handover {
            // The engine already switched to the incoming deck; only the queue
            // index needs to follow so the UI and the next plan agree.
            self.on_handover(&to);
        }

        if let Some(in_at) = advance_to {
            // Safe to load: the outgoing track is inaudible (gain 0). Loading
            // and then seeking happen on the engine worker in command order,
            // so the seek lands on the freshly loaded incoming track.
            self.on_ended();
            if in_at > 0.05 {
                let _ = self.engine.send(Command::Seek {
                    position_secs: in_at,
                });
            }
        }
    }

    /// The overlapping crossfade promoted its incoming deck to the audible one.
    /// Advance the queue index to that track *without* reloading the engine,
    /// because the incoming deck is already playing at the right position.
    fn on_handover(&self, to: &str) {
        let mut inner = self.inner.lock().unwrap();
        if inner.queue.is_empty() {
            return;
        }
        let len = inner.queue.len() as i64;
        let current = inner.index.rem_euclid(len);
        let mut found = None;
        for offset in 1..=len {
            let index = (current + offset).rem_euclid(len);
            if track_key(&inner.queue[index as usize]) == to {
                found = Some(index);
                break;
            }
        }
        let Some(index) = found else {
            return;
        };
        inner.index = index;
        inner.playing = true;
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
            if inner.shuffle && !inner.order_locked && inner.queue.len() > 1 {
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
            order_locked: inner.order_locked,
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
    let raw = match crate::platform::android::media_control_take_control() {
        Ok(Some(raw)) => raw,
        _ => return,
    };
    // The Android bridge parks a JSON command (`{"action":"play","value":...}`),
    // the same shape the desktop souvlaki path emits. The webview used to
    // `JSON.parse` it before matching on `action`; do the same here so a
    // command is not silently dropped by a literal string comparison.
    let parsed = serde_json::from_str::<serde_json::Value>(&raw).ok();
    let action = parsed
        .as_ref()
        .and_then(|value| value.get("action").and_then(|a| a.as_str()).map(|s| s.to_string()))
        .unwrap_or_else(|| raw.clone());
    let value = parsed
        .as_ref()
        .and_then(|value| value.get("value").and_then(|v| v.as_f64()));
    let context = match app.try_state::<PlaybackContext>() {
        Some(context) => context,
        None => return,
    };
    context.apply_control(&action, value);
}

/// Begin a lightweight timer that drains the Android MediaSession queue.
///
/// Spawned once at startup; only does work on Android. Lives for the process.
#[cfg(target_os = "android")]
pub fn start_media_control_poller(app: AppHandle) {
    if let Err(error) = std::thread::Builder::new()
        .name("welkin-media-control".to_string())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_millis(250));
            drain_media_control(&app);
        })
    {
        log::error!("failed to start media-control poller: {error}");
    }
}

/// On non-Android targets there is nothing to poll; the OS media controls feed
/// the webview (desktop) through souvlaki events, which the webview forwards to
/// the Rust context commands. Provide a no-op so callers need no cfg.
#[cfg(not(target_os = "android"))]
pub fn start_media_control_poller(_app: AppHandle) {}
