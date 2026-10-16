/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

import { effectScope, shallowRef, watch } from "vue";
import type { EffectScope } from "vue";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { invoke, toAppError } from "../api";
import { i18n } from "../i18n";
import { usePlayerStore } from "../stores/player";
import { useNetworkStore } from "../stores/network";
import { useCacheStore } from "../stores/cache";
import { pushToast } from "./toast";
import { trackKey } from "./sources";

/**
 * Playback client.
 *
 * Decoding and output live in the Rust engine (desktop + Android); this module
 * only issues transport commands (`audio_load` / `audio_play` / `audio_pause` /
 * `audio_seek` / `audio_set_volume` / `audio_set_mute` / `audio_stop`) and
 * mirrors the `audio:state`, `audio:ended` and `audio:error` events back into
 * the player store. The webview media element is no longer used.
 */

/** Localization keys for the `AppError` codes the engine can surface. */
const ERROR_KEYS: Record<string, string> = {
  UNAUTHORIZED: "errors.unauthorized",
  FORBIDDEN: "errors.forbidden",
  NOT_FOUND: "errors.notFound",
  TIMEOUT: "errors.timeout",
  DNS: "errors.dns",
  TLS: "errors.tls",
  CONNECTION: "errors.connection",
  HTTP: "errors.http",
  MISSING_CREDENTIALS: "errors.missingCredentials",
  INSECURE_URL: "errors.insecureUrl",
  STORE: "errors.store",
  INVALID_ARGUMENT: "errors.invalidArgument",
  UNSUPPORTED: "playback.unsupported",
  AUDIO_DEVICE: "playback.failed",
  SEEK_UNSUPPORTED: "playback.seekUnsupported",
};

/** Prefetch the next queue entry once this many seconds remain. */
const NEXT_PREFETCH_LEAD_SECS = 60;
/** How much of the next track to preload (seconds), when on Wi-Fi. */
const NEXT_PREFETCH_SECONDS = 15;

interface AudioStatePayload {
  playing: boolean;
  positionMs: number;
  durationMs: number;
  ended: boolean;
}

interface AudioErrorPayload {
  code: string;
  message: string;
}

export interface SpectrumData {
  sampleRate: number;
  magnitudes: number[];
}

/** FFT frequency magnitudes published by the native output callback. */
export const spectrumData = shallowRef<SpectrumData | null>(null);

let scope: EffectScope | null = null;
let initPromise: Promise<void> | null = null;
let unlisteners: UnlistenFn[] = [];
let bufferedTimer = 0;
/** Last reported playhead, extrapolated between events for smooth lyrics. */
let lastPositionMs = 0;
let lastPositionAt = 0;
let enginePlaying = false;
/** Guards the speculative next-track prefetch (one attempt per next track). */
let prefetchedKey: string | null = null;
/** Throttle cache-position IPC while keeping the downloader near the playhead. */
let lastStreamReportAt = 0;
let lastStreamReportKey: string | null = null;

function t(key: string): string {
  return i18n.global.t(key);
}

/** Playback diagnostics: webview console + Rust stderr (see `log_message`). */
function playbackLog(message: string, details?: unknown): void {
  const suffix = details === undefined ? "" : ` ${typeof details === "string" ? details : JSON.stringify(details)}`;
  const line = `[playback] ${message}${suffix}`;
  console.log(line);
  void invoke("log_message", { level: "info", message: line }).catch(() => {});
}

function onState(payload: AudioStatePayload) {
  const player = usePlayerStore();
  const duration = payload.durationMs / 1000;
  const position = Math.max(0, duration > 0 ? Math.min(payload.positionMs / 1000, duration) : payload.positionMs / 1000);
  lastPositionMs = position * 1000;
  lastPositionAt = performance.now();
  enginePlaying = payload.playing;
  player.setPosition(position);
  if (duration > 0) player.setDuration(duration);
  reportStreamProgress(position, duration);
  maybePrefetchNext(position, duration);
}

function onSpectrum(payload: SpectrumData) {
  if (!payload || !Array.isArray(payload.magnitudes)) return;
  spectrumData.value = {
    sampleRate: Number(payload.sampleRate) || 44100,
    magnitudes: payload.magnitudes.map((sample) => Math.max(0, Math.min(1, Number(sample) || 0))),
  };
}

