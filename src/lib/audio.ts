import { effectScope, watch } from "vue";
import type { EffectScope } from "vue";
import { i18n } from "../i18n";
import { invoke, toAppError } from "../api";
import { usePlayerStore } from "../stores/player";
import { useNetworkStore } from "../stores/network";
import { useCacheStore } from "../stores/cache";
import { pushToast } from "./toast";
import { setStreamEndpoint, trackStreamUrl } from "./remote";
import { trackKey } from "./sources";
import type { StreamEndpoint } from "./remote";

/**
 * Single `<audio>` element driving playback. The store holds the state; this
 * module reacts to store changes (current track, play/pause, volume, mute) and
 * feeds media-element events back into the store.
 *
 * Audio is streamed from the Rust loopback proxy (see `src-tauri/src/proxy.rs`),
 * which injects WebDAV auth and pipes the upstream body, so the browser gets
 * native progressive playback and Range-based seeking.
 */
let audio: HTMLAudioElement | null = null;
/** In-flight/complete initialisation, so repeated calls never build a second element. */
let initPromise: Promise<void> | null = null;
/** Collects the store watchers so they can be stopped on teardown. */
let scope: EffectScope | null = null;
/** Position (seconds) to apply once the current source reports its metadata. */
let pendingSeek: number | null = null;
/**
 * Bumped whenever the media source changes. Async play/error results capture the
 * generation they were issued for, so a failure from a source that has since
 * been replaced can never pause the track the user just started.
 */
let sourceGeneration = 0;
/** Source generation whose failure was already surfaced (toast once per source). */
let reportedGeneration = -1;
/** No `timeupdate` for this long while the store wants to play means a stall. */
const STALL_TIMEOUT_MS = 4000;
/** Minimum gap between automatic stall-recovery attempts. */
const RECOVERY_COOLDOWN_MS = 3000;
/** Report a network error after this many consecutive failed recoveries. */
const MAX_STALL_RECOVERIES = 6;
/** Timestamp of the last observed playback progress, for stall detection. */
let lastProgressAt = 0;
/** Timestamp of the last stall-recovery attempt. */
let lastRecoveryAt = 0;
/** Consecutive recoveries without a healthy buffer; resets once playback is smooth. */
let stallRecoveries = 0;
/** Stall watchdog interval id. */
let watchdogTimer = 0;
/** Report the playhead to the Rust stream cache at most this often. */
const PROGRESS_REPORT_INTERVAL_MS = 3000;
/** Prefetch the next queue entry once this many seconds remain. */
const NEXT_PREFETCH_LEAD_SECS = 60;
/** How much of the next track to preload (seconds), when on Wi-Fi. */
const NEXT_PREFETCH_SECONDS = 15;
/** Source generation whose following track was already prefetched. */
let prefetchedNextGeneration = -1;
/** Timestamp of the last progress report sent to the Rust cache. */
let lastProgressReportAt = 0;

/**
 * Ask Rust to download a track into its read-ahead cache. `leadSecs` shortens
 * the window for a speculative preload; omit it for the normal lead. Rust
 * ignores local sources and tracks already in the smart cache.
 */
function prefetchTrack(track: { sourceId?: string; path?: string } | null, leadSecs?: number): void {
  if (!track?.path || !track.sourceId) return;
  void invoke("prefetch_track", { sourceId: track.sourceId, path: track.path, leadSecs }).catch(() => {});
}

/** The queue entry that will follow the current one, if any. */
function nextQueueTrack(): { sourceId?: string; path?: string } | null {
  const player = usePlayerStore();
  if (player.repeat === "one") return null;
  const queue = player.queue;
  const index = player.queueIndex;
  if (index < 0 || queue.length === 0) return null;
  let nextIndex = index + 1;
  if (nextIndex >= queue.length) {
    if (player.repeat !== "all") return null;
    nextIndex = 0;
  }
  return queue[nextIndex] ?? null;
}

/** Feed the playhead to the Rust cache so it can keep its lead window ahead. */
function reportStreamProgress(generation: number) {
  if (!audio || generation !== sourceGeneration) return;
  const now = performance.now();
  if (now - lastProgressReportAt < PROGRESS_REPORT_INTERVAL_MS) return;
  const track = usePlayerStore().currentTrack;
  if (!track?.path || !track.sourceId) return;
  lastProgressReportAt = now;
  const duration = audio.duration;
  void invoke("report_stream_progress", {
    sourceId: track.sourceId,
    path: track.path,
    positionSecs: audio.currentTime,
    durationSecs: Number.isFinite(duration) && duration > 0 ? duration : 0,
  }).catch(() => {});
}

/**
 * Warm the next queue entry shortly before the current track ends. Speculative,
 * so it is limited to a short 15s head start and skipped on metered networks.
 */
