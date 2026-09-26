<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, toRaw, watch } from "vue";
import {
  ChevronDown,
  Pause,
  Play,
  Repeat,
  Repeat1,
  Shuffle,
  SkipBack,
  SkipForward,
  Volume2,
  VolumeX,
} from "@lucide/vue";
import { useI18n } from "vue-i18n";
import "@applemusic-like-lyrics/core/style.css";
import AmllBackground from "../components/AmllBackground.vue";
import AmllLyrics from "../components/AmllLyrics.vue";
import { usePlayerStore } from "../stores/player";
import { useLyricsStore } from "../stores/lyrics";
import { currentTime, seekPercent, seekTo } from "../lib/audio";
import { initial, percent } from "../lib/format";
import type { View } from "../lib/app";

const props = defineProps<{ returnView: View; active: boolean }>();
const emit = defineEmits<{ navigate: [view: View]; closed: [] }>();
const { t } = useI18n();
const player = usePlayerStore();
const lyrics = useLyricsStore();

const positionMs = ref(0);
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

onMounted(() => { frame = requestAnimationFrame(loop); });
onBeforeUnmount(() => cancelAnimationFrame(frame));

watch(
  () => player.currentTrack?.path,
  (path) => { lyrics.loadForTrack(path); },
  { immediate: true },
);

const cover = computed(() => player.currentTrack?.cover);
const repeatTitle = computed(() => player.repeat === "one" ? t("controls.repeatOne") : player.repeat === "all" ? t("controls.repeatAll") : t("controls.repeatOff"));
const shuffleTitle = computed(() => player.shuffle ? t("controls.shuffleOn") : t("controls.shuffleOff"));

/**
 * AMLL deep-clones its input with `structuredClone`, which rejects Vue's
 * reactive proxies — hand it the raw (non-reactive) lyric data instead. Also
 * respect the translation toggle by stripping translated text when disabled.
 */
