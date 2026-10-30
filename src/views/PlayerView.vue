<!--
 Copyright 2026 Atemukesu
 SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, toRaw, watch } from "vue";
import {
  ChevronDown,
  LoaderCircle,
  Maximize2,
  Minimize2,
  Pause,
  Play,
  Repeat,
  Repeat1,
  Shuffle,
  SkipBack,
  SkipForward,
} from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { usePlayerStore } from "../stores/player";
import { useLyricsStore } from "../stores/lyrics";
import type { LyricLine } from "../stores/lyrics";
import { currentTime, seekPercent } from "../lib/audio";
import { coverPending, initial, percent } from "../lib/format";
import { cssFontFamily } from "../lib/fonts";
import { useWakeLock } from "../composables/useWakeLock";
import RealtimeWaveform from "../components/RealtimeWaveform.vue";
import { Virtualizer } from "virtua/vue";
import type { VirtualizerHandle } from "virtua/vue";
import type { View } from "../lib/app";

defineProps<{ returnView: View }>();
const emit = defineEmits<{ navigate: [view: View]; queue: []; openArtist: [artist: string]; openAlbum: [album: string] }>();
const { t } = useI18n();
const player = usePlayerStore();
const lyrics = useLyricsStore();

/** Keep the display on for as long as this full-screen player is open. */
useWakeLock();

const lyricsScroll = ref<HTMLElement | null>(null);
const lyricAtTop = ref(true);
const lyricAtBottom = ref(false);
const lyricsVirtualizer = ref<VirtualizerHandle | null>(null);
/** Whether a user scroll is currently holding automatic line following. */
let manualScroll = false;
let lastScrolledIndex = -1;
let frame = 0;
let running = false;
let lastLyricsSync = 0;
let programmaticScroll = false;
let programmaticTimer = 0;
let followTarget = -1;
let manualTimer = 0;
let manualDeadline = 0;
let settleTimer = 0;

/** How long a manual scroll suspends automatic line following (ms). */
const MANUAL_RESUME_MS = 2500;
/** Fraction of the viewport the active lyric line is anchored at. */
const ACTIVE_LINE_RATIO = 0.35;
/** Minimum word-progress change before the gradient is rewritten. */
const WORD_PROGRESS_EPSILON = 0.01;
/** Playback/lyrics refresh cadence (ms). */
const LYRICS_TICK_MS = 33;
/** Retry delay while the active line is not mounted by the virtualizer (ms). */
const WORD_CACHE_RETRY_MS = 100;

/** CSS `font-family` value for the standard lyrics renderer, if configured. */
const lyricFontFamily = computed(() => cssFontFamily(lyrics.classic.fontFamilies));
/** Active lines stay a touch heavier than the configured base weight. */
const lyricActiveWeight = computed(() => Math.min(900, lyrics.classic.fontWeight + 200));
/** Whether the per-word readings (注音 / ruby) carried by the lyrics are shown. */
const showRuby = computed(() => lyrics.classic.ruby);
/** Initial row height hint for the virtualizer, following the font settings. */
const lyricItemSize = computed(() => {
  const { lineSize, lineSpacing, translationSize, translate } = lyrics.classic;
  const translation = translate ? translationSize * 1.5 + 6 : 0;
  return Math.round(Math.min(220, Math.max(48, lineSize * 1.4 + lineSpacing + translation + 8)));
});

/** Join a word's ruby spans into the single label shown above it. */
function rubyText(word: { ruby?: Array<{ word: string }> }): string {
  return (word.ruby ?? []).map((span) => span.word).join("");
}

/**
 * A line carries genuine per-word timestamps only when its words have
 * distinct, valid start times. Line-level timing (every word shares one
 * timestamp) means there is no word-by-word sweep to run, so the whole line
 * is highlighted instead of attempting a per-word animation.
 */
function lineHasWordTimestamps(line: LyricLine): boolean {
  const words = line.words;
  if (!words || words.length < 2) return false;
  const starts = words.map((word) => Number(word.startTime)).filter(Number.isFinite);
  if (starts.length !== words.length) return false;
  return new Set(starts).size > 1;
}

/**
 * Every line that should render as active: the primary line (which advances
 * ahead of the beat and ignores background vocals) plus any line whose own time
 * window overlaps it — background vocals and duet counter-lines.
 */
const activeSet = computed(() => {
  const set = new Set(lyrics.activeIndices);
  if (lyrics.activeIndex >= 0) set.add(lyrics.activeIndex);
  return set;
});

const activeKey = computed(() =>
  Array.from(activeSet.value).sort((a, b) => a - b).join(",")
);

/** Line indices that carry genuine per-word timestamps. Lines without them
 * (line-level timing only) get a whole-line highlight instead of a word sweep. */
const wordTimestampLines = computed(() =>
  new Set(
    lyrics.lines
      .map((line, index) => ({ line, index }))
      .filter(({ line }) => lineHasWordTimestamps(line))
      .map(({ index }) => index),
  ),
);

