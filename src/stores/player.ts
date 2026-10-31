/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

import { computed, ref, watch } from "vue";
import { defineStore } from "pinia";
import { invoke } from "../api";
import { trackKey, splitTrackKey } from "../lib/sources";

/**
 * Player store — a *mirror* of the Rust playback context.
 *
 * The queue, shuffle/repeat, track selection and every next/previous/play
 * decision now live in Rust (`playback::context::PlaybackContext`). This store
 * keeps only the UI-facing state and forwards intent through the `playback_*`
 * Tauri commands. Rust pushes the authoritative snapshot back on the
 * `playback:changed` event, which {@link applySnapshot} mirrors here. This is
 * what lets playback (and the Android MediaSession) keep working when the
 * webview is backgrounded or killed: the store holds no decision logic.
 */

const VOLUME_STORAGE_KEY = "welkin-volume";
const MUTED_STORAGE_KEY = "welkin-muted";
const DEFAULT_VOLUME = 100;
const RATE_STORAGE_KEY = "welkin-playback-rate";
const DEFAULT_RATE = 1;
/** Bounds offered by the UI; the engine clamps to its own stretcher range. */
export const MIN_RATE = 0.5;
export const MAX_RATE = 2;
const SHUFFLE_STORAGE_KEY = "welkin-shuffle";
const REPEAT_STORAGE_KEY = "welkin-repeat";
export type RepeatMode = "off" | "all" | "one";

/** Snapshot pushed by Rust on every context change. */
export interface PlaybackSnapshot {
  /** Track keys (`sourceId::path`) of the active, possibly-shuffled queue. */
  queue: string[];
  /** Index of the current item within `queue`, or -1 when nothing is queued. */
  queueIndex: number;
  shuffle: boolean;
  repeat: RepeatMode;
  hasTrack: boolean;
  currentSourceId: string | null;
  currentPath: string | null;
  isPlaying: boolean;
}

/** Restore the locally saved output volume, falling back to the default. */
function readStoredVolume(): number {
  const raw = localStorage.getItem(VOLUME_STORAGE_KEY);
  // A missing key reads back as `null`; Number(null) is 0, which would wrongly
  // present the first launch as muted, so check for absence explicitly.
  if (raw === null) return DEFAULT_VOLUME;
  const stored = Number(raw);
  return Number.isFinite(stored) && stored >= 0 && stored <= 100 ? stored : DEFAULT_VOLUME;
}

/** Restore the locally saved playback rate, falling back to normal speed. */
function readStoredRate(): number {
  const raw = localStorage.getItem(RATE_STORAGE_KEY);
  if (raw === null) return DEFAULT_RATE;
  const stored = Number(raw);
  return Number.isFinite(stored) && stored >= MIN_RATE && stored <= MAX_RATE ? stored : DEFAULT_RATE;
}

export interface Track {
  id: number;
  title: string;
  artist: string;
  album: string;
  duration: string;
  color: string;
  /** Path inside its source, used to fetch metadata, covers and the stream. */
  path?: string;
  /** Id of the source this track was read from. */
  sourceId?: string;
  /** Display name of the source (kept denormalized for the UI). */
  sourceName?: string;
  /** Source last-modified timestamp, used to bust the cover image cache. */
  modified?: string | null;
  /** Cover thumbnail URL (asset protocol), once resolved. */
  cover?: string;
  /** Hex cache id of the cover thumbnail, for native media controls. */
  coverHash?: string;
  /** Whether metadata has already been loaded from cache or the network. */
  metaLoaded?: boolean;
  /** Whether a metadata fetch was attempted and returned nothing (no retry UI). */
  metaFailed?: boolean;
  /** Whether the on-disk cache has already been consulted for this track. */
  assetsHydrated?: boolean;
  /**
   * True for a synced profile entry this device cannot resolve to a real file
   * (e.g. a playlist created on another device). Such rows are display-only:
   * they carry no `path`/`sourceId`, so they are never queued or streamed.
   */
  unavailable?: boolean;
  /** Original profile key (`sourceProfileId::path`) of a placeholder track. */
  profileKey?: string;
}

function formatClock(seconds: number): string {
  const total = Number.isFinite(seconds) && seconds > 0 ? Math.floor(seconds) : 0;
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}