function maybePrefetchNext(generation: number) {
  if (!audio || generation !== sourceGeneration) return;
  if (prefetchedNextGeneration === generation) return;
  const duration = audio.duration;
  if (!(duration > 0)) return;
  if (duration - audio.currentTime > NEXT_PREFETCH_LEAD_SECS) return;
  if (!useNetworkStore().isUnmetered()) return;
  prefetchedNextGeneration = generation;
  const next = nextQueueTrack();
  if (next && useCacheStore().isCached(trackKey(next))) return;
  prefetchTrack(next, NEXT_PREFETCH_SECONDS);
}

/** Localization keys for the `AppError` codes a stream probe can return. */
const PROBE_ERROR_KEYS: Record<string, string> = {
  UNAUTHORIZED: "errors.unauthorized",
  FORBIDDEN: "errors.forbidden",
  NOT_FOUND: "errors.notFound",
  TIMEOUT: "errors.timeout",
  DNS: "errors.dns",
  TLS: "errors.tls",
  CONNECTION: "errors.connection",
  HTTP: "errors.http",
  XML: "errors.xml",
  MISSING_CREDENTIALS: "errors.missingCredentials",
  INSECURE_URL: "errors.insecureUrl",
  STORE: "errors.store",
  INVALID_ARGUMENT: "errors.invalidArgument",
};

function t(key: string): string {
  return i18n.global.t(key);
}

function describeMediaError(element: HTMLAudioElement | null): string {
  switch (element?.error?.code) {
    case MediaError.MEDIA_ERR_SRC_NOT_SUPPORTED:
      return t("playback.unsupported");
    case MediaError.MEDIA_ERR_NETWORK:
      return t("playback.network");
    default:
      return t("playback.failed");
  }
}

/**
 * Rejections that are part of normal playback control rather than real errors.
 * A pending `play()` promise rejects with `AbortError` whenever a newer source is
 * loaded or the element is paused, and the autoplay policy rejects with
 * `NotAllowedError`. Both happen routinely while swapping tracks, so they must
 * never surface as a "playback failed" toast or force playback to stop.
 */
function isBenignPlayRejection(error: DOMException | null | undefined): boolean {
  return error?.name === "AbortError" || error?.name === "NotAllowedError";
}

/**
 * React to a real playback failure. Failures from a superseded source are
 * ignored entirely so they cannot clobber the current track; the media `error`
 * event and the rejected `play()` promise describe the same failure, so the
 * toast is only shown once per source.
 */
function fail(message: string, generation: number) {
  if (generation !== sourceGeneration) return;
  const player = usePlayerStore();
  audio?.pause();
  player.setPlaying(false);
  if (generation === reportedGeneration) return;
  reportedGeneration = generation;
  pushToast("error", message);
}

function play() {
  if (!audio) return;
  const generation = sourceGeneration;
  const track = usePlayerStore().currentTrack;
  audio.play().catch((error: DOMException) => {
    if (isBenignPlayRejection(error)) return;
    if (error?.name === "NotSupportedError" && track?.path) {
      void reportSourceUnavailable(generation, track.path, track.sourceId);
      return;
    }
    fail(t("playback.failed"), generation);
  });
}

/** Guard so a media `error` and its rejected `play()` promise share one probe. */
let probingGeneration = -1;

/**
 * The media element collapses every failed fetch — a `401` from bad credentials,
 * a `404`, a `5xx` — into `MEDIA_ERR_SRC_NOT_SUPPORTED`, so on its own it cannot
 * tell a decode problem from an authentication failure. Ask the backend (which
 * owns the credentials) why the stream is unavailable and report the real
 * reason; a successful probe means the file was served, i.e. the format.
 */
async function reportSourceUnavailable(generation: number, path: string, sourceId?: string) {
  if (
    generation !== sourceGeneration ||
    generation === reportedGeneration ||
    generation === probingGeneration
  ) {
    return;
  }
  probingGeneration = generation;
  try {
    await invoke("probe_track", { sourceId, path });
    fail(t("playback.unsupported"), generation);
  } catch (error) {
    const key = PROBE_ERROR_KEYS[toAppError(error).code];
    fail(key ? t(key) : t("playback.failed"), generation);
  } finally {
    if (probingGeneration === generation) probingGeneration = -1;
  }
}

function syncBuffered() {
  if (!audio) return;
  const player = usePlayerStore();
  if (player.duration <= 0 || audio.buffered.length === 0) {
    player.setBufferedProgress(0);
    return;
  }

  let bufferedEnd = 0;
  for (let index = 0; index < audio.buffered.length; index++) {
    const start = audio.buffered.start(index);
    const end = audio.buffered.end(index);
    if (audio.currentTime >= start && audio.currentTime <= end) {
      bufferedEnd = end;
      break;
    }
    if (start <= audio.currentTime) bufferedEnd = end;
  }
  player.setBufferedProgress((bufferedEnd / player.duration) * 100);
}

