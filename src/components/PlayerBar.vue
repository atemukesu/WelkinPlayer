<script setup lang="ts">
import { useI18n } from "vue-i18n";
import {
  AudioLines,
  ListMusic,
  Maximize2,
  Pause,
  Play,
  Repeat,
  Shuffle,
  SkipBack,
  SkipForward,
  Volume2,
  VolumeX,
} from "@lucide/vue";
import { usePlayerStore } from "../stores/player";
import { initial, percent } from "../lib/format";
import { seekPercent } from "../lib/audio";

const emit = defineEmits<{ open: [] }>();

const { t } = useI18n();
const player = usePlayerStore();

function onSeek(event: Event) {
  seekPercent(Number((event.target as HTMLInputElement).value));
}
</script>

<template>
  <footer class="relative border-t border-line bg-surface">
    <input
      :value="player.progress"
      :style="{ '--fill': percent(player.progress), '--buffered': percent(player.bufferedProgress) }"
      class="ak-slider absolute inset-x-0 top-0 z-10"
      type="range"
      min="0"
      max="100"
      aria-label="Playback position"
      @input="onSeek"
    />
    <div class="flex h-[72px] items-center gap-4 px-4">
      <button class="flex min-w-0 flex-1 items-center gap-3 text-left md:w-[220px] md:flex-none" @click="emit('open')">
        <span
          class="grid h-11 w-11 shrink-0 place-items-center overflow-hidden text-lg font-black text-white/90"
          :style="{ backgroundColor: player.currentTrack?.color }"
        >
          <img
            v-if="player.currentTrack?.cover"
            :src="player.currentTrack.cover"
            alt=""
            decoding="async"
            class="h-full w-full object-cover"
          />
          <template v-else>{{ player.currentTrack ? initial(player.currentTrack) : "" }}</template>
        </span>
        <span class="grid min-w-0 gap-0.5">
          <strong class="truncate text-[13px] font-semibold tracking-wide">{{ player.currentTrack?.title }}</strong>
          <small class="truncate text-[13px] text-muted">{{ player.currentTrack?.artist }}</small>
        </span>
      </button>

      <div class="absolute left-1/2 hidden -translate-x-1/2 items-center gap-2 md:flex">
        <button class="grid h-9 w-9 place-items-center text-muted transition-colors hover:text-fg">
          <Shuffle :size="16" :stroke-width="1.8" />
        </button>
        <button class="grid h-10 w-10 place-items-center text-fg transition-colors hover:text-accent" @click="player.previous()">
          <SkipBack :size="18" :stroke-width="2" />
        </button>
        <button
          class="ak-clip grid h-11 w-11 place-items-center bg-accent text-accent-fg transition-transform hover:scale-[1.03] active:scale-95"
          :aria-label="player.isPlaying ? t('controls.pause') : t('controls.play')"
          @click="player.togglePlayback()"
        >
          <Pause v-if="player.isPlaying" :size="20" :stroke-width="2.2" />
          <Play v-else :size="20" :stroke-width="2.2" />
        </button>
        <button class="grid h-10 w-10 place-items-center text-fg transition-colors hover:text-accent" @click="player.next()">
          <SkipForward :size="18" :stroke-width="2" />
        </button>
        <button class="grid h-9 w-9 place-items-center text-muted transition-colors hover:text-fg">
          <Repeat :size="16" :stroke-width="1.8" />
        </button>
      </div>

      <div class="ml-auto flex items-center gap-2">
        <span class="hidden font-mono text-[13px] tabular-nums text-muted lg:block">
          {{ player.elapsedTime }} / {{ player.totalTime }}
        </span>
        <button
          class="hidden text-muted transition-colors hover:text-fg sm:block"
          :title="t('controls.queue')"
          :aria-label="t('controls.queue')"
        >
          <ListMusic :size="17" :stroke-width="1.8" />
        </button>
        <button
          class="hidden text-muted transition-colors hover:text-fg sm:block"
          :title="t('controls.lyrics')"
          :aria-label="t('controls.lyrics')"
        >
          <AudioLines :size="17" :stroke-width="1.8" />
        </button>
        <div class="hidden items-center gap-2 md:flex">
          <button
            type="button"
            class="text-dim transition-colors hover:text-fg"
            :title="player.muted ? t('controls.unmute') : t('controls.mute')"
            :aria-label="player.muted ? t('controls.unmute') : t('controls.mute')"
            @click="player.toggleMute()"
          >
            <VolumeX v-if="player.muted" :size="17" :stroke-width="1.8" />
            <Volume2 v-else :size="17" :stroke-width="1.8" />
          </button>
          <input
            v-model.number="player.volume"
            :style="{ '--fill': percent(player.volume) }"
            class="ak-slider w-24"
            type="range"
            min="0"
            max="100"
            aria-label="Volume"
          />
        </div>
        <button
          class="grid h-10 w-10 place-items-center bg-accent text-accent-fg transition-transform hover:scale-[1.03] active:scale-95 md:hidden"
          :aria-label="player.isPlaying ? t('controls.pause') : t('controls.play')"
          @click="player.togglePlayback()"
        >
          <Pause v-if="player.isPlaying" :size="18" :stroke-width="2.2" />
          <Play v-else :size="18" :stroke-width="2.2" />
        </button>
        <button
          class="text-muted transition-colors hover:text-fg"
          :title="t('controls.fullscreen')"
          :aria-label="t('controls.fullscreen')"
          @click="emit('open')"
        >
          <Maximize2 :size="17" :stroke-width="1.8" />
        </button>
      </div>
    </div>
  </footer>
</template>
