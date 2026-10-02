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
import "@applemusic-like-lyrics/core/style.css";
import AmllBackground from "../components/AmllBackground.vue";
import AmllLyrics from "../components/AmllLyrics.vue";
import { usePlayerStore } from "../stores/player";
import { useLyricsStore } from "../stores/lyrics";
import { currentTime, seekPercent, seekTo } from "../lib/audio";
import { coverPending, initial, percent } from "../lib/format";
import { useWakeLock } from "../composables/useWakeLock";
import type { View } from "../lib/app";

const props = defineProps<{ returnView: View; active: boolean }>();
const emit = defineEmits<{ navigate: [view: View]; closed: [] }>();
const { t } = useI18n();
const player = usePlayerStore();
const lyrics = useLyricsStore();

/** Keep the display on for as long as this full-screen player is open. */
useWakeLock(() => props.active);

const positionMs = ref(0);
const showRemaining = ref(false);
let frame = 0;

/**
 * The page starts translated off-screen and only slides in once the AMLL
 * background has painted its first frame, so the animation never shows an
 * empty/black background. On close it slides back out, then reports `closed`
 * so the host can unmount it.
 */
const visible = ref(false);
let closed = false;

function onBackgroundReady() {
  if (closed) return;
  requestAnimationFrame(() => { if (!closed) visible.value = true; });
}

function finishClose() {
  if (closed) return;
  closed = true;
  emit("closed");
}

function onTransitionEnd(event: TransitionEvent) {
  if (event.target !== event.currentTarget) return;
  if (event.propertyName === "transform" && !visible.value) finishClose();
}

watch(
  () => props.active,
  (active) => {
    if (active) return;
    if (!visible.value) { finishClose(); return; }
    visible.value = false;
    // Fallback in case the transitionend event never arrives.
    window.setTimeout(() => { if (!visible.value) finishClose(); }, 420);
  },
);

function loop() {
  positionMs.value = currentTime() * 1000;
  frame = requestAnimationFrame(loop);
}

const controlsVisible = ref(true);
let controlsTimer = 0;

/** Reveal the floating close controls on activity, then fade them out when idle. */
function bumpControls() {
  controlsVisible.value = true;
  window.clearTimeout(controlsTimer);
  controlsTimer = window.setTimeout(() => { controlsVisible.value = false; }, 1000);
}

const revealClass = computed(() => (controlsVisible.value
  ? "pointer-events-auto translate-y-0 opacity-100"
  : "pointer-events-none -translate-y-2 opacity-0"));

const isFullscreen = ref(false);

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

/**
 * Space toggles playback and Escape folds the page away (same as the return
 * control). App.vue's global shortcut bails out when a button keeps focus, so
 * intercept both in the capture phase and stop the previously focused control
 * from being re-triggered.
 */
function onKeydown(event: KeyboardEvent) {
  const isSpace = event.code === "Space" || event.key === " ";
  const isEscape = event.key === "Escape";
  if (!isSpace && !isEscape) return;
  bumpControls();
  if (!props.active) return;
  if (isEscape) {
    event.preventDefault();
    event.stopPropagation();
    emit("navigate", props.returnView);
    return;
  }
  const target = event.target as HTMLElement | null;
  if (target) {
    const tag = target.tagName;
    if (tag === "TEXTAREA" || tag === "SELECT" || target.isContentEditable) return;
    if (tag === "INPUT" && (target as HTMLInputElement).type !== "range") return;
  }
  event.preventDefault();
  event.stopPropagation();
  if (event.repeat) return;
  player.togglePlayback();
}

