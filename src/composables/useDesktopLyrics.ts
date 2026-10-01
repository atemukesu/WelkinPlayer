import { effectScope, watch } from "vue";
import type { EffectScope } from "vue";
import { findActiveLyricIndices, pickPrimaryIndex } from "lyric-kit";
import { invoke } from "../api";
import { currentTime } from "../lib/audio";
import type { DesktopLyricLine, DesktopLyricLoadPayload, DesktopLyricTickPayload } from "../lib/desktopLyric";
import { useDesktopLyricsStore } from "../stores/desktopLyrics";
import { useLyricsStore } from "../stores/lyrics";
import { usePlayerStore } from "../stores/player";

/** Cadence for the playhead document (ms). ~30 fps is plenty for text. */
const TICK_INTERVAL_MS = 33;

let running = false;
let timer = 0;
let scope: EffectScope | null = null;
let lastKey = "";

/** Flatten the parsed lyric lines into the renderer's transfer shape. */
function toLines(): DesktopLyricLine[] {
  const lines = useLyricsStore().lines;
  return lines.map((line) => ({
    text: line.words.map((word) => word.word).join(""),
    translation: line.translatedLyric ?? "",
    roman: line.romanLyric ?? "",
    words: line.words.map((word) => ({ text: word.word, start: word.startTime, end: word.endTime })),
    isBG: Boolean(line.isBG),
    isDuet: Boolean(line.isDuet),
  }));
}

/** Track identity used to decide when a fresh snapshot must be pushed. */
function trackKey(): string {
  const track = usePlayerStore().currentTrack;
  if (!track?.path) return "";
  return `${track.sourceId ?? ""}::${track.path}`;
}

/** Whether the floating layer should currently be painted. */
function shouldShow(hasLines: boolean): boolean {
  const dl = useDesktopLyricsStore();
  const player = usePlayerStore();
  const lyrics = useLyricsStore();
  if (!dl.settings.enabled || lyrics.suppressed) return false;
  if (!hasLines && dl.settings.hideWhenNoLyrics) return false;
  if (dl.settings.hideOnPause && !player.isPlaying) return false;
  return true;
}

function pushLoad() {
  const player = usePlayerStore();
  const dl = useDesktopLyricsStore();
  const payload: DesktopLyricLoadPayload = {
    title: player.currentTrack?.title ?? "",
    artist: player.currentTrack?.artist ?? "",
    lines: toLines(),
    settings: { ...dl.settings },
  };
  void invoke("desktop_lyric_load", { payload }).catch((error) => {
    dl.error = String(error);
  });
}

function pushSettings() {
  const dl = useDesktopLyricsStore();
  void invoke("desktop_lyric_set_settings", { settings: { ...dl.settings } }).catch(() => {});
}

function tick() {
  const player = usePlayerStore();
  const lyrics = useLyricsStore();
  const dl = useDesktopLyricsStore();
  if (!dl.settings.enabled) return;
  const positionMs = currentTime() * 1000;
  const lines = lyrics.lines;
  const activeIndex = lines.length > 0 ? pickPrimaryIndex(lines, positionMs) : -1;
  const activeIndices = lines.length > 0 ? findActiveLyricIndices(lines, positionMs) : [];
  const payload: DesktopLyricTickPayload = {
    positionMs,
    playing: player.isPlaying,
    activeIndex,
    activeIndices,
    visible: shouldShow(lines.length > 0),
  };
  void invoke("desktop_lyric_tick", { payload }).catch(() => {});
}

/** (Re)push the full lyric snapshot when the track or its lyrics change. */
function syncSnapshot() {
  const key = `${trackKey()}::${useLyricsStore().lines.length}::${useLyricsStore().status}`;
  if (key === lastKey) return;
  lastKey = key;
  pushLoad();
}

/** Start forwarding playback state to the floating layer. */
export function startDesktopLyrics() {
  if (running) return;
  running = true;
  const dl = useDesktopLyricsStore();

  scope = effectScope();
  scope.run(() => {
    watch(
      () => dl.settings.enabled,
      (enabled) => {
        if (enabled) void openFloating();
        else void closeFloating();
      },
      { immediate: true },
    );
    watch(() => [trackKey(), useLyricsStore().lines, useLyricsStore().status] as const, syncSnapshot, {
      immediate: true,
    });
    watch(() => dl.settings, pushSettings, { deep: true });
  });

  timer = window.setInterval(tick, TICK_INTERVAL_MS);
}

/** Stop forwarding and tear the floating layer down. */
export function stopDesktopLyrics() {
  if (!running) return;
  running = false;
  scope?.stop();
  scope = null;
  window.clearInterval(timer);
  timer = 0;
  lastKey = "";
}

/** Ask the backend to bring the floating renderer up. */
async function openFloating() {
  const dl = useDesktopLyricsStore();
  dl.busy = true;
  try {
    const status = await invoke<{ active: boolean; permissionGranted: boolean; supported: boolean }>("desktop_lyric_open");
    dl.active = status.active;
    dl.permissionGranted = status.permissionGranted;
    dl.supported = status.supported;
    dl.error = status.active ? "" : status.supported ? "permission" : "unsupported";
    if (status.active) {
      lastKey = "";
      syncSnapshot();
      pushSettings();
    }
  } catch (error) {
    dl.active = false;
    dl.error = String(error);
  } finally {
    dl.busy = false;
  }
}

async function closeFloating() {
  const dl = useDesktopLyricsStore();
  try {
    await invoke("desktop_lyric_close");
  } catch {
    /* ignore */
  }
  dl.active = false;
}

/** Android only: open the system overlay-permission screen, then retry. */
export async function requestDesktopLyricPermission() {
  const dl = useDesktopLyricsStore();
  try {
    await invoke("desktop_lyric_request_permission");
  } catch {
    /* ignore */
  }
  // The user may take a while; poll a few times and start the layer once the
  // permission lands (the `enabled` watcher will not re-run by itself).
  const deadline = Date.now() + 60_000;
  const poll = window.setInterval(() => {
    void invoke<{ active: boolean; permissionGranted: boolean; supported: boolean }>("desktop_lyric_status")
      .then((status) => {
        dl.permissionGranted = status.permissionGranted;
        dl.supported = status.supported;
        if (status.permissionGranted) {
          dl.error = "";
          window.clearInterval(poll);
          if (dl.settings.enabled && !dl.active) void openFloating();
        }
      })
      .catch(() => {});
    if (Date.now() > deadline) window.clearInterval(poll);
  }, 1200);
}