const amllLines = computed(() => {
  const raw = toRaw(lyrics.lines);
  if (lyrics.translate) return raw;
  return raw.map((line) => (line.translatedLyric ? { ...line, translatedLyric: "" } : line));
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

    <header class="absolute inset-x-0 top-0 z-20 flex h-16 items-center gap-4 px-4 sm:px-6">
      <button
        type="button"
        class="grid h-10 w-10 shrink-0 place-items-center rounded-full bg-white/10 text-white backdrop-blur transition hover:bg-white/20"
        :aria-label="t('controls.return')"
        @click="emit('navigate', returnView)"
      >
        <ChevronDown :size="20" :stroke-width="2" />
      </button>
      <span class="flex-1"></span>
      <span class="h-10 w-10 shrink-0"></span>
    </header>

    <div class="relative z-10 grid h-full grid-cols-1 gap-8 px-6 pb-44 pt-24 md:grid-cols-[minmax(240px,0.82fr)_minmax(0,1.18fr)] md:gap-14 md:px-14 lg:px-24">
      <div class="hidden min-h-0 flex-col items-center justify-center gap-6 md:flex">
        <div class="aspect-square w-full max-w-[420px] overflow-hidden rounded-2xl shadow-2xl shadow-black/40" :style="{ backgroundColor: player.currentTrack.color }">
          <img v-if="player.currentTrack.cover" :src="player.currentTrack.cover" alt="" decoding="async" class="h-full w-full object-cover" />
          <span v-else class="grid h-full w-full place-items-center text-8xl font-black text-white/90">{{ initial(player.currentTrack) }}</span>
        </div>
        <div class="w-full max-w-[420px] text-center md:text-left">
          <h1 class="truncate text-2xl font-bold tracking-tight lg:text-3xl">{{ player.currentTrack.title }}</h1>
          <p class="mt-2 truncate text-lg text-white/75">{{ player.currentTrack.artist }}</p>
          <p class="mt-1 truncate text-sm text-white/55">{{ player.currentTrack.album }}</p>
        </div>
      </div>

      <div class="flex min-h-0 flex-col justify-center">
        <div class="mb-4 text-center md:hidden">
          <h1 class="truncate text-xl font-bold">{{ player.currentTrack.title }}</h1>
          <p class="truncate text-sm text-white/70">{{ player.currentTrack.artist }}</p>
        </div>
        <p v-if="lyrics.status === 'loading'" class="text-center text-white/70">{{ t("lyrics.loading") }}</p>
        <p v-else-if="lyrics.status === 'error' || !lyrics.hasLyrics" class="text-center text-white/70">{{ t("lyrics.empty") }}</p>
        <div v-else class="relative min-h-0 flex-1">
          <AmllLyrics
            :lines="amllLines"
            :current-time="positionMs"
            :playing="player.isPlaying"
            :font-size="lyrics.lineSize"
            color="#ffffff"
            blend="plus-lighter"
            @seek="onLyricSeek"
          />
        </div>
      </div>
    </div>

    <footer class="absolute inset-x-0 bottom-0 z-20 px-6 pb-6">
      <div class="mx-auto w-full max-w-3xl">
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
        <div class="mt-4 flex items-center justify-between gap-4">
          <span class="hidden w-24 font-mono text-xs tabular-nums text-white/60 sm:block">{{ player.elapsedTime }}</span>
          <div class="flex flex-1 items-center justify-center gap-5 sm:gap-7">
            <button type="button" class="transition" :class="player.shuffle ? 'text-white' : 'text-white/50 hover:text-white'" :title="shuffleTitle" @click="player.toggleShuffle()">
              <Shuffle :size="20" :stroke-width="2" />
            </button>
            <button type="button" class="text-white transition hover:scale-105 active:scale-95" @click="player.previous()">
              <SkipBack :size="26" :stroke-width="2" />
            </button>
            <button
              type="button"
              class="grid h-14 w-14 place-items-center rounded-full bg-white text-black shadow-lg shadow-black/30 transition hover:scale-105 active:scale-95"
              :aria-label="player.isPlaying ? t('controls.pause') : t('controls.play')"
              @click="player.togglePlayback()"
            >
              <Pause v-if="player.isPlaying" :size="24" :stroke-width="2.4" />
              <Play v-else :size="24" :stroke-width="2.4" class="translate-x-[1px]" />
            </button>
            <button type="button" class="text-white transition hover:scale-105 active:scale-95" @click="player.next()">
              <SkipForward :size="26" :stroke-width="2" />
            </button>
            <button type="button" class="transition" :class="player.repeat !== 'off' ? 'text-white' : 'text-white/50 hover:text-white'" :title="repeatTitle" @click="player.cycleRepeat()">
              <Repeat1 v-if="player.repeat === 'one'" :size="20" :stroke-width="2" />
              <Repeat v-else :size="20" :stroke-width="2" />
            </button>
          </div>
          <div class="hidden items-center gap-2 sm:flex">
            <button
              type="button"
              class="text-white/60 transition hover:text-white"
              :aria-label="player.muted ? t('controls.unmute') : t('controls.mute')"
              @click="player.toggleMute()"
            >
              <VolumeX v-if="player.muted" :size="18" :stroke-width="2" />
              <Volume2 v-else :size="18" :stroke-width="2" />
            </button>
            <input v-model.number="player.volume" :style="{ '--fill': percent(player.volume) }" class="ak-slider w-24" type="range" min="0" max="100" aria-label="Volume" />
          </div>
        </div>
      </div>
    </footer>
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

.amll-page :deep(.ak-slider) {
  height: 4px;
}

.amll-page :deep(.ak-slider::-webkit-slider-thumb) {
  background: #fff;
  border-color: rgba(0, 0, 0, 0.35);
}

.amll-page :deep(.ak-slider::-moz-range-thumb) {
  background: #fff;
  border-color: rgba(0, 0, 0, 0.35);
}
</style>
