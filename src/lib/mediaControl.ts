import { effectScope, watch } from "vue";
import type { EffectScope } from "vue";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "../api";
import { currentTime, seekTo } from "./audio";
import { isAndroid } from "./desktopLyric";
import { trackKey } from "./sources";
import { usePlayerStore } from "../stores/player";

/**
 * Mirrors the player store onto the OS transport controls (Windows SMTC, macOS
 * Now Playing, Linux MPRIS, Android MediaSession) and forwards transport
 * commands from those surfaces back into the store.
 *
 * Metadata + play/pause are pushed on change; the playhead is pushed on a slow
 * cadence because the OS extrapolates position while playing. Transport events
 * arrive as a `media:control` Tauri event on desktop and as a polled native
 * queue on Android.
 */
const POSITION_TICK_MS = 5000;
const ANDROID_POLL_MS = 250;

interface ControlCommand {
  action?: string;
  value?: number | null;
}

let running = false;
let scope: EffectScope | null = null;
let tickTimer = 0;
let pollTimer = 0;
let unlisten: UnlistenFn | null = null;

/** Full metadata + playback snapshot for the OS. */
function snapshot() {
  const player = usePlayerStore();
  const track = player.currentTrack;
  return {
    title: track?.title ?? "",
    artist: track?.artist ?? "",
    album: track?.album ?? "",
    durationMs: Math.round((player.duration > 0 ? player.duration : 0) * 1000),
    positionMs: Math.round(currentTime() * 1000),
    playing: player.isPlaying,
    volume: player.volume,
    coverHash: track?.coverHash ?? null,
  };
}

/** Push metadata + play/pause (or clear the UI when nothing is selected). */
function pushFull() {
  const player = usePlayerStore();
  if (!player.currentTrack) {
    void invoke("media_control_clear").catch(() => {});
    return;
  }
  void invoke("media_control_update", { payload: snapshot() }).catch(() => {});
}

/** Correct playhead drift; metadata is intentionally left alone. */
function pushPosition() {
  const player = usePlayerStore();
  if (!player.currentTrack || !player.isPlaying) return;
  void invoke("media_control_position", {
    positionMs: Math.round(currentTime() * 1000),
    playing: true,
  }).catch(() => {});
}

function parseControl(raw: string): ControlCommand | null {
  try {
    const parsed = JSON.parse(raw);
    return parsed && typeof parsed === "object" ? parsed : null;
  } catch {
    return null;
  }
}

/** Apply a transport command from the OS. */
function handleControl(raw: unknown) {
  const command = (typeof raw === "string" ? parseControl(raw) : raw) as ControlCommand | null;
  if (!command || typeof command.action !== "string") return;
  const player = usePlayerStore();
  switch (command.action) {
    case "play":
      if (!player.isPlaying) player.setPlaying(true);
      break;
    case "pause":
      if (player.isPlaying) player.setPlaying(false);
      break;
    case "toggle":
      player.togglePlayback();
      break;
    case "next":
      player.next();
      break;
    case "previous":
      player.previous();
      break;
    case "stop":
      player.setPlaying(false);
      break;
    case "seek":
      seekTo(Math.max(0, (command.value ?? 0) / 1000));
      break;
    case "seekBy":
      seekTo(Math.max(0, currentTime() + (command.value ?? 0)));
      break;
    case "volume":
      player.setVolume(Math.round((command.value ?? 0) * 100));
      break;
  }
}

/** Start mirroring playback to the OS media controls. */
export function startMediaControl() {
  if (running) return;
  running = true;

  scope = effectScope();
  scope.run(() => {
    watch(
      () => {
        const player = usePlayerStore();
        return `${trackKey(player.currentTrack)}::${player.isPlaying}`;
      },
      pushFull,
      { immediate: true },
    );
  });

  void listen<unknown>("media:control", (event) => {
    handleControl(event.payload);
  }).then((stop) => {
    unlisten = stop;
  });

  tickTimer = window.setInterval(pushPosition, POSITION_TICK_MS);
  if (isAndroid()) {
    pollTimer = window.setInterval(() => {
      void invoke<string | null>("media_control_take_control")
        .then((action) => {
          if (action) handleControl(action);
        })
        .catch(() => {});
    }, ANDROID_POLL_MS);
  }
}

/** Stop mirroring and tear the listeners down. */
export function stopMediaControl() {
  if (!running) return;
  running = false;
  scope?.stop();
  scope = null;
  window.clearInterval(tickTimer);
  tickTimer = 0;
  window.clearInterval(pollTimer);
  pollTimer = 0;
  unlisten?.();
  unlisten = null;
  void invoke("media_control_clear").catch(() => {});
}