onMounted(() => {
  frame = requestAnimationFrame(loop);
  bumpControls();
  void syncFullscreen();
  window.addEventListener("keydown", onKeydown, true);
  window.addEventListener("pointermove", bumpControls);
  window.addEventListener("pointerdown", bumpControls);
  window.addEventListener("wheel", bumpControls, { passive: true });
  window.addEventListener("resize", syncFullscreen);
});
onBeforeUnmount(() => {
  cancelAnimationFrame(frame);
  window.clearTimeout(controlsTimer);
  window.removeEventListener("keydown", onKeydown, true);
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
  () => { lyrics.loadForTrack(player.currentTrack ?? undefined); },
  { immediate: true },
);

const cover = computed(() => player.currentTrack?.cover);
const repeatTitle = computed(() => player.repeat === "one" ? t("controls.repeatOne") : player.repeat === "all" ? t("controls.repeatAll") : t("controls.repeatOff"));
const shuffleTitle = computed(() => player.shuffle ? t("controls.shuffleOn") : t("controls.shuffleOff"));

/**
 * AMLL deep-clones its input with `structuredClone`, which rejects Vue's
 * reactive proxies — hand it the raw (non-reactive) lyric data instead. Also
 * respect the translation and ruby toggles by stripping the corresponding text
 * when disabled.
 */
const amllLines = computed(() => {
  const raw = toRaw(lyrics.lines);
  const { translate, ruby } = lyrics.amll;
  if (translate && ruby) return raw;
  return raw.map((line) => {
    let next = line;
    if (!translate && line.translatedLyric) {
      next = { ...next, translatedLyric: "" };
    }
    if (!ruby && line.words.some((word) => word.ruby?.length)) {
      next = { ...next, words: line.words.map((word) => (word.ruby?.length ? { ...word, ruby: undefined } : word)) };
    }
    return next;
  });
});

function onSeek(event: Event) {
  seekPercent(Number((event.target as HTMLInputElement).value));
}

/** Jump to the start of a clicked lyric line. */
function onLyricSeek(timeMs: number) {
  seekTo(timeMs / 1000);
}
</script>

<template>
  <div
    v-if="player.currentTrack"
    class="amll-page fixed inset-0 z-50 overflow-hidden text-white"
    :class="{ 'is-open': visible }"
    @transitionend="onTransitionEnd"
    :style="{
      backgroundColor: player.currentTrack.color,
      '--accent': '#ffffff',
      '--accent-fg': '#0b0c0f',
      '--accent-soft': 'rgba(255, 255, 255, 0.18)',
      '--line': 'rgba(255, 255, 255, 0.22)',
      '--line-strong': 'rgba(255, 255, 255, 0.5)',
      '--fg': '#ffffff',
      '--muted': 'rgba(255, 255, 255, 0.72)',
      '--dim': 'rgba(255, 255, 255, 0.55)',
    }"
  >
    <div class="absolute inset-0">
      <AmllBackground :album="cover" :fps="30" :render-scale="0.5" @ready="onBackgroundReady" />
    </div>
    <div class="pointer-events-none absolute inset-0 bg-black/30"></div>
    <div class="pointer-events-none absolute inset-x-0 top-0 h-40 bg-gradient-to-b from-black/40 to-transparent"></div>
    <div class="pointer-events-none absolute inset-x-0 bottom-0 h-56 bg-gradient-to-t from-black/60 to-transparent"></div>

    <header class="pointer-events-none absolute inset-x-0 top-[env(safe-area-inset-top)] z-20 h-16 px-4 md:hidden">
      <div class="flex h-full min-w-0 items-center gap-3">
        <span
          class="grid h-10 w-10 shrink-0 place-items-center overflow-hidden rounded-md text-sm font-black text-white/90"
          :style="{ backgroundColor: player.currentTrack.color }"
        >
          <img v-if="player.currentTrack.cover" :src="player.currentTrack.cover" alt="" decoding="async" class="h-full w-full object-cover" />
          <LoaderCircle v-else-if="coverPending(player.currentTrack)" :size="16" :stroke-width="2" class="animate-spin" />
          <template v-else>{{ initial(player.currentTrack) }}</template>
        </span>
        <span class="grid min-w-0 gap-0.5">
          <strong class="truncate text-sm font-semibold leading-tight">{{ player.currentTrack.title }}</strong>
          <small class="truncate text-xs leading-tight text-white/70">{{ player.currentTrack.artist }}</small>
        </span>
      </div>
      <div class="absolute right-4 top-1/2 -translate-y-1/2">
        <div class="flex items-center gap-2 transition-all duration-300 ease-out" :class="revealClass">
          <button
            type="button"
            class="grid h-10 w-10 shrink-0 place-items-center rounded-full bg-white/10 text-white backdrop-blur transition hover:bg-white/20"
            :aria-label="t('controls.return')"
            @click="emit('navigate', returnView)"
          >
            <ChevronDown :size="20" :stroke-width="2" />
          </button>
          <button
            type="button"
            class="grid h-10 w-10 shrink-0 place-items-center rounded-full bg-white/10 text-white backdrop-blur transition hover:bg-white/20"
            :title="fullscreenTitle"
            :aria-label="fullscreenTitle"
            :aria-pressed="isFullscreen"
            @click="toggleFullscreen()"
          >
            <Minimize2 v-if="isFullscreen" :size="18" :stroke-width="2" />
            <Maximize2 v-else :size="18" :stroke-width="2" />
          </button>
        </div>
      </div>
    </header>

    <div class="relative z-10 flex h-full flex-col px-6 pt-[env(safe-area-inset-top)] md:grid md:grid-cols-[minmax(240px,0.82fr)_minmax(0,1.18fr)] md:grid-rows-1 md:gap-14 md:px-14 md:pt-0 lg:px-24">
      <div class="amll-controls order-2 shrink-0 md:order-none md:min-h-0" :data-collapsed="controlsVisible ? 'false' : 'true'">
        <div class="flex flex-col items-center justify-center gap-6 px-6 pb-6 pt-4 md:h-full md:gap-6 md:px-0 md:pb-16 md:pt-24">
        <div class="relative hidden w-full max-w-[420px] md:block">
          <div class="absolute -top-20 left-1/2 flex -translate-x-1/2 items-center gap-4 transition-all duration-300 ease-out" :class="revealClass">
            <button
              type="button"
              class="grid h-11 w-11 place-items-center rounded-full bg-white/10 text-white backdrop-blur transition hover:bg-white/20"
              :aria-label="t('controls.return')"
              @click="emit('navigate', returnView)"
            >
              <ChevronDown :size="20" :stroke-width="2" />
            </button>
            <button
              type="button"
              class="grid h-11 w-11 place-items-center rounded-full bg-white/10 text-white backdrop-blur transition hover:bg-white/20"
              :title="fullscreenTitle"
              :aria-label="fullscreenTitle"
              :aria-pressed="isFullscreen"
              @click="toggleFullscreen()"
            >
              <Minimize2 v-if="isFullscreen" :size="18" :stroke-width="2" />
              <Maximize2 v-else :size="18" :stroke-width="2" />
            </button>
          </div>
          <div class="aspect-square w-full overflow-hidden rounded-2xl shadow-2xl shadow-black/40" :style="{ backgroundColor: player.currentTrack.color }">
            <img v-if="player.currentTrack.cover" :src="player.currentTrack.cover" alt="" decoding="async" class="h-full w-full object-cover" />
            <span v-else-if="coverPending(player.currentTrack)" class="grid h-full w-full place-items-center text-white/90"><LoaderCircle :size="44" :stroke-width="1.8" class="animate-spin" /></span>
            <span v-else class="grid h-full w-full place-items-center text-8xl font-black text-white/90">{{ initial(player.currentTrack) }}</span>
          </div>
        </div>
        <div class="hidden w-full max-w-[420px] text-center md:block md:text-left">
          <h1 class="truncate text-2xl font-bold tracking-tight lg:text-3xl">{{ player.currentTrack.title }}</h1>
          <p class="mt-2 truncate text-lg text-white/75">{{ player.currentTrack.artist }}</p>
        </div>

        <div class="w-full max-w-[420px]">
          <div class="flex h-[9px] items-center">
            <input
              :value="player.progress"
              :style="{ '--fill': percent(player.progress), '--buffered': percent(player.bufferedProgress) }"
              class="ak-slider w-full"
              type="range"
              min="0"
              max="100"
              aria-label="Playback position"
              @input="onSeek"
            />
          </div>
          <div class="mt-1.5 flex items-center justify-between font-mono text-xs tabular-nums text-white/60">
            <span>{{ player.elapsedTime }}</span>
            <button
              type="button"
              class="transition hover:text-white"
              :aria-pressed="showRemaining"
              @click="showRemaining = !showRemaining"
            >
              <template v-if="showRemaining">-{{ player.remainingTime }}</template>
              <template v-else>{{ player.totalTime }}</template>
            </button>
          </div>
          <div class="mt-4 flex items-center justify-between gap-3">
            <button type="button" class="transition" :class="player.shuffle ? 'text-white' : 'text-white/50'" :title="shuffleTitle" @click="player.toggleShuffle()">
              <Shuffle :size="20" :stroke-width="2" />
            </button>
            <div class="flex items-center gap-3 sm:gap-5">
              <button type="button" class="grid h-12 w-12 place-items-center rounded-full text-white transition hover:scale-105 hover:bg-white/15 active:scale-95" @click="player.previous()">
                <SkipBack :size="26" :stroke-width="2" fill="currentColor" />
              </button>
              <button
                type="button"
                class="grid h-14 w-14 scale-110 place-items-center rounded-full text-white transition hover:scale-[1.16] hover:bg-white/15 active:scale-95"
                :aria-label="player.isPlaying ? t('controls.pause') : t('controls.play')"
                @click="player.togglePlayback()"
              >
                <Pause v-if="player.isPlaying" :size="24" :stroke-width="1.5" fill="currentColor" />
                <Play v-else :size="24" :stroke-width="1.5" fill="currentColor" class="translate-x-[1px]" />
              </button>
              <button type="button" class="grid h-12 w-12 place-items-center rounded-full text-white transition hover:scale-105 hover:bg-white/15 active:scale-95" @click="player.next()">
                <SkipForward :size="26" :stroke-width="2" fill="currentColor" />
              </button>
            </div>
            <button type="button" class="transition" :class="player.repeat !== 'off' ? 'text-white' : 'text-white/50'" :title="repeatTitle" @click="player.cycleRepeat()">
              <Repeat1 v-if="player.repeat === 'one'" :size="20" :stroke-width="2" />
              <Repeat v-else :size="20" :stroke-width="2" />
            </button>
          </div>
        </div>
        </div>
      </div>

      <div class="order-1 flex min-h-0 flex-1 flex-col justify-center pt-16 md:order-none md:pt-0">
        <p v-if="lyrics.status === 'loading'" class="text-center text-white/70">{{ t("lyrics.loading") }}</p>
        <div v-else-if="lyrics.hasLyrics" class="relative min-h-0 flex-1">
          <AmllLyrics
            :lines="amllLines"
            :current-time="positionMs"
            :playing="player.isPlaying"
            :font-size="lyrics.amll.lineSize"
            :font-weight="lyrics.amll.fontWeight"
            :font-families="lyrics.amll.fontFamilies"
            :translation-size="lyrics.amll.translationSize"
            color="#ffffff"
            blend="plus-lighter"
            :align-position="0.35"
            @seek="onLyricSeek"
          />
        </div>
        <p v-else-if="!lyrics.suppressed" class="text-center text-white/70">{{ t("lyrics.empty") }}</p>
      </div>
    </div>
  </div>
