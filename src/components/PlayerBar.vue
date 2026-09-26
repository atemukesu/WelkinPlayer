<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import {
  ChevronUp,
  ListMusic,
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
import { usePlayerStore } from "../stores/player";
import { initial, percent } from "../lib/format";
import { seekPercent } from "../lib/audio";

const emit = defineEmits<{ open: []; queue: [] }>();

const { t } = useI18n();
const player = usePlayerStore();
/** Mobile control tray; desktop always renders the full set of controls. */
const expanded = ref(false);

const repeatTitle = computed(() => player.repeat === "one" ? t("controls.repeatOne") : player.repeat === "all" ? t("controls.repeatAll") : t("controls.repeatOff"));
const shuffleTitle = computed(() => player.shuffle ? t("controls.shuffleOn") : t("controls.shuffleOff"));

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
        <button class="grid h-9 w-9 place-items-center" :class="player.shuffle ? 'text-accent' : 'text-muted'" :title="shuffleTitle" @click="player.toggleShuffle()">
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
        <button class="grid h-9 w-9 place-items-center" :class="player.repeat !== 'off' ? 'text-accent' : 'text-muted'" :title="repeatTitle" @click="player.cycleRepeat()">
          <Repeat1 v-if="player.repeat === 'one'" :size="16" :stroke-width="1.8" />
          <Repeat v-else :size="16" :stroke-width="1.8" />
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
          @click="emit('queue')"
        >
          <ListMusic :size="17" :stroke-width="1.8" />
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
          type="button"
          class="grid h-10 w-10 place-items-center text-muted transition-colors hover:text-fg md:hidden"
          :title="t(expanded ? 'controls.collapse' : 'controls.expand')"
          :aria-label="t(expanded ? 'controls.collapse' : 'controls.expand')"
          :aria-expanded="expanded"
          @click="expanded = !expanded"
        >
          <ChevronUp :size="18" :stroke-width="2" class="transition-transform duration-300" :class="expanded ? 'rotate-180' : ''" />
        </button>
      </div>
    </div>

    <Transition name="tray">
      <div v-if="expanded" class="player-tray md:hidden">
      <div class="overflow-hidden">
      <div class="grid gap-3 border-t border-line px-4 py-3">
      <div class="flex items-center justify-between">
        <button class="grid h-10 w-10 place-items-center" :class="player.shuffle ? 'text-accent' : 'text-muted'" :title="shuffleTitle" :aria-label="shuffleTitle" @click="player.toggleShuffle()">
          <Shuffle :size="17" :stroke-width="1.8" />
        </button>
        <button class="grid h-10 w-10 place-items-center text-fg transition-colors hover:text-accent" :title="t('controls.previous')" :aria-label="t('controls.previous')" @click="player.previous()">
          <SkipBack :size="20" :stroke-width="2" />
        </button>
        <button class="grid h-10 w-10 place-items-center text-fg transition-colors hover:text-accent" :title="t('controls.next')" :aria-label="t('controls.next')" @click="player.next()">
          <SkipForward :size="20" :stroke-width="2" />
        </button>
        <button class="grid h-10 w-10 place-items-center" :class="player.repeat !== 'off' ? 'text-accent' : 'text-muted'" :title="repeatTitle" :aria-label="repeatTitle" @click="player.cycleRepeat()">
          <Repeat1 v-if="player.repeat === 'one'" :size="17" :stroke-width="1.8" />
          <Repeat v-else :size="17" :stroke-width="1.8" />
        </button>
        <button class="grid h-10 w-10 place-items-center text-muted transition-colors hover:text-fg" :title="t('controls.queue')" :aria-label="t('controls.queue')" @click="emit('queue')">
          <ListMusic :size="18" :stroke-width="1.8" />
        </button>
      </div>
      <div class="flex items-center gap-3">
        <span class="font-mono text-[12px] tabular-nums text-muted">{{ player.elapsedTime }} / {{ player.totalTime }}</span>
        <div class="ml-auto flex items-center gap-2">
          <button type="button" class="text-dim transition-colors hover:text-fg" :title="player.muted ? t('controls.unmute') : t('controls.mute')" :aria-label="player.muted ? t('controls.unmute') : t('controls.mute')" @click="player.toggleMute()">
            <VolumeX v-if="player.muted" :size="17" :stroke-width="1.8" />
            <Volume2 v-else :size="17" :stroke-width="1.8" />
          </button>
          <input v-model.number="player.volume" :style="{ '--fill': percent(player.volume) }" class="ak-slider w-28" type="range" min="0" max="100" aria-label="Volume" />
        </div>
      </div>
      </div>
      </div>
      </div>
    </Transition>
  </footer>
</template>

<style scoped>
.player-tray {
  display: grid;
  grid-template-rows: 1fr;
}

.player-tray > * {
  min-height: 0;
  overflow: hidden;
}

.tray-enter-active,
.tray-leave-active {
  transition: grid-template-rows 300ms cubic-bezier(0.2, 0.8, 0.2, 1), opacity 220ms ease;
}

.tray-enter-from,
.tray-leave-to {
  grid-template-rows: 0fr;
  opacity: 0;
}
</style>