/** Build a minimal placeholder track for a key the library has not loaded yet. */
function placeholderFromKey(key: string): Track | null {
  const split = splitTrackKey(key);
  if (!split.path) return null;
  const name = split.path.split("/").pop() ?? split.path;
  return {
    id: -1,
    title: name,
    artist: "",
    album: "",
    duration: "",
    color: "",
    sourceId: split.sourceId,
    path: split.path,
  };
}

export const usePlayerStore = defineStore("player", () => {
  const tracks = ref<Track[]>([]);
  /** The active, possibly-shuffled queue, mirrored from Rust. */
  const queue = ref<Track[]>([]);
  /** Kept in sync with `queue` for API compatibility; Rust owns the real order. */
  const baseQueue = ref<Track[]>([]);
  const queueIndex = ref(-1);
  const shuffle = ref(localStorage.getItem(SHUFFLE_STORAGE_KEY) === "1");
  const repeat = ref<RepeatMode>(localStorage.getItem(REPEAT_STORAGE_KEY) === "all" ? "all" : localStorage.getItem(REPEAT_STORAGE_KEY) === "one" ? "one" : "off");

  const currentTrack = ref<Track | null>(null);
  const isPlaying = ref(false);
  /** Playback position as a percentage (drives the progress sliders). */
  const progress = ref(0);
  /** End of the buffered range containing the current position, as a percentage. */
  const bufferedProgress = ref(0);
  /** Playback position in seconds. */
  const position = ref(0);
  /** Track length in seconds, once the media element reports it. */
  const duration = ref(0);
  const volume = ref(readStoredVolume());
  const muted = ref(localStorage.getItem(MUTED_STORAGE_KEY) === "1");
  const playbackRate = ref(readStoredRate());
  /** Native sample rate of the loaded file (Hz), 0 when idle. */
  const sampleRate = ref(0);
  /** Rate the engine actually processes at: min(file, device). */
  const workRate = ref(0);
  /** Rate the output stream is currently opened at (the quality picker's target). */
  const outputRate = ref(0);

  /** Last snapshot from Rust, re-applied when `tracks` resolves new metadata. */
  let lastSnapshot: PlaybackSnapshot | null = null;

  // Persist the output volume/mute locally so each launch starts where the
  // previous one left off (mirrors the theme/accent localStorage pattern).
  // The watcher also clamps, so no code path can ever store a value outside 0-100.
  watch(volume, (value) => {
    const clamped = Number.isFinite(value) ? Math.min(100, Math.max(0, value)) : DEFAULT_VOLUME;
    if (clamped !== value) {
      volume.value = clamped;
      return;
    }
    localStorage.setItem(VOLUME_STORAGE_KEY, String(clamped));
  });
  watch(muted, (value) => localStorage.setItem(MUTED_STORAGE_KEY, value ? "1" : "0"));
  watch(playbackRate, (value) => {
    const clamped = Number.isFinite(value) ? Math.min(MAX_RATE, Math.max(MIN_RATE, value)) : DEFAULT_RATE;
    if (clamped !== value) {
      playbackRate.value = clamped;
      return;
    }
    localStorage.setItem(RATE_STORAGE_KEY, String(clamped));
  });
  watch(shuffle, (value) => localStorage.setItem(SHUFFLE_STORAGE_KEY, value ? "1" : "0"));
  watch(repeat, (value) => localStorage.setItem(REPEAT_STORAGE_KEY, value));

  // Re-resolve a queued/playing track once the library loads the real metadata
  // for a key that previously only had a placeholder.
  watch(
    () => tracks.value.length,
    () => {
      if (lastSnapshot) applySnapshot(lastSnapshot);
    },
  );

  const currentIndex = computed(() => {
    if (queue.value.length === 0) return currentTrack.value ? 1 : 0;
    return queueIndex.value >= 0 ? queueIndex.value + 1 : (currentTrack.value ? tracks.value.findIndex((track) => trackKey(track) === trackKey(currentTrack.value)) + 1 : 0);
  });
  const hasTrack = computed(() => currentTrack.value !== null);
  const elapsedTime = computed(() => formatClock(position.value));
  const totalTime = computed(() =>
    duration.value > 0 ? formatClock(duration.value) : currentTrack.value?.duration ?? "--:--",
  );
  const remainingTime = computed(() =>
    duration.value > 0 ? formatClock(Math.max(0, duration.value - position.value)) : totalTime.value,
  );

  /**
   * Mirror a Rust `playback:changed` snapshot into the store. Track *metadata*
   * is resolved from the local library when present, so the UI can show titles
   * and covers; unknown keys fall back to a placeholder.
   */
  function applySnapshot(snapshot: PlaybackSnapshot) {
    lastSnapshot = snapshot;
    if (snapshot.shuffle !== shuffle.value) shuffle.value = snapshot.shuffle;
    if (snapshot.repeat !== repeat.value) repeat.value = snapshot.repeat;
    queueIndex.value = snapshot.queueIndex;
    queue.value = snapshot.queue
      .map((key) => tracks.value.find((track) => trackKey(track) === key) ?? placeholderFromKey(key))
      .filter((track): track is Track => track !== null);
    baseQueue.value = queue.value;
    isPlaying.value = snapshot.isPlaying;
    if (snapshot.currentSourceId && snapshot.currentPath) {
      const key = `${snapshot.currentSourceId}::${snapshot.currentPath}`;
      currentTrack.value =
        tracks.value.find((track) => trackKey(track) === key) ?? placeholderFromKey(key);
    } else {
      currentTrack.value = null;
    }
  }

  function resolveIndex(track: Track): number {
    const key = trackKey(track);
    if (!key) return -1;
    return queue.value.findIndex((item) => trackKey(item) === key);
  }

  /** Push the whole library listing (used by UI that lists tracks to play). */
  function setTracks(nextTracks: Track[]) {
    tracks.value = [...nextTracks];
  }

  /** Serialize the play queue as stable track keys, for persistence. */
  function queueKeys(): string[] {
    return queue.value
      .map((track) => trackKey(track))
      .filter((key): key is string => !!key);
  }

  /** Start a track while preserving the exact context it was chosen from. */
  async function playInQueue(items: Track[], track: Track) {
    if (track.unavailable) return;
    const valid = items.filter((item) => item.path);
    if (valid.length === 0) return;
    const keys = valid.map((item) => trackKey(item)).filter((key): key is string => !!key);
    const startKey = trackKey(track) ?? undefined;
    try {
      await invoke("playback_set_queue", { keys, startKey });
    } catch (error) {
      console.warn("[welkin] playback_set_queue failed", error);
    }
  }

  /** Select a track by its identity, finding its index in the current queue. */
  async function selectTrack(track: Track) {
    if (track.unavailable) return;
    const index = resolveIndex(track);
    if (index < 0) return;
    try {
      await invoke("playback_select", { index });
    } catch (error) {
      console.warn("[welkin] playback_select failed", error);
    }
  }

  async function toggleShuffle() {
    if (repeat.value === "one") {
      // The Rust context mirrors the frontend's cycle semantics: leaving
      // repeat-one also clears shuffle. Request the cycle first; if the result is
      // still repeat-one the UI shows it as off+shuffle as appropriate.
    }
    try {
      await invoke("playback_toggle_shuffle");
    } catch (error) {
      console.warn("[welkin] playback_toggle_shuffle failed", error);
    }
  }

  async function cycleRepeat() {
    try {
      await invoke("playback_cycle_repeat");
    } catch (error) {
      console.warn("[welkin] playback_cycle_repeat failed", error);
    }
  }

  async function setShuffle(on: boolean) {
    try {
      await invoke("playback_set_shuffle", { on });
    } catch (error) {
      console.warn("[welkin] playback_set_shuffle failed", error);
    }
  }

  /** Advance to the next track (Rust honours repeat-one / repeat-all / end). */
  async function next() {
    try {
      await invoke("playback_next");
    } catch (error) {
      console.warn("[welkin] playback_next failed", error);
    }
  }

  /** Go to the previous track. */
  async function previous() {
    try {
      await invoke("playback_previous");
    } catch (error) {
      console.warn("[welkin] playback_previous failed", error);
    }
  }

  async function togglePlayback() {
    if (!currentTrack.value) return;
    try {
      await invoke("playback_toggle");
    } catch (error) {
      console.warn("[welkin] playback_toggle failed", error);
    }
  }

  async function setPlaying(value: boolean) {
    try {
      await invoke(value ? "playback_play" : "playback_pause");
    } catch (error) {
      console.warn("[welkin] playback play/pause failed", error);
    }
  }

  /** Restore a persisted queue + position into the Rust context. */
  async function restoreQueue(keys: string[], index: number, currentKey?: string): Promise<boolean> {
    if (keys.length === 0) return false;
    try {
      const ok = await invoke<boolean>("playback_restore", {
        keys,
        index,
        currentKey: currentKey ?? null,
      });
      return ok;
    } catch (error) {
      console.warn("[welkin] playback_restore failed", error);
      return false;
    }
  }

  // --- UI-facing transport state set by the audio layer (position/duration) ---

  function setPosition(seconds: number) {
    position.value = Number.isFinite(seconds) && seconds > 0 ? seconds : 0;
    progress.value = duration.value > 0 ? Math.min(100, (position.value / duration.value) * 100) : 0;
  }

  function setDuration(seconds: number) {
    duration.value = Number.isFinite(seconds) && seconds > 0 ? seconds : 0;
    setPosition(position.value);
  }

  function setBufferedProgress(value: number) {
    bufferedProgress.value = Number.isFinite(value) ? Math.min(100, Math.max(0, value)) : 0;
  }

  function setRates(fileRate: number, workRate_: number, outputRate_ = 0) {
    sampleRate.value = Number.isFinite(fileRate) && fileRate > 0 ? Math.round(fileRate) : 0;
    workRate.value = Number.isFinite(workRate_) && workRate_ > 0 ? Math.round(workRate_) : 0;
    if (Number.isFinite(outputRate_) && outputRate_ > 0) outputRate.value = Math.round(outputRate_);
  }

  function setVolume(value: number) {
    volume.value = Number.isFinite(value) ? Math.min(100, Math.max(0, value)) : volume.value;
  }

  function setPlaybackRate(value: number) {
    playbackRate.value = Number.isFinite(value) ? Math.min(MAX_RATE, Math.max(MIN_RATE, value)) : playbackRate.value;
  }

  function toggleMute() {
    muted.value = !muted.value;
  }

  /** Merge a partial update into one track (metadata / cover arrive later). */
  function updateTrack(id: number, patch: Partial<Track>) {
    const track = tracks.value.find((item) => item.id === id);
    if (!track) return;
    Object.assign(track, patch);
    if (currentTrack.value && currentTrack.value.id === id) {
      currentTrack.value = { ...currentTrack.value, ...patch };
    }
  }

  /** Merge asynchronous metadata by stable source/path identity. */
  function updateTrackByKey(key: string, patch: Partial<Track>) {
    const track = tracks.value.find((item) => trackKey(item) === key);
    if (track) Object.assign(track, patch);
    if (currentTrack.value && trackKey(currentTrack.value) === key) {
      currentTrack.value = { ...currentTrack.value, ...patch };
    }
    for (const item of queue.value) {
      if (trackKey(item) === key) Object.assign(item, patch);
    }
  }

  /** Merge many partial updates in a single pass. */
  function updateTracks(patches: Array<{ id: number; patch: Partial<Track> }>) {
    if (patches.length === 0) return;
    const byId = new Map(tracks.value.map((track) => [track.id, track]));
    for (const { id, patch } of patches) {
      const track = byId.get(id);
      if (track) Object.assign(track, patch);
      if (currentTrack.value && currentTrack.value.id === id) {
        currentTrack.value = { ...currentTrack.value, ...patch };
      }
    }
  }

  return {
    tracks,
    queue,
    baseQueue,
    queueIndex,
    shuffle,
    repeat,
    currentTrack,
    isPlaying,
    progress,
    bufferedProgress,
    position,
    duration,
    volume,
    muted,
    playbackRate,
    sampleRate,
    workRate,
    outputRate,
    currentIndex,
    hasTrack,
    elapsedTime,
    totalTime,
    remainingTime,
    applySnapshot,
    setTracks,
    queueKeys,
    restoreQueue,
    updateTrack,
    updateTrackByKey,
    updateTracks,
    togglePlayback,
    setPlaying,
    setPosition,
    setDuration,
    setBufferedProgress,
    setRates,
    setVolume,
    setPlaybackRate,
    toggleMute,
    next,
    previous,
    selectTrack,
    playInQueue,
    toggleShuffle,
    cycleRepeat,
    setShuffle,
  };
});