function setSource(path: string, sourceId?: string) {
  if (!audio) return;
  const url = trackStreamUrl(path, sourceId);
  if (!url) return;

  const player = usePlayerStore();
  // A new source must never inherit a seek that was queued for the previous track.
  pendingSeek = null;
  sourceGeneration += 1;
  lastProgressAt = performance.now();
  lastRecoveryAt = 0;
  stallRecoveries = 0;
  prefetchedNextGeneration = -1;
  lastProgressReportAt = 0;
  audio.src = url;
  audio.load();
  player.setPosition(0);
  player.setDuration(0);
  player.setBufferedProgress(0);
  if (!useCacheStore().isCached(trackKey({ path, sourceId }))) prefetchTrack({ path, sourceId });
  if (player.isPlaying) play();
}

/**
 * Playback can stall exactly at the end of the buffered range when a media
 * request fails: the element stops fetching but never fires `error`, so it waits
 * forever. A new request is only issued when playback seeks outside the buffered
 * range — which is why the manual workaround is to drag forward (a backward seek
 * stays buffered and does nothing). Reload the resource at the current position
 * to force a fresh range request without drifting forward.
 */
function recoverFromStall() {
  if (!audio) return;
  const now = performance.now();
  if (now - lastRecoveryAt < RECOVERY_COOLDOWN_MS) return;
  lastRecoveryAt = now;
  // Give the fresh request a full timeout before judging it again.
  lastProgressAt = now;

  if (stallRecoveries >= MAX_STALL_RECOVERIES) {
    fail(t("playback.network"), sourceGeneration);
    return;
  }
  stallRecoveries += 1;

  const player = usePlayerStore();
  const resumeAt = audio.currentTime;
  pendingSeek = resumeAt > 0 ? resumeAt : null;
  audio.load();
  if (player.isPlaying) play();
}

/** Watchdog: detect playback that stopped advancing at the buffered edge. */
function checkStall() {
  if (!audio) return;
  const player = usePlayerStore();
  if (!player.isPlaying || audio.paused || audio.ended || audio.seeking) return;
  // A load that never produced metadata is handled by the error path instead.
  if (audio.readyState < HTMLMediaElement.HAVE_METADATA) return;

  const now = performance.now();
  if (audio.readyState >= HTMLMediaElement.HAVE_FUTURE_DATA) {
    // Enough data is buffered to keep playing: playback is healthy again.
    lastProgressAt = now;
    stallRecoveries = 0;
    return;
  }
  if (now - lastProgressAt < STALL_TIMEOUT_MS) return;
  recoverFromStall();
}

/**
 * Create the single media element and wire it to the store. Safe to call
 * repeatedly: the first call wins and later calls await the same promise, so
 * only one `<audio>` (and one set of watchers) ever exists.
 */
export function initAudio(): Promise<void> {
  initPromise ??= bootstrapAudio().catch((error) => {
    // Don't cache a failed initialisation — allow a later retry.
    initPromise = null;
    throw error;
  });
  return initPromise;
}