const controlsVisible = ref(true);
const isFullscreen = ref(false);
let controlsTimer = 0;

let lastBump = 0;
function bumpControls() {
  const now = performance.now();
  if (now - lastBump < 100) return;
  lastBump = now;
  controlsVisible.value = true;
  window.clearTimeout(controlsTimer);
  controlsTimer = window.setTimeout(() => { controlsVisible.value = false; }, 1000);
}

const revealClass = computed(() => (controlsVisible.value
  ? "pointer-events-auto translate-y-0 opacity-100"
  : "pointer-events-none -translate-y-2 opacity-0"));

let fullscreenTimer = 0;
let fullscreenRequest = 0;

async function syncFullscreen() {
  const request = ++fullscreenRequest;
  try {
    const value = await getCurrentWindow().isFullscreen();
    if (request === fullscreenRequest) isFullscreen.value = value;
  } catch {
    if (request === fullscreenRequest) isFullscreen.value = Boolean(document.fullscreenElement);
  }
}

/** Resize events are frequent; collapse them into one IPC round-trip. */
function scheduleSyncFullscreen() {
  window.clearTimeout(fullscreenTimer);
  fullscreenTimer = window.setTimeout(() => { void syncFullscreen(); }, 150);
}

async function toggleFullscreen() {
  try {
    await getCurrentWindow().setFullscreen(!isFullscreen.value);
    await syncFullscreen();
  } catch {
    if (document.fullscreenElement) await document.exitFullscreen();
    else await document.documentElement.requestFullscreen();
    await syncFullscreen();
  }
}

const fullscreenTitle = computed(() => (isFullscreen.value ? t("controls.exitFullscreen") : t("controls.fullscreen")));

const repeatTitle = computed(() => player.repeat === "one" ? t("controls.repeatOne") : player.repeat === "all" ? t("controls.repeatAll") : t("controls.repeatOff"));
const shuffleTitle = computed(() => player.shuffle ? t("controls.shuffleOn") : t("controls.shuffleOff"));
/** Whether the total-time label toggles to show the remaining time instead. */
const showRemaining = ref(false);
/** True when the rate actually playing is Hi-Res (above the 48 kHz CD ceiling). */
const isHiRes = computed(() => player.outputRate > 48000);

/**
 * Scrub state. While the user drags, the slider shows `seekValue` instead of
 * `player.progress`, so the engine's ~4 Hz state ticks cannot yank the thumb
 * back mid-gesture. The value is committed on pointer release (and on `change`
 * for keyboard), never left dangling as a bare preview.
 */
const seekValue = ref<number | null>(null);
const sliderValue = computed(() => seekValue.value ?? player.progress);
let pointerSeeking = false;
let pointerCommitDone = false;

/** Seconds shown by the time labels while dragging, without touching the engine. */
const previewPosition = computed(() => {
  if (seekValue.value === null) return null;
  return (Math.min(100, Math.max(0, seekValue.value)) / 100) * player.duration;
});

function formatClock(seconds: number): string {
  const total = Number.isFinite(seconds) && seconds > 0 ? Math.floor(seconds) : 0;
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}

const elapsedDisplay = computed(() =>
  previewPosition.value === null ? player.elapsedTime : formatClock(previewPosition.value),
);
const remainingDisplay = computed(() => {
  if (previewPosition.value === null) return player.remainingTime;
  return player.duration > 0
    ? formatClock(Math.max(0, player.duration - previewPosition.value))
    : player.totalTime;
});

function onSeek(event: Event) {
  const value = Number((event.target as HTMLInputElement).value);
  if (!Number.isFinite(value)) return;
  // Keep the preview local: committing `player.position` (and the module-level
  // playback clock) on every input event would re-render the app and send IPC
  // logs at pointer frequency.
  seekValue.value = value;
  pointerCommitDone = false;
}

function commitSeek(event: Event) {
  // `change` handles keyboard and is the normal release path, but the webview
  // can skip it after a pointer release; a pointer gesture already commits.
  if (pointerSeeking) return;
  if (pointerCommitDone) {
    pointerCommitDone = false;
    return;
  }
  const value = Number((event.target as HTMLInputElement).value);
  if (!Number.isFinite(value)) return;
  seekValue.value = null;
  seekPercent(value, true);
}

function beginSeek() {
  pointerSeeking = true;
  pointerCommitDone = false;
  seekValue.value = player.progress;
}

function endSeek() {
  if (!pointerSeeking) return;
  pointerSeeking = false;
  if (pointerCommitDone) return;
  const value = seekValue.value;
  if (value === null) return;
  pointerCommitDone = true;
  seekValue.value = null;
  seekPercent(value, true);
}

function syncLyrics() {
  const positionMs = currentTime() * 1000;
  lyrics.sync(positionMs / 1000);
  updateWordProgress(positionMs);
}

