<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { ChevronDown, LoaderCircle, Maximize2, Minimize2 } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { usePlayerStore } from "../stores/player";
import { useLyricsStore } from "../stores/lyrics";
import { currentTime } from "../lib/audio";
import { coverPending, initial } from "../lib/format";
import { cssFontFamily } from "../lib/fonts";
import { useWakeLock } from "../composables/useWakeLock";
import PlayerBar from "../components/PlayerBar.vue";
import type { View } from "../lib/app";

defineProps<{ returnView: View }>();
const emit = defineEmits<{ navigate: [view: View]; queue: []; openArtist: [artist: string]; openAlbum: [album: string] }>();
const { t } = useI18n();
const player = usePlayerStore();
const lyrics = useLyricsStore();

/** Keep the display on for as long as this full-screen player is open. */
useWakeLock();

const lyricsScroll = ref<HTMLElement | null>(null);
const lyricsTrack = ref<HTMLElement | null>(null);
const scrollOffset = ref(0);
const manualScroll = ref(false);
let lastScrolledIndex = -1;
let frame = 0;
let lastLyricsSync = 0;
let manualTimer = 0;

/** CSS `font-family` value for the standard lyrics renderer, if configured. */
const lyricFontFamily = computed(() => cssFontFamily(lyrics.classic.fontFamilies));
/** Active lines stay a touch heavier than the configured base weight. */
const lyricActiveWeight = computed(() => Math.min(900, lyrics.classic.fontWeight + 200));
/** Whether the per-word readings (注音 / ruby) carried by the lyrics are shown. */
const showRuby = computed(() => lyrics.classic.ruby);