function reportStreamProgress(position: number, duration: number, force = false) {
  const track = usePlayerStore().currentTrack;
  if (!track?.path || !track.sourceId) return;
  const key = trackKey(track) ?? null;
  const now = performance.now();
  if (!force && key === lastStreamReportKey && now - lastStreamReportAt < 1000) return;
  lastStreamReportKey = key;
  lastStreamReportAt = now;
  void invoke("report_stream_progress", {
    sourceId: track.sourceId,
    path: track.path,
    positionSecs: position,
    durationSecs: duration > 0 ? duration : 0,
  }).catch(() => {});
}

function onError(payload: AudioErrorPayload) {
  playbackLog("engine error", { code: payload.code, message: payload.message });
  pushToast("error", t(ERROR_KEYS[payload.code] ?? "playback.failed"));
  usePlayerStore().setPlaying(false);
}

/** Load the current track into the engine and start/stop it to match the store. */
async function syncTrack() {
  const player = usePlayerStore();
  const track = player.currentTrack;
  prefetchedKey = null;
  spectrumData.value = null;
  lastStreamReportKey = null;
  lastStreamReportAt = 0;
  if (!track?.path) {
    playbackLog("syncTrack: no track, stopping engine");
    await invoke("audio_stop").catch(() => {});
    return;
  }
  playbackLog("syncTrack: loading", { sourceId: track.sourceId, path: track.path, playing: player.isPlaying });
  try {
    await invoke("audio_load", { sourceId: track.sourceId, path: track.path, autoplay: player.isPlaying });
    // Keep the explicit transport command as a compatibility fallback for
    // older native command schemas and for a load completed while the store
    // changed its play intent.
    if (player.isPlaying) {
      await invoke("audio_play");
    }
    if (!useCacheStore().isCached(trackKey(track))) {
      void invoke("prefetch_track", { sourceId: track.sourceId, path: track.path }).catch(() => {});
    }
  } catch (error) {
    const appError = toAppError(error);
    playbackLog("syncTrack failed", { code: appError.code, message: appError.message });
    pushToast("error", t(ERROR_KEYS[appError.code] ?? "playback.failed"));
    player.setPlaying(false);
  }
}

/** Mirror the real cache coverage (Rust) into the progress bar's prefill. */
function syncBuffered() {
  const player = usePlayerStore();
  const track = player.currentTrack;
  if (!track?.path || !track.sourceId) {
    player.setBufferedProgress(0);
    return;
  }
  void invoke<number>("stream_progress", { sourceId: track.sourceId, path: track.path })
    .then((percent) => player.setBufferedProgress(percent))
    .catch(() => {});
}

/** Warm the next queue entry shortly before the current track ends. */
function maybePrefetchNext(position: number, duration: number) {
  if (duration <= 0 || duration - position > NEXT_PREFETCH_LEAD_SECS) return;
  if (!useNetworkStore().isUnmetered()) return;
  const player = usePlayerStore();
  if (player.repeat === "one") return;
  const queue = player.queue;
  const index = player.queueIndex;
  if (index < 0 || queue.length === 0) return;
  let nextIndex = index + 1;
  if (nextIndex >= queue.length) {
    if (player.repeat !== "all") return;
    nextIndex = 0;
  }
  const next = queue[nextIndex];
  if (!next?.path || !next.sourceId) return;
  const key = trackKey(next);
  if (!key || prefetchedKey === key || useCacheStore().isCached(key)) return;
  prefetchedKey = key;
  void invoke("prefetch_track", {
    sourceId: next.sourceId,
    path: next.path,
    leadSecs: NEXT_PREFETCH_SECONDS,
  }).catch(() => {});
}

/** Advance the queue, restarting the current track when repeat-one is on. */
function onEnded() {
  const player = usePlayerStore();
  playbackLog("engine ended", { repeat: player.repeat, queueIndex: player.queueIndex });
  if (player.repeat === "one" && player.currentTrack) {
    seekTo(0);
    return;
  }
  player.next();
}

/** Create the engine client and wire it to the store. Safe to call repeatedly. */
export function initAudio(): Promise<void> {
  initPromise ??= bootstrap().catch((error) => {
    initPromise = null;
    throw error;
  });
  return initPromise;
}