// Cache active word elements + their (line, wordIndex) so we don't re-query
// the DOM every frame. Invalidated when the active set changes, when a
// virtualized row is remounted, or when the lyric data itself changes.
type CachedWord = { el: HTMLElement; rt: HTMLElement | null; line: LyricLine; wordIndex: number };
let cachedWords: CachedWord[] | null = null;
let cachedSet: Set<number> | null = null;
let wordCacheDirty = true;
let wordCacheRetryAt = 0;
const wordProgressCache = new WeakMap<HTMLElement, number>();

function invalidateWordCache() {
  cachedWords = null;
  cachedSet = null;
  wordCacheDirty = true;
  wordCacheRetryAt = 0;
}

function rebuildWordCache(container: HTMLElement) {
  const words: CachedWord[] = [];
  let hasMountedActiveLine = false;
  container.querySelectorAll<HTMLElement>('[data-lyric-active="true"]').forEach((element) => {
    const lineIndex = Number(element.dataset.lyricIndex);
    const line = lyrics.lines[lineIndex];
    if (!line) return;
    hasMountedActiveLine = true;
    // Lines without genuine per-word timestamps have no word-by-word sweep to
    // run; they are highlighted as a whole line instead (--word-progress stays
    // at its initial 0%, so the gradient resolves to --word-rest everywhere).
    if (!wordTimestampLines.value.has(lineIndex)) return;
    // Parsed lyrics are immutable; unwrap the reactive proxy once so the
    // update loop does not pay a proxy `get` trap for every word.
    const raw = toRaw(line);
    element.querySelectorAll<HTMLElement>(".lyric-word").forEach((word, wordIndex) => {
      words.push({ el: word, rt: word.closest("ruby")?.querySelector<HTMLElement>("rt") ?? null, line: raw, wordIndex });
    });
  });
  cachedWords = words;
  cachedSet = activeSet.value;
  // Retry only when there are active lines mounted but none of them carry
  // word timestamps yet; otherwise stop polling the DOM.
  wordCacheDirty = hasMountedActiveLine && words.length === 0;
  wordCacheRetryAt = wordCacheDirty ? performance.now() + WORD_CACHE_RETRY_MS : 0;
}

function updateWordProgress(positionMs: number) {
  const container = lyricsScroll.value;
  if (!container) return;
  const set = activeSet.value;
  if (set.size === 0) {
    if (cachedWords) invalidateWordCache();
    return;
  }
  const now = performance.now();
  if (
    !cachedWords ||
    cachedSet !== set ||
    (cachedWords.length > 0 && !cachedWords[0].el.isConnected) ||
    (wordCacheDirty && now >= wordCacheRetryAt)
  ) {
    rebuildWordCache(container);
  }
  if (!cachedWords?.length) return;
  for (const w of cachedWords) {
    const p = lyrics.wordProgress(w.line, w.wordIndex, positionMs);
    const prev = wordProgressCache.get(w.el);
    if (prev !== undefined && Math.abs(p - prev) < WORD_PROGRESS_EPSILON) continue;
    wordProgressCache.set(w.el, p);
    const progress = `${p * 100}%`;
    w.el.style.setProperty("--word-progress", progress);
    w.rt?.style.setProperty("--word-progress", progress);
  }
}

/** Refresh the active words once the virtualizer has settled (paused seek). */
function scheduleWordRefresh() {
  if (running) return;
  window.clearTimeout(settleTimer);
  settleTimer = window.setTimeout(() => {
    settleTimer = 0;
    updateWordProgress(currentTime() * 1000);
  }, 120);
}

function loop() {
  if (!running) return;
  frame = requestAnimationFrame(loop);
  const now = performance.now();
  const positionMs = currentTime() * 1000;
  // `lyrics.sync` stays throttled (it re-derives the active set), but the
  // per-word gradient is rewritten every frame so the sweep tracks the beat
  // instead of stepping at ~30 Hz. The epsilon in updateWordProgress already
  // skips DOM writes when a word barely changed.
  if (now - lastLyricsSync >= LYRICS_TICK_MS) {
    lastLyricsSync = now;
    lyrics.sync(positionMs / 1000);
  }
  updateWordProgress(positionMs);
}

function startLoop() {
  if (running || document.hidden) return;
  running = true;
  lastLyricsSync = 0;
  frame = requestAnimationFrame(loop);
}

function stopLoop() {
  running = false;
  if (frame) {
    cancelAnimationFrame(frame);
    frame = 0;
  }
}

function onVisibilityChange() {
  if (document.hidden) stopLoop();
  else if (player.isPlaying) startLoop();
}

/**
 * Anchor the active line at {@link ACTIVE_LINE_RATIO} of the viewport. Smooth
 * scrolling is only used for short hops: animating across hundreds of rows
 * mounts/unmounts the whole window and defeats virtualization.
 */
