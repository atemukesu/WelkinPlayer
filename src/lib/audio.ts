import { watch } from "vue";
import { i18n } from "../i18n";
import { invoke } from "../api";
import { usePlayerStore } from "../stores/player";
import { pushToast } from "./toast";
import { setStreamEndpoint, trackStreamUrl } from "./remote";
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
let initialized = false;
/** Position (seconds) to apply once the current source reports its metadata. */
let pendingSeek: number | null = null;

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

function fail(message: string) {
  const player = usePlayerStore();
  audio?.pause();
  player.setPlaying(false);
  pushToast("error", message);
}

function play() {
  const result = audio?.play();
  if (result) {
    result.catch((error: DOMException) => {
      fail(error?.name === "NotSupportedError" ? t("playback.unsupported") : t("playback.failed"));
    });
  }
}

function syncBuffered() {
  if (!audio) return;
  const player = usePlayerStore();
  if (player.duration <= 0 || audio.buffered.length === 0) {
    player.bufferedProgress = 0;
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
  player.bufferedProgress = (bufferedEnd / player.duration) * 100;
}

function setSource(path: string) {
  if (!audio) return;
  const url = trackStreamUrl(path);
  if (!url) return;

  const player = usePlayerStore();
  audio.src = url;
  audio.load();
  player.setPosition(0);
  player.setDuration(0);
  player.bufferedProgress = 0;
  if (player.isPlaying) play();
}

export async function initAudio() {
  if (initialized) return;
  initialized = true;

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
  audio.addEventListener("timeupdate", () => { player.setPosition(audio?.currentTime ?? 0); syncBuffered(); });
  audio.addEventListener("progress", syncBuffered);
  audio.addEventListener("play", () => player.setPlaying(true));
  audio.addEventListener("pause", () => player.setPlaying(false));
  audio.addEventListener("ended", () => player.next());
  audio.addEventListener("error", () => {
    // Ignore the error fired when the source is intentionally cleared.
    if (!player.currentTrack?.path) return;
    fail(describeMediaError(audio));
  });

  watch(
    () => player.currentTrack?.path,
    (path) => {
      if (!audio) return;
      if (!path) {
        audio.removeAttribute("src");
        audio.load();
        return;
      }
      setSource(path);
    },
  );

  watch(
    () => player.isPlaying,
    (playing) => {
      if (!audio || !player.currentTrack?.path) return;
      if (playing) play();
      else audio.pause();
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

  const initialPath = player.currentTrack?.path;
  if (initialPath) setSource(initialPath);
}

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