async function bootstrap(): Promise<void> {
  const player = usePlayerStore();

  unlisteners.push(await listen<AudioStatePayload>("audio:state", (event) => onState(event.payload)));
  unlisteners.push(await listen<SpectrumData>("audio:spectrum", (event) => onSpectrum(event.payload)));
  unlisteners.push(await listen("audio:ended", onEnded));
  unlisteners.push(await listen<AudioErrorPayload>("audio:error", (event) => onError(event.payload)));

  scope = effectScope();
  scope.run(() => {
    // Track identity includes the source, so switching to the same path on a
    // different source still reloads.
    watch(
      () => {
        const track = player.currentTrack;
        return track?.path ? `${track.sourceId ?? ""}::${track.path}` : "";
      },
      () => void syncTrack(),
    );

    // Re-selecting the same track (repeat-one, clicking the current row) bumps
    // the nonce without changing the path, so restart the engine explicitly.
    watch(
      () => player.playbackNonce,
      () => {
        if (!player.currentTrack?.path) return;
        playbackLog("playbackNonce: restarting current track");
        seekTo(0);
        if (player.isPlaying) void invoke("audio_play").catch(() => {});
      },
    );

    watch(
      () => player.isPlaying,
      (playing) => {
        playbackLog("isPlaying changed", { playing });
        void invoke(playing ? "audio_play" : "audio_pause").catch(() => {});
      },
    );

    watch(
      () => player.volume,
      (volume) => {
        void invoke("audio_set_volume", { volume }).catch(() => {});
      },
    );

    watch(
      () => player.muted,
      (muted) => {
        void invoke("audio_set_mute", { muted }).catch(() => {});
      },
    );

    watch(
      () => player.playbackRate,
      (rate) => {
        void invoke("audio_set_speed", { rate }).catch(() => {});
      },
    );
  });

  await invoke("audio_set_volume", { volume: player.volume }).catch(() => {});
  await invoke("audio_set_mute", { muted: player.muted }).catch(() => {});
  await invoke("audio_set_speed", { rate: player.playbackRate }).catch(() => {});
  await syncTrack();

  syncBuffered();
  bufferedTimer = window.setInterval(syncBuffered, 1000);
}

/** Current playback position in seconds, extrapolated between state events. */
export function currentTime(): number {
  if (!enginePlaying) return lastPositionMs / 1000;
  const elapsed = lastPositionAt > 0 ? (performance.now() - lastPositionAt) / 1000 : 0;
  // The source advances `rate` seconds per wall second, so scale the gap since
  // the last event (state ticks arrive at ~4 Hz) by the playback rate.
  return lastPositionMs / 1000 + elapsed * usePlayerStore().playbackRate;
}

/** Seek to an absolute number of seconds. */
export function seekTo(seconds: number) {
  if (!Number.isFinite(seconds) || seconds < 0) return;
  playbackLog("seekTo: sending audio_seek", { seconds, duration: usePlayerStore().duration });
  lastPositionMs = seconds * 1000;
  lastPositionAt = performance.now();
  usePlayerStore().setPosition(seconds);
  reportStreamProgress(seconds, usePlayerStore().duration, true);
  void invoke("audio_seek", { positionSecs: seconds }).catch(() => {});
}

/** Seek to a percentage (0-100) of the current track. */
export function seekPercent(value: number, commit = true) {
  const player = usePlayerStore();
  if (player.duration <= 0) return;
  const seconds = (Math.min(100, Math.max(0, value)) / 100) * player.duration;
  if (!commit) {
    // Keep the slider and time label responsive while dragging. Do not enqueue
    // a native decoder seek for every pointer movement.
    playbackLog("seekPercent: input preview (no seek sent)", { value, seconds });
    lastPositionMs = seconds * 1000;
    lastPositionAt = performance.now();
    player.setPosition(seconds);
    return;
  }
  playbackLog("seekPercent: commit", { value, seconds });
  seekTo(seconds);
}

/** Seek by a relative number of seconds (clamped to the track bounds). */
export function seekBy(delta: number) {
  const player = usePlayerStore();
  const max = player.duration > 0 ? player.duration : undefined;
  const target = player.position + delta;
  seekTo(max !== undefined ? Math.max(0, Math.min(max, target)) : Math.max(0, target));
}

/** Tear down listeners and watchers (mainly guards against HMR leaks). */
function disposeAudio() {
  scope?.stop();
  scope = null;
  for (const stop of unlisteners) stop();
  unlisteners = [];
  window.clearInterval(bufferedTimer);
  bufferedTimer = 0;
  initPromise = null;
  enginePlaying = false;
}

if (import.meta.hot) import.meta.hot.dispose(disposeAudio);