function followActiveLine(index: number) {
  const handle = lyricsVirtualizer.value;
  if (!handle || index < 0) return;
  // A line change and a lyric refresh can both ask for the same anchor in one
  // flush; don't restart the smooth scroll that is already in flight.
  if (programmaticScroll && followTarget === index) return;
  const viewport = handle.viewportSize;
  const target = handle.getItemOffset(index) - viewport * ACTIVE_LINE_RATIO;
  if (Math.abs(target - handle.scrollOffset) < 1) return;
  window.clearTimeout(programmaticTimer);
  programmaticScroll = true;
  followTarget = index;
  const smooth = Math.abs(target - handle.scrollOffset) < Math.max(viewport, 1) * 1.5;
  handle.scrollToIndex(index, { align: "start", smooth, offset: -viewport * ACTIVE_LINE_RATIO });
  programmaticTimer = window.setTimeout(() => {
    programmaticScroll = false;
    followTarget = -1;
    programmaticTimer = 0;
  }, 800);
}

/** Jump back to the start of a freshly loaded lyric sheet. */
function resetScroll() {
  const handle = lyricsVirtualizer.value;
  if (!handle || handle.scrollOffset === 0) return;
  window.clearTimeout(programmaticTimer);
  programmaticScroll = true;
  followTarget = -1;
  handle.scrollTo(0);
  programmaticTimer = window.setTimeout(() => {
    programmaticScroll = false;
    programmaticTimer = 0;
  }, 800);
}

/** Clear the manual-scroll hold and cancel its pending resume. */
function clearManualResume() {
  window.clearTimeout(manualTimer);
  manualTimer = 0;
  manualDeadline = 0;
  manualScroll = false;
  followTarget = -1;
}

function releaseManualScroll() {
  manualTimer = 0;
  const remaining = manualDeadline - performance.now();
  if (remaining > 0) {
    manualTimer = window.setTimeout(releaseManualScroll, remaining);
    return;
  }
  manualDeadline = 0;
  manualScroll = false;
  if (lyrics.activeIndex >= 0) {
    lastScrolledIndex = lyrics.activeIndex;
    followActiveLine(lyrics.activeIndex);
  }
}

/** Suspend auto-follow while the user reads ahead, then glide back. */
function holdManualScroll() {
  manualScroll = true;
  followTarget = -1;
  manualDeadline = performance.now() + MANUAL_RESUME_MS;
  if (!manualTimer) manualTimer = window.setTimeout(releaseManualScroll, MANUAL_RESUME_MS);
}

function updateLyricEdgeFlags() {
  const handle = lyricsVirtualizer.value;
  if (!handle) return;
  const offset = handle.scrollOffset;
  const viewport = handle.viewportSize;
  const total = handle.scrollSize;
  lyricAtTop.value = offset <= 1;
  lyricAtBottom.value = offset + viewport >= total - 1;
}

function onVirtualScroll() {
  if (programmaticScroll) return;
  updateLyricEdgeFlags();
  holdManualScroll();
}

function onVirtualScrollEnd() {
  // Rows mount/unmount while scrolling, so cached word elements may belong to
  // a detached instance; refresh them on the next tick.
  wordCacheDirty = true;
  wordCacheRetryAt = 0;
  updateLyricEdgeFlags();
  if (programmaticScroll) {
    programmaticScroll = false;
    followTarget = -1;
    window.clearTimeout(programmaticTimer);
    programmaticTimer = 0;
  }
  scheduleWordRefresh();
}

onMounted(() => {
  if (player.isPlaying) startLoop();
  else syncLyrics();
  bumpControls();
  window.setTimeout(updateLyricEdgeFlags, 100);
  void syncFullscreen();
  window.addEventListener("pointermove", bumpControls);
  window.addEventListener("pointerdown", bumpControls);
  window.addEventListener("wheel", bumpControls, { passive: true });
  window.addEventListener("resize", scheduleSyncFullscreen);
  document.addEventListener("visibilitychange", onVisibilityChange);
});
onBeforeUnmount(() => {
  stopLoop();
  clearManualResume();
  window.clearTimeout(controlsTimer);
  window.clearTimeout(programmaticTimer);
  window.clearTimeout(fullscreenTimer);
  window.clearTimeout(settleTimer);
  fullscreenRequest += 1;
  cachedWords = null;
  cachedSet = null;
  window.removeEventListener("pointermove", bumpControls);
  window.removeEventListener("pointerdown", bumpControls);
  window.removeEventListener("wheel", bumpControls);
  window.removeEventListener("resize", scheduleSyncFullscreen);
  document.removeEventListener("visibilitychange", onVisibilityChange);
});