/** Join a word's ruby spans into the single label shown above it. */
function rubyText(word: { ruby?: Array<{ word: string }> }): string {
  return (word.ruby ?? []).map((span) => span.word).join("");
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

const controlsVisible = ref(true);
const isFullscreen = ref(false);
let controlsTimer = 0;

function bumpControls() {
  controlsVisible.value = true;
  window.clearTimeout(controlsTimer);
  controlsTimer = window.setTimeout(() => { controlsVisible.value = false; }, 1000);
}

const revealClass = computed(() => (controlsVisible.value
  ? "pointer-events-auto translate-y-0 opacity-100"
  : "pointer-events-none -translate-y-2 opacity-0"));

async function syncFullscreen() {
  try {
    isFullscreen.value = await getCurrentWindow().isFullscreen();
  } catch {
    isFullscreen.value = Boolean(document.fullscreenElement);
  }
}

async function toggleFullscreen() {
  try {
    await getCurrentWindow().setFullscreen(!isFullscreen.value);
    await syncFullscreen();
  } catch {
    if (document.fullscreenElement) await document.exitFullscreen();
    else await document.documentElement.requestFullscreen();
  }
}

const fullscreenTitle = computed(() => (isFullscreen.value ? t("controls.exitFullscreen") : t("controls.fullscreen")));

function syncLyrics() {
  const positionMs = currentTime() * 1000;
  lyrics.sync(positionMs / 1000);
  updateWordProgress(positionMs);
}

function updateWordProgress(positionMs: number) {
  const container = lyricsScroll.value;
  if (!container || activeSet.value.size === 0) return;
  container.querySelectorAll<HTMLElement>('[data-lyric-active="true"]').forEach((element) => {
    const line = lyrics.lines[Number(element.dataset.lyricIndex)];
    if (!line) return;
    element.querySelectorAll<HTMLElement>(".lyric-word").forEach((word, wordIndex) => {
      const progress = `${lyrics.wordProgress(line, wordIndex, positionMs) * 100}%`;
      word.style.setProperty("--word-progress", progress);
      // The reading is a sibling of the base word and `--word-progress` does not
      // inherit, so write the same sweep onto it explicitly to keep them in step.
      word
        .closest("ruby")
        ?.querySelector<HTMLElement>("rt")
        ?.style.setProperty("--word-progress", progress);
    });
  });
}

function loop() {
  const now = performance.now();
  if (now - lastLyricsSync >= 33) {
    lastLyricsSync = now;
    const positionMs = currentTime() * 1000;
    lyrics.sync(positionMs / 1000);
  }
  updateWordProgress(currentTime() * 1000);
  frame = requestAnimationFrame(loop);
}

/** Fraction of the viewport height at which the active line sits. */
const ACTIVE_LINE_RATIO = 0.35;

/** Vertical offset (px) that puts the active line at 35% of the viewport. */
function activeLineOffset(): number {
  const container = lyricsScroll.value;
  const track = lyricsTrack.value;
  if (!container || !track) return scrollOffset.value;
  // Anchor on the whole active block, not just the primary line: background
  // vocals and duet counter-lines can sit several lines away, and anchoring on
  // the primary alone pushes them off screen.
  const active = Array.from(container.querySelectorAll<HTMLElement>('[data-lyric-active="true"]'));
  if (active.length === 0) return scrollOffset.value;
  let top = Number.POSITIVE_INFINITY;
  let bottom = Number.NEGATIVE_INFINITY;
  for (const element of active) {
    top = Math.min(top, element.offsetTop);
    bottom = Math.max(bottom, element.offsetTop + element.offsetHeight);
  }
  const maxOffset = Math.max(0, track.offsetHeight - container.clientHeight);
  const target = (top + bottom) / 2 - container.clientHeight * ACTIVE_LINE_RATIO;
  return Math.min(maxOffset, Math.max(0, target));
}

/** Maximum scroll offset (px) before the track would leave the viewport. */
function scrollLimit(): number {
  const container = lyricsScroll.value;
  const track = lyricsTrack.value;
  if (!container || !track) return 0;
  return Math.max(0, track.offsetHeight - container.clientHeight);
}

/** Clamp an offset to the valid scroll range. */
function clampOffset(value: number): number {
  return Math.min(scrollLimit(), Math.max(0, value));
}

/** Suspend auto-scroll and schedule the return to the active line. */
function holdManualScroll(resumeMs: number) {
  manualScroll.value = true;
  window.clearTimeout(manualTimer);
  manualTimer = window.setTimeout(() => { manualScroll.value = false; }, resumeMs);
}

function onWheel(event: WheelEvent) {
  if (!lyricsScroll.value || !lyricsTrack.value) return;
  holdManualScroll(160);
  scrollOffset.value = clampOffset(scrollOffset.value + event.deltaY);
}

/**
 * Touch (and pen) drag scrolling with release inertia. The track is moved
 * through a CSS transform, so the browser never scrolls the overflow-hidden
 * container itself and there is no native momentum to piggyback on — the
 * pointer stream and the glide that follows a flick are tracked by hand.
 * `touch-action: none` on `.lyrics-scroll` keeps the browser from hijacking
 * the gesture and cancelling our pointer events.
 */
let touchActive = false;
let touchStartY = 0;
let touchStartOffset = 0;
let touchLastTime = 0;
let touchVelocity = 0;
let momentumFrame = 0;

const TOUCH_RESUME_MS = 2500;
/** Fraction of the velocity retained per 60fps frame; lower stops sooner. */
const TOUCH_FRICTION = 0.95;
/** Speeds (px/ms) below this are treated as stopped. */
const TOUCH_MIN_VELOCITY = 0.02;
/** Exponential smoothing applied to the sampled drag velocity. */
const TOUCH_VELOCITY_SMOOTHING = 0.7;
/** A release after this long without movement is treated as a hold, not a flick. */
const TOUCH_HOLD_MS = 120;

function cancelMomentum() {
  if (momentumFrame) {
    cancelAnimationFrame(momentumFrame);
    momentumFrame = 0;
  }
}

/** Glide to a stop after a flick, then hand control back to auto-scroll. */
function startMomentum() {
  let velocity = touchVelocity;
  if (Math.abs(velocity) < TOUCH_MIN_VELOCITY) {
    holdManualScroll(TOUCH_RESUME_MS);
    return;
  }

  // Stay in manual mode while gliding so the CSS transition does not fight the
  // per-frame transform, and keep the resume timer from firing mid-glide.
  manualScroll.value = true;
  window.clearTimeout(manualTimer);

  let last = performance.now();
  const step = (now: number) => {
    const dt = Math.min(now - last, 64);
    last = now;
    const raw = scrollOffset.value + velocity * dt;
    const clamped = clampOffset(raw);
    scrollOffset.value = clamped;
    if (clamped !== raw) {
      // Reached the top or bottom: stop cleanly rather than overshoot.
      momentumFrame = 0;
      holdManualScroll(TOUCH_RESUME_MS);
      return;
    }
    // Decay against real elapsed time so the glide feels the same at any fps.
    velocity *= Math.pow(TOUCH_FRICTION, dt / (1000 / 60));
    if (Math.abs(velocity) < TOUCH_MIN_VELOCITY) {
      momentumFrame = 0;
      holdManualScroll(TOUCH_RESUME_MS);
      return;
    }
    momentumFrame = requestAnimationFrame(step);
  };
  momentumFrame = requestAnimationFrame(step);
}

function onTouchStart(event: PointerEvent) {
  if (event.pointerType === "mouse") return;
  const container = lyricsScroll.value;
  if (!container) return;
  cancelMomentum();
  touchActive = true;
  touchStartY = event.clientY;
  touchStartOffset = scrollOffset.value;
  touchLastTime = performance.now();
  touchVelocity = 0;
  holdManualScroll(TOUCH_RESUME_MS);
  container.setPointerCapture(event.pointerId);
}

function onTouchMove(event: PointerEvent) {
  if (!touchActive) return;
  event.preventDefault();
  const now = performance.now();
  const delta = event.clientY - touchStartY;
  const next = clampOffset(touchStartOffset - delta);
  const dt = now - touchLastTime;
  if (dt > 0) {
    // Offset grows as the content moves up, so a flick upward keeps gliding up.
    const instantaneous = (next - scrollOffset.value) / dt;
    touchVelocity = touchVelocity * (1 - TOUCH_VELOCITY_SMOOTHING) + instantaneous * TOUCH_VELOCITY_SMOOTHING;
    touchLastTime = now;
  }
  scrollOffset.value = next;
  holdManualScroll(TOUCH_RESUME_MS);
}

function onTouchEnd(event: PointerEvent) {
  if (!touchActive) return;
  touchActive = false;
  const container = lyricsScroll.value;
  if (container?.hasPointerCapture(event.pointerId)) container.releasePointerCapture(event.pointerId);
  // Releasing after standing still should not fling the lyrics.
  if (performance.now() - touchLastTime > TOUCH_HOLD_MS) touchVelocity = 0;
  startMomentum();
}

onMounted(() => {
  frame = requestAnimationFrame(loop);
  bumpControls();
  void syncFullscreen();
  window.addEventListener("pointermove", bumpControls);
  window.addEventListener("pointerdown", bumpControls);
  window.addEventListener("wheel", bumpControls, { passive: true });
  window.addEventListener("resize", syncFullscreen);
});
onBeforeUnmount(() => {
  cancelAnimationFrame(frame);
  cancelMomentum();
  window.clearTimeout(manualTimer);
  window.clearTimeout(controlsTimer);
  window.removeEventListener("pointermove", bumpControls);
  window.removeEventListener("pointerdown", bumpControls);
  window.removeEventListener("wheel", bumpControls);
  window.removeEventListener("resize", syncFullscreen);
});

watch(
  () => {
    const track = player.currentTrack;
    return track?.path ? `${track.sourceId ?? ""}::${track.path}` : "";
  },
  () => {
    lastScrolledIndex = -1;
    scrollOffset.value = 0;
    void lyrics.loadForTrack(player.currentTrack ?? undefined).then(syncLyrics);
  },
  { immediate: true },
);
watch(
  () => lyrics.activeIndex,
  (index) => {
    if (index < 0 || index === lastScrolledIndex) return;
    lastScrolledIndex = index;
    // Don't yank the view back to the active line while the user is dragging.
    if (manualScroll.value) return;
    void nextTick(() => { scrollOffset.value = activeLineOffset(); });
  },
  { flush: "pre" },
);

/** Once a manual drag settles, glide back to the active line. */
watch(manualScroll, (manual) => {
  if (!manual) scrollOffset.value = activeLineOffset();
});

/**
 * The animation loop only writes `--word-progress` for elements that are
 * already marked active, so a line entering the active set would otherwise
 * paint one frame with a stale (or initial) progress. `flush: "post"` writes
 * the real progress right after the DOM patch, so the first painted frame is
 * already correct.
 */
watch(
  activeSet,
  () => {
    updateWordProgress(currentTime() * 1000);
    // A new line joining the block (or an old one leaving) moves the anchor.
    if (!manualScroll.value) scrollOffset.value = activeLineOffset();
  },
  { flush: "post" },
);
</script>

<template>
  <div v-if="player.currentTrack" class="fixed inset-0 z-50 flex h-screen flex-col overflow-hidden bg-bg text-fg">
    <div class="pointer-events-none absolute inset-0 opacity-20" :style="{ background: `radial-gradient(circle at 20% 20%, ${player.currentTrack.color}, transparent 55%), radial-gradient(circle at 80% 80%, ${player.currentTrack.color}, transparent 55%)` }"></div>
    <div class="pointer-events-none absolute -right-32 top-1/2 h-[150%] w-40 rotate-[18deg] bg-accent/10"></div>
    <div class="pointer-events-none absolute -left-24 bottom-0 h-[60%] w-24 -rotate-[18deg] bg-accent/5"></div>
    <header class="relative z-10 flex h-16 shrink-0 items-center px-5">
      <div class="ml-auto flex items-center gap-3">
        <button
          type="button"
          class="grid h-10 w-10 place-items-center border border-line text-muted transition-all duration-300 ease-out hover:border-fg hover:text-fg"
          :class="revealClass"
          :title="t('controls.return')"
          :aria-label="t('controls.return')"
          @click="emit('navigate', returnView)"
        >
          <ChevronDown :size="16" :stroke-width="2" />
        </button>
        <button
          type="button"
          class="grid h-10 w-10 place-items-center border border-line text-muted transition-all duration-300 ease-out hover:border-fg hover:text-fg"
          :class="revealClass"
          :title="fullscreenTitle"
          :aria-label="fullscreenTitle"
          :aria-pressed="isFullscreen"
          @click="toggleFullscreen()"
        >
          <Minimize2 v-if="isFullscreen" :size="16" :stroke-width="2" />
          <Maximize2 v-else :size="16" :stroke-width="2" />
        </button>
      </div>
    </header>
    <div class="relative z-10 grid min-h-0 flex-1 gap-6 overflow-hidden px-5 py-4 md:grid-cols-[minmax(220px,0.34fr)_minmax(0,0.66fr)] md:gap-12 md:px-10 md:py-8 lg:grid-cols-[minmax(280px,0.32fr)_minmax(0,0.68fr)] lg:gap-16 lg:px-16">
      <div class="hidden min-h-0 flex-col justify-center md:flex">
        <div class="ak-frame mx-auto aspect-square w-full max-w-[420px] overflow-hidden" :style="{ backgroundColor: player.currentTrack.color }">
          <img v-if="player.currentTrack.cover" :src="player.currentTrack.cover" alt="" decoding="async" class="h-full w-full object-cover" />
          <span v-else-if="coverPending(player.currentTrack)" class="grid h-full w-full place-items-center text-white/90"><LoaderCircle :size="40" :stroke-width="1.8" class="animate-spin" /></span>
          <span v-else-if="player.currentTrack.metaLoaded" class="grid h-full w-full place-items-center text-8xl font-black text-white/90">{{ initial(player.currentTrack) }}</span>
        </div>
        <div class="mx-auto mt-6 w-full max-w-[420px]">
          <h1 class="mt-3 text-2xl font-black leading-none tracking-tight lg:text-3xl">{{ player.currentTrack.title }}</h1>
          <button v-if="player.currentTrack.album" type="button" class="mt-4 block w-fit max-w-full truncate text-left text-lg text-muted transition-colors hover:text-accent" :title="t('library.openAlbum')" @click="emit('openAlbum', player.currentTrack.album)">{{ player.currentTrack.album }}</button>
          <button v-if="player.currentTrack.artist" type="button" class="mt-2 block w-fit max-w-full truncate text-left text-base text-dim transition-colors hover:text-accent" :title="t('library.openArtist')" @click="emit('openArtist', player.currentTrack.artist)">{{ player.currentTrack.artist }}</button>
        </div>
      </div>
      <div class="flex min-h-0 min-w-0 flex-col justify-center">
        <div class="mb-5 md:hidden">
          <h1 class="mt-2 truncate text-xl font-black leading-none tracking-tight">{{ player.currentTrack.title }}</h1>
          <button v-if="player.currentTrack.album" type="button" class="mt-2 block w-fit max-w-full truncate text-left text-sm text-muted transition-colors hover:text-accent" :title="t('library.openAlbum')" @click="emit('openAlbum', player.currentTrack.album)">{{ player.currentTrack.album }}</button>
        </div>
        <div ref="lyricsScroll" class="lyrics-scroll min-h-0 flex-1 overflow-hidden border-l border-line pl-6 md:max-h-[76vh] md:pl-10" :style="{ '--active-index': lyrics.activeIndex, '--line-size': `${lyrics.classic.lineSize}px`, '--translation-size': `${lyrics.classic.translationSize}px`, '--line-spacing': `${lyrics.classic.lineSpacing}px`, '--line-weight': lyrics.classic.fontWeight, '--line-weight-active': lyricActiveWeight, fontFamily: lyricFontFamily }" @wheel.prevent="onWheel" @pointerdown="onTouchStart" @pointermove="onTouchMove" @pointerup="onTouchEnd" @pointercancel="onTouchEnd">
          <div ref="lyricsTrack" class="lyrics-track" :class="{ 'is-manual': manualScroll }" :style="{ transform: `translate3d(0, ${-scrollOffset}px, 0)` }">
          <div class="pt-[30vh] pb-[50vh]">
            <p v-if="lyrics.status === 'loading'" class="text-muted">{{ t("lyrics.loading") }}</p>
            <template v-else-if="lyrics.hasLyrics">
              <div v-for="(line, lineIndex) in lyrics.lines" :key="`${line.startTime}-${lineIndex}`" class="lyric-line" :class="{ 'is-active': activeSet.has(lineIndex), 'text-muted': !activeSet.has(lineIndex), 'is-bg': line.isBG, 'is-duet': line.isDuet }" :data-lyric-active="activeSet.has(lineIndex)" :data-lyric-primary="lineIndex === lyrics.activeIndex" :data-lyric-index="lineIndex" :style="{ '--line-i': lineIndex }">
                <p class="lyric-primary">
                  <template v-for="(word, wordIndex) in line.words" :key="`${word.startTime}-${wordIndex}`">
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
            <p v-else-if="!lyrics.suppressed" class="text-muted">{{ t("lyrics.empty") }}</p>
          </div>
          </div>
        </div>
      </div>
    </div>
    <PlayerBar @open="emit('navigate', returnView)" @queue="emit('queue')" />
  </div>
</template>

<style scoped>
@property --word-progress {
  syntax: "<percentage>";
  inherits: false;
  initial-value: 0%;
}

@property --word-sung {
  syntax: "<color>";
  inherits: false;
  initial-value: transparent;
}

@property --word-rest {
  syntax: "<color>";
  inherits: false;
  initial-value: transparent;
}

.lyrics-scroll {
  scrollbar-width: none;
  /* Dragging the lyrics is handled by pointer events, so stop the browser
     from claiming the vertical gesture (which would cancel our pointermove). */
  touch-action: none;
  --lyric-ease: cubic-bezier(0.22, 1, 0.36, 1);
  --lyric-duration: 560ms;
  -webkit-mask-image: linear-gradient(to bottom, transparent 0%, #000 5%, #000 95%, transparent 100%);
  mask-image: linear-gradient(to bottom, transparent 0%, #000 5%, #000 95%, transparent 100%);
  -webkit-mask-repeat: no-repeat;
  mask-repeat: no-repeat;
  -webkit-mask-size: 100% 100%;
  mask-size: 100% 100%;
}

.lyrics-scroll::-webkit-scrollbar {
  display: none;
}

.lyrics-track {
  position: relative;
  transition: transform 900ms cubic-bezier(0.22, 1, 0.36, 1);
  will-change: transform;
}

.lyrics-track.is-manual {
  transition: none;
}

.lyric-line {
  opacity: 0.72;
  transition: opacity var(--lyric-duration) var(--lyric-ease);
  margin-bottom: var(--line-spacing, 16px);
}

.lyric-line.is-active {
  opacity: 1;
}

.lyric-primary {
  font-size: var(--line-size, 24px);
  font-weight: var(--line-weight, 600);
  line-height: 1.4;
  transition: font-weight var(--lyric-duration) var(--lyric-ease);
}

.lyric-line.is-active .lyric-primary {
  color: var(--fg);
  font-weight: var(--line-weight-active, 800);
}

/*
 * The colours now switch instantly. Interpolating a registered custom property
 * that a `background-clip: text` gradient paints produced a visible flash while
 * the scroll transform was re-rasterising at the same moment; the line still
 * fades in and out through the `opacity` transition on `.lyric-line`.
 */
.lyric-word {
  --word-sung: var(--muted);
  --word-rest: var(--muted);
  display: inline;
  background: linear-gradient(90deg, var(--word-sung) var(--word-progress), var(--word-rest) var(--word-progress));
  -webkit-background-clip: text;
  background-clip: text;
  color: transparent;
  -webkit-text-fill-color: transparent;
}

.lyric-line.is-active .lyric-word {
  --word-sung: var(--accent);
  --word-rest: var(--fg);
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
  --word-sung: var(--muted);
  --word-rest: var(--muted);
  font-size: 0.5em;
  font-weight: calc(var(--line-weight, 600) - 200);
  background: linear-gradient(90deg, var(--word-sung) var(--word-progress), var(--word-rest) var(--word-progress));
  -webkit-background-clip: text;
  background-clip: text;
  color: transparent;
  -webkit-text-fill-color: transparent;
}

.lyric-line.is-active .lyric-ruby-text {
  --word-sung: var(--accent);
  --word-rest: var(--fg);
}

.lyric-line.is-bg .lyric-ruby-text {
  --word-sung: var(--dim);
  --word-rest: var(--dim);
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

.lyric-line.is-bg .lyric-word {
  --word-sung: var(--dim);
  --word-rest: var(--dim);
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

.lyric-line.is-bg.is-active .lyric-word {
  --word-sung: var(--accent);
  --word-rest: var(--muted);
}

.lyric-line.is-bg.is-active .lyric-primary {
  font-weight: calc(var(--line-weight-active, 800) - 200);
}

@media (min-width: 640px) {
  .lyric-primary {
    font-size: var(--line-size, 24px);
  }
}
</style>