</template>

<style scoped>
.amll-page {
  transform: translateY(100%);
  transition: transform 320ms cubic-bezier(0.2, 0.8, 0.2, 1);
  will-change: transform;
}

.amll-page.is-open {
  transform: translateY(0);
}

/*
 * Mobile: the transport controls own a region below the lyrics. When they
 * auto-hide the region collapses to zero height, so the lyrics reclaim the
 * space instead of leaving a blank strip. A grid-row `0fr` collapse does NOT
 * work here because the container's height is auto inside a flex column, so
 * animate `max-height` instead.
 */
.amll-controls {
  max-height: 180px;
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


.amll-page :deep(.ak-slider) {
  height: 6px;
  border-radius: 9999px;
}

.amll-page :deep(.ak-slider:hover),
.amll-page :deep(.ak-slider:focus-visible),
.amll-page :deep(.ak-slider:active) {
  height: 9px;
}

.amll-page :deep(.ak-slider::-webkit-slider-thumb) {
  width: 14px;
  height: 14px;
  border-radius: 9999px;
  background: #fff;
  border-color: rgba(0, 0, 0, 0.35);
}

.amll-page :deep(.ak-slider::-moz-range-thumb) {
  width: 14px;
  height: 14px;
  border-radius: 9999px;
  background: #fff;
  border-color: rgba(0, 0, 0, 0.35);
}
</style>