watch(
  () => {
    const track = player.currentTrack;
    return track?.path ? `${track.sourceId ?? ""}::${track.path}` : "";
  },
  () => {
    invalidateWordCache();
    clearManualResume();
    lastScrolledIndex = -1;
    seekValue.value = null;
    void lyrics
      .loadForTrack(player.currentTrack ?? undefined)
      .then(() => {
        // Re-sync on the early-return path too (same track, already loaded):
        // the virtualizer is freshly mounted and still needs to find the line.
        syncLyrics();
        const index = lyrics.activeIndex;
        if (index >= 0 && index !== lastScrolledIndex) {
          lastScrolledIndex = index;
          followActiveLine(index);
        }
      })
      .catch(() => {});
  },
  { immediate: true },
);

watch(
  () => lyrics.lines,
  () => {
    invalidateWordCache();
    clearManualResume();
    syncLyrics();
    const index = lyrics.activeIndex;
    lastScrolledIndex = index;
    if (index >= 0) followActiveLine(index);
    else resetScroll();
  },
  { flush: "post" },
);

watch(
  () => lyrics.activeIndex,
  (index) => {
    if (index < 0 || index === lastScrolledIndex) return;
    lastScrolledIndex = index;
    if (!manualScroll) followActiveLine(index);
  },
  { flush: "post" },
);

watch(
  activeKey,
  () => {
    invalidateWordCache();
    updateWordProgress(currentTime() * 1000);
  },
  { flush: "post" },
);

// Toggling ruby replaces the word spans, so the cached element references are
// no longer the ones painted on screen.
watch(
  showRuby,
  () => {
    invalidateWordCache();
    updateWordProgress(currentTime() * 1000);
  },
  { flush: "post" },
);

watch(
  () => player.isPlaying,
  (playing) => {
    if (playing) startLoop();
    else {
      stopLoop();
      syncLyrics();
    }
  },
);

// While paused the animation loop is stopped, so a seek/resume still needs a
// one-shot lyric sync to move the highlight.
watch(
  () => player.position,
  () => {
    if (!player.isPlaying) syncLyrics();
  },
);
</script>