async function bootstrapAudio(): Promise<void> {
  const player = usePlayerStore();

  try {
    setStreamEndpoint(await invoke<StreamEndpoint>("stream_endpoint"));
  } catch (error) {
    console.error("[welkin] stream endpoint unavailable", error);
    pushToast("error", t("playback.failed"));
  }

  audio = new Audio();
  audio.preload = "metadata";
  audio.volume = Math.min(1, Math.max(0, player.volume / 100));
  audio.muted = player.muted;

  audio.addEventListener("loadedmetadata", () => {
    player.setDuration(audio?.duration ?? 0);
    syncBuffered();
    if (pendingSeek !== null && audio) {
      const limit = Number.isFinite(audio.duration) && audio.duration > 0 ? audio.duration : pendingSeek;
      audio.currentTime = Math.min(pendingSeek, limit);
      player.setPosition(audio.currentTime);
      pendingSeek = null;
    }
  });
  audio.addEventListener("durationchange", () => { player.setDuration(audio?.duration ?? 0); syncBuffered(); });
  audio.addEventListener("timeupdate", () => {
    player.setPosition(audio?.currentTime ?? 0);
    syncBuffered();
    lastProgressAt = performance.now();
    reportStreamProgress(sourceGeneration);
    maybePrefetchNext(sourceGeneration);
  });
  audio.addEventListener("progress", syncBuffered);
  audio.addEventListener("play", () => { lastProgressAt = performance.now(); if (!player.isPlaying) player.setPlaying(true); });
  audio.addEventListener("pause", () => { if (player.isPlaying) player.setPlaying(false); });
  audio.addEventListener("ended", () => {
    // A source that "ends" without ever reporting a duration is a broken/empty
    // file. Auto-advancing would spin through the whole queue, so report it and
    // stop instead.
    if (!audio || !(audio.duration > 0)) {
      fail(t("playback.failed"), sourceGeneration);
      return;
    }
    player.next();
  });
  audio.addEventListener("error", () => {
    // Ignore the error fired when the source is intentionally cleared.
    const track = player.currentTrack;
    if (!track?.path) return;
    // A missing/undecodable source is indistinguishable from a failed fetch
    // here, so probe for the real reason instead of blaming the format.
    if (audio?.error?.code === MediaError.MEDIA_ERR_SRC_NOT_SUPPORTED) {
      void reportSourceUnavailable(sourceGeneration, track.path, track.sourceId);
      return;
    }
    fail(describeMediaError(audio), sourceGeneration);
  });

  scope = effectScope();
  scope.run(() => {
    // Track identity includes the source, so switching to the same path on a
    // different source still reloads the element.
    watch(
      () => {
        const track = player.currentTrack;
        return track?.path ? `${track.sourceId ?? ""}::${track.path}` : "";
      },
      () => {
        if (!audio) return;
        const track = player.currentTrack;
        if (!track?.path) {
          pendingSeek = null;
          sourceGeneration += 1;
          lastRecoveryAt = 0;
          stallRecoveries = 0;
          prefetchedNextGeneration = -1;
          audio.removeAttribute("src");
          audio.load();
          return;
        }
        setSource(track.path, track.sourceId);
      },
    );

    // Re-selecting the same track (repeat-one, clicking the current row) bumps
    // the nonce without changing the path, so restart the element explicitly.
    watch(
      () => player.playbackNonce,
      () => {
        if (!audio || !player.currentTrack?.path) return;
        pendingSeek = null;
        audio.currentTime = 0;
        // The same element keeps its duration, but the store was rewound; make
        // sure the duration/progress baseline is still valid so the bar moves.
        player.setDuration(Number.isFinite(audio.duration) && audio.duration > 0 ? audio.duration : 0);
        player.setPosition(0);
        syncBuffered();
        if (player.isPlaying) play();
      },
    );

    watch(
      () => player.isPlaying,
      (playing) => {
        if (!audio || !player.currentTrack?.path) return;
        if (playing) {
          // Don't stack redundant play() promises while one is already pending
          // or the element is already running.
          if (audio.paused) play();
        } else {
          audio.pause();
        }
      },
    );

    watch(
      () => player.volume,
      (volume) => {
        if (audio) audio.volume = Math.min(1, Math.max(0, volume / 100));
      },
    );

    watch(
      () => player.muted,
      (muted) => {
        if (audio) audio.muted = muted;
      },
    );
  });

  lastProgressAt = performance.now();
  watchdogTimer = window.setInterval(checkStall, 1000);

  const initialTrack = player.currentTrack;
  if (initialTrack?.path) setSource(initialTrack.path, initialTrack.sourceId);
}

/** Tear the element and its watchers down. Mainly guards against HMR leaks. */
function disposeAudio() {
  scope?.stop();
  scope = null;
  if (audio) {
    audio.pause();
    audio.removeAttribute("src");
    audio.load();
    audio = null;
  }
  window.clearInterval(watchdogTimer);
  watchdogTimer = 0;
  initPromise = null;
  pendingSeek = null;
  lastProgressAt = 0;
  lastRecoveryAt = 0;
  stallRecoveries = 0;
}

if (import.meta.hot) import.meta.hot.dispose(disposeAudio);

/** Current playback time of the audio element, in seconds. */
export function currentTime(): number {
  return audio?.currentTime ?? 0;
}

/** Seek to a percentage (0-100) of the current track. */
export function seekPercent(value: number) {
  const player = usePlayerStore();
  if (!audio || player.duration <= 0) return;
  const seconds = (Math.min(100, Math.max(0, value)) / 100) * player.duration;
  audio.currentTime = seconds;
  player.setPosition(seconds);
}

/** Seek to an absolute number of seconds (applied once metadata is ready). */
export function seekTo(seconds: number) {
  const player = usePlayerStore();
  if (!Number.isFinite(seconds) || seconds <= 0) return;
  if (!audio) {
    pendingSeek = seconds;
    return;
  }
  const duration = Number.isFinite(audio.duration) && audio.duration > 0 ? audio.duration : 0;
  if (duration > 0) {
    audio.currentTime = Math.min(seconds, duration);
    player.setPosition(audio.currentTime);
  } else {
    pendingSeek = seconds;
  }
}

/** Seek by a relative number of seconds (clamped to the track bounds). */
export function seekBy(delta: number) {
  const player = usePlayerStore();
  if (!audio) return;
  const max = player.duration > 0 ? player.duration : audio.duration;
  audio.currentTime = Math.max(0, Math.min(max || 0, audio.currentTime + delta));
}
