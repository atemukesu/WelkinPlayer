<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { ChevronDown } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { usePlayerStore } from "../stores/player";
import { useLyricsStore } from "../stores/lyrics";
import { currentTime } from "../lib/audio";
import { initial } from "../lib/format";
import PlayerBar from "../components/PlayerBar.vue";
import type { View } from "../lib/app";

defineProps<{ returnView: View }>();
const emit = defineEmits<{ navigate: [view: View]; queue: [] }>();
const { t } = useI18n();
const player = usePlayerStore();
const lyrics = useLyricsStore();
const lyricsScroll = ref<HTMLElement | null>(null);
const lyricsTrack = ref<HTMLElement | null>(null);
const scrollOffset = ref(0);
const manualScroll = ref(false);
let lastScrolledIndex = -1;
let frame = 0;
let lastLyricsSync = 0;
let manualTimer = 0;

function syncLyrics() {
  const positionMs = currentTime() * 1000;
  lyrics.sync(positionMs / 1000);
  updateWordProgress(positionMs);
}

function updateWordProgress(positionMs: number) {
  const container = lyricsScroll.value;
  const activeLine = container?.querySelector<HTMLElement>('[data-lyric-active="true"]');
  if (!activeLine || lyrics.activeIndex < 0) return;
  const line = lyrics.lines[lyrics.activeIndex];
  if (!line) return;
  activeLine.querySelectorAll<HTMLElement>(".lyric-word").forEach((word, wordIndex) => {
    word.style.setProperty("--word-progress", `${lyrics.wordProgress(line, wordIndex, positionMs) * 100}%`);
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

/** Vertical offset (px) that centers the active line inside the viewport. */
function activeLineOffset(): number {
  const container = lyricsScroll.value;
  const track = lyricsTrack.value;
  if (!container || !track) return scrollOffset.value;
  const line = container.querySelector<HTMLElement>('[data-lyric-active="true"]');
  if (!line) return scrollOffset.value;
  const maxOffset = Math.max(0, track.offsetHeight - container.clientHeight);
  const target = line.offsetTop + line.offsetHeight / 2 - container.clientHeight / 2;
  return Math.min(maxOffset, Math.max(0, target));
}

function onWheel(event: WheelEvent) {
  const container = lyricsScroll.value;
  const track = lyricsTrack.value;
  if (!container || !track) return;
  manualScroll.value = true;
  window.clearTimeout(manualTimer);
  manualTimer = window.setTimeout(() => { manualScroll.value = false; }, 160);
  const maxOffset = Math.max(0, track.offsetHeight - container.clientHeight);
  scrollOffset.value = Math.min(maxOffset, Math.max(0, scrollOffset.value + event.deltaY));
}

onMounted(() => {
  frame = requestAnimationFrame(loop);
});
onBeforeUnmount(() => {
  cancelAnimationFrame(frame);
  window.clearTimeout(manualTimer);
});

watch(
  () => player.currentTrack?.path,
  (path) => {
    lastScrolledIndex = -1;
    scrollOffset.value = 0;
    void lyrics.loadForTrack(path).then(syncLyrics);
  },
  { immediate: true },
);
watch(
  () => lyrics.activeIndex,
  (index) => {
    if (index < 0 || index === lastScrolledIndex) return;
    lastScrolledIndex = index;
    void nextTick(() => { scrollOffset.value = activeLineOffset(); });
  },
  { flush: "pre" },
);
</script>

<template>
  <div v-if="player.currentTrack" class="fixed inset-0 z-50 flex h-screen flex-col overflow-hidden bg-bg text-fg">
    <div class="pointer-events-none absolute inset-0 opacity-20" :style="{ background: `radial-gradient(circle at 20% 20%, ${player.currentTrack.color}, transparent 55%), radial-gradient(circle at 80% 80%, ${player.currentTrack.color}, transparent 55%)` }"></div>
    <div class="pointer-events-none absolute -right-32 top-1/2 h-[150%] w-40 rotate-[18deg] bg-accent/10"></div>
    <div class="pointer-events-none absolute -left-24 bottom-0 h-[60%] w-24 -rotate-[18deg] bg-accent/5"></div>
    <header class="relative z-10 flex h-16 shrink-0 items-center px-5">
      <button class="flex items-center gap-2 text-[13px] font-semibold uppercase tracking-[0.3em] text-muted transition-colors hover:text-fg" @click="emit('navigate', returnView)">
        <ChevronDown :size="16" :stroke-width="2" />{{ t("controls.return") }}
      </button>
    </header>
    <div class="relative z-10 grid min-h-0 flex-1 gap-6 overflow-hidden px-5 py-4 md:grid-cols-[minmax(220px,0.34fr)_minmax(0,0.66fr)] md:gap-12 md:px-10 md:py-8 lg:grid-cols-[minmax(280px,0.32fr)_minmax(0,0.68fr)] lg:gap-16 lg:px-16">
      <div class="hidden min-h-0 flex-col justify-center md:flex">
        <div class="ak-frame mx-auto aspect-square w-full max-w-[420px] overflow-hidden" :style="{ backgroundColor: player.currentTrack.color }">
          <img v-if="player.currentTrack.cover" :src="player.currentTrack.cover" alt="" decoding="async" class="h-full w-full object-cover" />
          <span v-else-if="player.currentTrack.metaLoaded" class="grid h-full w-full place-items-center text-8xl font-black text-white/90">{{ initial(player.currentTrack) }}</span>
        </div>
        <div class="mx-auto mt-6 w-full max-w-[420px]">
          <h1 class="mt-3 text-2xl font-black leading-none tracking-tight lg:text-3xl">{{ player.currentTrack.title }}</h1>
          <p class="mt-4 text-lg text-muted">{{ player.currentTrack.album }}</p>
          <p class="mt-2 text-base text-dim">{{ player.currentTrack.artist }}</p>
        </div>
      </div>
      <div class="flex min-h-0 min-w-0 flex-col justify-center">
        <div class="mb-5 md:hidden">
          <h1 class="mt-2 truncate text-xl font-black leading-none tracking-tight">{{ player.currentTrack.title }}</h1>
          <p class="mt-2 truncate text-sm text-muted">{{ player.currentTrack.album }}</p>
        </div>
        <div ref="lyricsScroll" class="lyrics-scroll min-h-0 flex-1 overflow-hidden border-l border-line pl-6 md:max-h-[76vh] md:pl-10" :style="{ '--active-index': lyrics.activeIndex, '--line-size': `${lyrics.lineSize}px`, '--translation-size': `${lyrics.translationSize}px`, '--line-spacing': `${lyrics.lineSpacing}px` }" @wheel.prevent="onWheel">
          <div ref="lyricsTrack" class="lyrics-track" :class="{ 'is-manual': manualScroll }" :style="{ transform: `translate3d(0, ${-scrollOffset}px, 0)` }">
          <div class="py-[30vh]">
            <p v-if="lyrics.status === 'loading'" class="text-muted">{{ t("lyrics.loading") }}</p>
            <p v-else-if="lyrics.status === 'error' || !lyrics.hasLyrics" class="text-muted">{{ t("lyrics.empty") }}</p>
            <template v-else>
              <div v-for="(line, lineIndex) in lyrics.lines" :key="`${line.startTime}-${lineIndex}`" class="lyric-line" :class="lineIndex === lyrics.activeIndex ? 'is-active' : 'text-muted'" :data-lyric-active="lineIndex === lyrics.activeIndex" :style="{ '--line-i': lineIndex }">
                <p class="lyric-primary"><span v-for="(word, wordIndex) in line.words" :key="`${word.startTime}-${wordIndex}`" class="lyric-word" :style="{ '--word-progress': lineIndex === lyrics.activeIndex ? '0%' : '100%' }">{{ word.word }}</span></p>
                <p v-if="lyrics.translate && line.translatedLyric" class="lyric-translation">{{ line.translatedLyric }}</p>
              </div>
            </template>
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
  inherits: true;
  initial-value: transparent;
}

@property --word-rest {
  syntax: "<color>";
  inherits: true;
  initial-value: transparent;
}

.lyrics-scroll {
  scrollbar-width: none;
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
  --word-sung: var(--muted);
  --word-rest: var(--muted);
  opacity: 0.72;
  transition: opacity var(--lyric-duration) var(--lyric-ease), --word-sung var(--lyric-duration) var(--lyric-ease), --word-rest var(--lyric-duration) var(--lyric-ease);
  margin-bottom: var(--line-spacing, 16px);
}

.lyric-line.is-active {
  --word-sung: var(--accent);
  --word-rest: var(--fg);
  opacity: 1;
}

.lyric-primary {
  font-size: var(--line-size, 24px);
  font-weight: 600;
  line-height: 1.4;
  transition: font-weight var(--lyric-duration) var(--lyric-ease);
}

.lyric-line.is-active .lyric-primary {
  color: var(--fg);
  font-weight: 800;
}

.lyric-word {
  display: inline;
  background: linear-gradient(90deg, var(--word-sung) var(--word-progress), var(--word-rest) var(--word-progress));
  -webkit-background-clip: text;
  background-clip: text;
  color: transparent;
}

.lyric-translation {
  margin-top: 0.35rem;
  font-size: var(--translation-size, 18px);
  line-height: 1.5;
  color: var(--dim);
}

@media (min-width: 640px) {
  .lyric-primary {
    font-size: var(--line-size, 24px);
  }
}
</style>