<template>
  <div v-if="player.currentTrack" class="fixed inset-0 z-50 flex h-screen flex-col overflow-hidden bg-bg text-fg">
    <div class="pointer-events-none absolute inset-0 opacity-20" :style="{ background: `radial-gradient(circle at 20% 20%, ${player.currentTrack.color}, transparent 55%), radial-gradient(circle at 80% 80%, ${player.currentTrack.color}, transparent 55%)` }"></div>
    <div class="pointer-events-none absolute -right-32 top-1/2 h-[150%] w-40 rotate-[18deg] bg-accent/10"></div>
    <div class="pointer-events-none absolute -left-24 bottom-0 h-[60%] w-24 -rotate-[18deg] bg-accent/5"></div>
    <header class="pointer-events-none absolute inset-x-0 top-[env(safe-area-inset-top)] z-20 h-16 px-4 md:hidden">
      <div class="flex h-full min-w-0 items-center gap-3">
        <span
          class="ak-frame grid h-10 w-10 shrink-0 place-items-center overflow-hidden text-sm font-black text-white/90"
          :style="{ backgroundColor: player.currentTrack.color }"
        >
          <img v-if="player.currentTrack.cover" :src="player.currentTrack.cover" alt="" decoding="async" class="h-full w-full object-cover" />
          <LoaderCircle v-else-if="coverPending(player.currentTrack)" :size="16" :stroke-width="2" class="animate-spin" />
          <template v-else>{{ initial(player.currentTrack) }}</template>
        </span>
        <span class="grid min-w-0 gap-0.5">
          <strong class="truncate text-sm font-semibold leading-tight">{{ player.currentTrack.title }}</strong>
          <button
            v-if="player.currentTrack.artist"
            type="button"
            class="truncate text-left text-xs leading-tight text-dim transition-colors hover:text-accent"
            :title="t('library.openArtist')"
            @click="emit('openArtist', player.currentTrack.artist)"
          >
            {{ player.currentTrack.artist }}
          </button>
        </span>
      </div>
    </header>

    <div class="pointer-events-none absolute right-8 top-[calc(env(safe-area-inset-top)+2rem)] z-30">
      <div class="flex items-center gap-4 transition-all duration-300 ease-out" :class="revealClass">
        <button
          type="button"
          class="grid h-10 w-10 shrink-0 place-items-center border border-line bg-bg/60 text-muted backdrop-blur transition-all duration-300 ease-out hover:border-fg hover:text-fg"
          :title="t('controls.return')"
          :aria-label="t('controls.return')"
          @click="emit('navigate', returnView)"
        >
          <ChevronDown :size="16" :stroke-width="2" />
        </button>
        <button
          type="button"
          class="grid h-10 w-10 shrink-0 place-items-center border border-line bg-bg/60 text-muted backdrop-blur transition-all duration-300 ease-out hover:border-fg hover:text-fg"
          :title="fullscreenTitle"
          :aria-label="fullscreenTitle"
          :aria-pressed="isFullscreen"
          @click="toggleFullscreen()"
        >
          <Minimize2 v-if="isFullscreen" :size="16" :stroke-width="2" />
          <Maximize2 v-else :size="16" :stroke-width="2" />
        </button>
      </div>
    </div>
    <div class="relative z-10 grid min-h-0 flex-1 grid-rows-[minmax(0,1fr)_auto] gap-6 overflow-hidden px-5 pt-[env(safe-area-inset-top)] py-4 md:grid-cols-[minmax(220px,0.34fr)_minmax(0,0.66fr)] md:grid-rows-1 md:gap-12 md:px-10 md:py-8 lg:grid-cols-[minmax(280px,0.32fr)_minmax(0,0.68fr)] lg:gap-16 lg:px-16">
      <div class="amll-controls order-2 flex min-h-0 flex-col justify-end md:order-none md:justify-center" :data-collapsed="controlsVisible ? 'false' : 'true'">
        <div class="ak-frame mx-auto hidden aspect-square w-full max-w-[420px] overflow-hidden md:block" :style="{ backgroundColor: player.currentTrack.color }">
          <img v-if="player.currentTrack.cover" :src="player.currentTrack.cover" alt="" decoding="async" class="h-full w-full object-cover" />
          <span v-else-if="coverPending(player.currentTrack)" class="grid h-full w-full place-items-center text-white/90"><LoaderCircle :size="40" :stroke-width="1.8" class="animate-spin" /></span>
          <span v-else-if="player.currentTrack.metaLoaded" class="grid h-full w-full place-items-center text-8xl font-black text-white/90">{{ initial(player.currentTrack) }}</span>
        </div>
        <div class="mx-auto mt-4 hidden w-full max-w-[420px] md:block">
          <h1 class="mt-3 text-2xl font-black leading-none tracking-tight lg:text-3xl">{{ player.currentTrack.title }}</h1>
          <button v-if="player.currentTrack.artist" type="button" class="mt-2 block w-fit max-w-full truncate text-left text-base text-dim transition-colors hover:text-accent" :title="t('library.openArtist')" @click="emit('openArtist', player.currentTrack.artist)">{{ player.currentTrack.artist }}</button>
        </div>
        <div class="mx-auto w-full max-w-[420px] md:mt-4">
          <div class="flex h-6 items-center">
            <input
              :value="sliderValue"
              :style="{ '--fill': percent(sliderValue), '--buffered': percent(player.bufferedProgress) }"
              class="ak-slider"
              type="range"
              min="0"
              max="100"
              aria-label="Playback position"
              @pointerdown="beginSeek"
              @input="onSeek"
              @change="commitSeek"
              @pointerup="endSeek"
              @pointercancel="endSeek"
              @lostpointercapture="endSeek"
            />
          </div>
          <div class="mt-1.5 flex items-center justify-between font-mono text-[13px] tabular-nums text-muted">
            <span>{{ elapsedDisplay }}</span>
            <span v-if="isHiRes" class="select-none text-[12px] tracking-wide">Hi-Res</span>
            <span v-else></span>
            <button type="button" class="transition-colors hover:text-fg" :aria-pressed="showRemaining" @click="showRemaining = !showRemaining">
              <template v-if="showRemaining">-{{ remainingDisplay }}</template>
              <template v-else>{{ player.totalTime }}</template>
            </button>
          </div>
          <div class="mt-3 flex items-center justify-between gap-3">
            <button class="grid h-9 w-9 place-items-center" :class="player.shuffle ? 'text-accent' : 'text-muted'" :title="shuffleTitle" @click="player.toggleShuffle()">
              <Shuffle :size="16" :stroke-width="1.8" />
            </button>
            <div class="flex items-center gap-2">
              <button class="grid h-10 w-10 place-items-center text-fg transition-colors hover:text-accent" @click="player.previous()">
                <SkipBack :size="18" :stroke-width="2" />
              </button>
              <button class="ak-clip grid h-11 w-11 place-items-center bg-accent text-accent-fg transition-transform hover:scale-[1.03] active:scale-95" :aria-label="player.isPlaying ? t('controls.pause') : t('controls.play')" @click="player.togglePlayback()">
                <Pause v-if="player.isPlaying" :size="20" :stroke-width="2.2" />
                <Play v-else :size="20" :stroke-width="2.2" />
              </button>
              <button class="grid h-10 w-10 place-items-center text-fg transition-colors hover:text-accent" @click="player.next()">
                <SkipForward :size="18" :stroke-width="2" />
              </button>
            </div>
            <button class="grid h-9 w-9 place-items-center" :class="player.repeat !== 'off' ? 'text-accent' : 'text-muted'" :title="repeatTitle" @click="player.cycleRepeat()">
              <Repeat1 v-if="player.repeat === 'one'" :size="16" :stroke-width="1.8" />
              <Repeat v-else :size="16" :stroke-width="1.8" />
            </button>
          </div>
        </div>
      </div>
      <div class="order-1 flex min-h-0 min-w-0 flex-col justify-center pt-16 md:order-none md:pt-0">
         <div ref="lyricsScroll" class="lyrics-scroll min-h-0 flex-1 overflow-y-auto pl-6 md:max-h-[76vh] md:border-l md:border-line md:pl-10" :class="{ 'lyric-at-top': lyricAtTop, 'lyric-at-bottom': lyricAtBottom }" :style="{ '--line-size': `${lyrics.classic.lineSize}px`, '--translation-size': `${lyrics.classic.translationSize}px`, '--line-spacing': `${lyrics.classic.lineSpacing}px`, '--line-weight': lyrics.classic.fontWeight, '--line-weight-active': lyricActiveWeight, fontFamily: lyricFontFamily }">
          <Virtualizer
            ref="lyricsVirtualizer"
            :data="lyrics.lines"
            :buffer-size="400"
            :item-size="lyricItemSize"
            @scroll="onVirtualScroll"
            @scroll-end="onVirtualScrollEnd"
          >
            <template #default="{ item: line, index: lineIndex }">
              <div class="lyric-line" :class="{ 'is-active': activeSet.has(lineIndex), 'text-muted': !activeSet.has(lineIndex), 'is-bg': line.isBG, 'is-duet': line.isDuet }" :data-lyric-active="activeSet.has(lineIndex)" :data-lyric-primary="lineIndex === lyrics.activeIndex" :data-lyric-index="lineIndex" :data-lyric-word-timestamps="wordTimestampLines.has(lineIndex)">
                <p class="lyric-primary">
                  <template v-for="(word, wordIndex) in line.words" :key="wordIndex">
                    <ruby v-if="showRuby && word.ruby?.length" class="lyric-ruby">
                      <span class="lyric-word">{{ word.word }}</span>
                      <rt class="lyric-ruby-text">{{ rubyText(word) }}</rt>
                    </ruby>
                    <span v-else class="lyric-word">{{ word.word }}</span>
                  </template>
                </p>
                <p v-if="lyrics.classic.translate && line.translatedLyric" class="lyric-translation">{{ line.translatedLyric }}</p>
                <p v-if="line.romanLyric" class="lyric-roman">{{ line.romanLyric }}</p>
              </div>
            </template>
          </Virtualizer>
          <p v-if="lyrics.status === 'loading'" class="text-muted px-0 py-4">{{ t("lyrics.loading") }}</p>
          <p v-else-if="!lyrics.hasLyrics && !lyrics.suppressed" class="text-muted px-0 py-4">{{ t("lyrics.empty") }}</p>
        </div>
      </div>
    </div>
    <div class="player-waveform pointer-events-none relative z-30 shrink-0 text-accent md:absolute md:inset-x-0 md:bottom-0">
      <RealtimeWaveform fit class="text-accent" />
    </div>
  </div>
</template>

<style scoped>
@property --word-progress {
  syntax: "<percentage>";
  inherits: false;
  initial-value: 0%;
}

.lyrics-scroll {
  scrollbar-width: none;
  touch-action: pan-y;
  overscroll-behavior: contain;
  --lyric-ease: cubic-bezier(0.22, 1, 0.36, 1);
  --lyric-duration: 800ms;
  -webkit-mask-image: linear-gradient(to bottom, transparent 0%, #000 5%, #000 95%, transparent 100%);
  mask-image: linear-gradient(to bottom, transparent 0%, #000 5%, #000 95%, transparent 100%);
  -webkit-mask-repeat: no-repeat;
  mask-repeat: no-repeat;
  -webkit-mask-size: 100% 100%;
  mask-size: 100% 100%;
}

.lyrics-scroll.lyric-at-top {
  -webkit-mask-image: linear-gradient(to bottom, #000 95%, transparent 100%);
  mask-image: linear-gradient(to bottom, #000 95%, transparent 100%);
}

.lyrics-scroll.lyric-at-bottom {
  -webkit-mask-image: linear-gradient(to bottom, transparent 0%, #000 5%);
  mask-image: linear-gradient(to bottom, transparent 0%, #000 5%);
}

.lyrics-scroll.lyric-at-top.lyric-at-bottom {
  -webkit-mask-image: none;
  mask-image: none;
}

.lyrics-scroll::-webkit-scrollbar {
  display: none;
}

.lyric-line {
  opacity: 0.72;
  transition: opacity var(--lyric-duration) var(--lyric-ease);
  margin-bottom: var(--line-spacing, 16px);
  contain: layout paint style;
}

.lyric-line.is-active {
  opacity: 1;
}

.lyric-primary {
  font-size: var(--line-size, 24px);
  font-weight: var(--line-weight, 600);
  line-height: 1.4;
}

.lyric-line.is-active .lyric-primary {
  color: var(--fg);
  font-weight: var(--line-weight-active, 800);
}

/*
 * Only the active line needs the per-word sweep; inactive lines are flat text.
 * Painting the gradient only where progress actually changes keeps most of the
 * visible lyrics off the `background-clip: text` path.
 */
.lyric-word {
  display: inline;
  color: var(--muted);
  -webkit-text-fill-color: currentColor;
}

.lyric-line.is-bg .lyric-word {
  color: var(--dim);
  -webkit-text-fill-color: currentColor;
}

.lyric-line.is-active .lyric-word {
  --word-sung: var(--accent);
  --word-rest: var(--fg);
  background: linear-gradient(90deg, var(--word-sung) var(--word-progress), var(--word-rest) var(--word-progress));
  -webkit-background-clip: text;
  background-clip: text;
  color: transparent;
  -webkit-text-fill-color: transparent;
}

/*
 * Lines that carry only a single line-level timestamp (no per-word timing)
 * have no word sweep to run: lock --word-progress at 100% so the whole line
 * paints in the accent ("sung") color instead of sitting flat at --word-rest.
 */
.lyric-line.is-active[data-lyric-word-timestamps="false"] .lyric-word {
  --word-progress: 100%;
}

.lyric-line.is-bg.is-active .lyric-word {
  --word-sung: var(--accent);
  --word-rest: var(--muted);
}

/*
 * Readings (注音 / ruby) sit above the base word and, like it, are painted by
 * the word-progress gradient so they fill in together as the line is sung.
 */
.lyric-ruby {
  ruby-position: over;
  ruby-align: center;
}

.lyric-ruby-text {
  font-size: 0.5em;
  font-weight: calc(var(--line-weight, 600) - 200);
  color: var(--muted);
  -webkit-text-fill-color: currentColor;
}

.lyric-line.is-bg .lyric-ruby-text {
  color: var(--dim);
  -webkit-text-fill-color: currentColor;
}

.lyric-line.is-active .lyric-ruby-text {
  --word-sung: var(--accent);
  --word-rest: var(--fg);
  background: linear-gradient(90deg, var(--word-sung) var(--word-progress), var(--word-rest) var(--word-progress));
  -webkit-background-clip: text;
  background-clip: text;
  color: transparent;
  -webkit-text-fill-color: transparent;
}

.lyric-line.is-bg.is-active .lyric-ruby-text {
  --word-sung: var(--accent);
  --word-rest: var(--muted);
}

.lyric-translation {
  margin-top: 0.35rem;
  font-size: var(--translation-size, 18px);
  line-height: 1.5;
  color: var(--dim);
}

.lyric-roman {
  margin-top: 0.2rem;
  font-size: calc(var(--translation-size, 18px) * 0.9);
  line-height: 1.45;
  color: var(--dim);
  opacity: 0.85;
}

/* Duet counter-lines (the second voice) hug the right edge, like AMLL. */
.lyric-line.is-duet {
  text-align: right;
}

/* Background vocals read as a smaller, dimmer sub-line under the main one. */
.lyric-line.is-bg {
  margin-bottom: calc(var(--line-spacing, 16px) * 0.5);
  opacity: 0.6;
}

.lyric-line.is-bg .lyric-primary {
  font-size: calc(var(--line-size, 24px) * 0.68);
  font-weight: calc(var(--line-weight, 600) - 100);
}

.lyric-line.is-bg .lyric-translation {
  font-size: calc(var(--translation-size, 18px) * 0.85);
}

.lyric-line.is-bg.is-active {
  opacity: 0.85;
}

.lyric-line.is-bg.is-active .lyric-primary {
  font-weight: calc(var(--line-weight-active, 800) - 200);
}

@media (min-width: 640px) {
  .lyric-primary {
    font-size: var(--line-size, 24px);
  }
}

/*
 * Mobile: the transport controls own a region below the lyrics. When they
 * auto-hide the region collapses to zero height, so the lyrics reclaim the
 * space instead of leaving a blank strip. A grid-row `0fr` collapse does NOT
 * work here because the container's height is auto inside a flex column, so
 * animate `max-height` instead.
 */
.amll-controls {
  max-height: 230px;
  overflow: hidden;
  transition: max-height 320ms cubic-bezier(0.2, 0.8, 0.2, 1), opacity 220ms ease;
}

.amll-controls[data-collapsed="true"] {
  max-height: 0;
  opacity: 0;
  pointer-events: none;
}

@media (min-width: 768px) {
  .amll-controls,
  .amll-controls[data-collapsed="true"] {
    max-height: none;
    overflow: visible;
    opacity: 1;
    pointer-events: auto;
  }
}

/* A touch thicker than the compact PlayerBar slider so it reads clearly on the
   full-screen player, while keeping the same flat, square-cornered style. */
.ak-slider {
  height: 5px;
}

.ak-slider:hover,
.ak-slider:focus-visible {
  height: 7px;
}
</style>
