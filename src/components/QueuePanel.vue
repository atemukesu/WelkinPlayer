<script setup lang="ts">
import { computed } from "vue";
import { VList } from "virtua/vue";
import { LoaderCircle, X } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { usePlayerStore } from "../stores/player";
import { coverPending, initial, pad } from "../lib/format";
import { requestMeta } from "../directives/requestMeta";

const vRequestMeta = requestMeta;
const emit = defineEmits<{ close: [] }>();
const { t } = useI18n();
const player = usePlayerStore();

const QUEUE_ROW_HEIGHT = 52;

const upcoming = computed(() => {
  if (player.queue.length === 0 || player.queueIndex < 0) return [];
  return player.queue.slice(player.queueIndex + 1);
});

function playFromQueue(index: number) {
  const track = player.queue[index];
  if (track) {
    player.queueIndex = index;
    player.selectTrack(track);
  }
}
</script>

<template>
  <div class="flex h-full flex-col">
    <div class="flex items-center justify-between px-5 py-4">
      <h2 class="text-sm font-bold uppercase tracking-[0.25em]">{{ t("nowPlaying.queue") }}</h2>
      <button class="text-dim transition-colors hover:text-fg" @click="emit('close')">
        <X :size="16" :stroke-width="1.8" />
      </button>
    </div>

    <div v-if="player.currentTrack" class="shrink-0 border-b border-line px-5 pb-4">
      <p class="mb-2 font-mono text-[10px] uppercase tracking-[0.3em] text-dim">{{ t("nowPlaying.now") }}</p>
      <div class="flex items-center gap-3">
        <span
          class="grid h-10 w-10 shrink-0 place-items-center overflow-hidden text-lg font-black text-white/90"
          :style="{ backgroundColor: player.currentTrack.color }"
        >
          <img v-if="player.currentTrack.cover" :src="player.currentTrack.cover" alt="" decoding="async" class="h-full w-full object-cover" />
          <LoaderCircle v-else-if="coverPending(player.currentTrack)" :size="16" :stroke-width="2" class="animate-spin" />
          <template v-else>{{ initial(player.currentTrack) }}</template>
        </span>
        <span class="min-w-0 flex-1">
          <strong class="block truncate text-[13px] font-semibold text-accent">{{ player.currentTrack.title }}</strong>
          <small class="block truncate text-[12px] text-muted">{{ player.currentTrack.artist }}</small>
        </span>
      </div>
    </div>

    <div v-if="upcoming.length === 0" class="min-h-0 flex-1 overflow-y-auto">
      <div class="px-5 py-10 text-center text-sm text-muted">
        {{ t("nowPlaying.emptyQueue") }}
      </div>
    </div>
    <template v-else>
      <p class="shrink-0 px-5 pt-3 pb-2 font-mono text-[10px] uppercase tracking-[0.3em] text-dim">{{ t("nowPlaying.nextUp", { count: upcoming.length }) }}</p>
      <VList :data="upcoming" :item-size="QUEUE_ROW_HEIGHT" class="min-h-0 flex-1">
        <template #default="{ item: track, index }">
          <button
            type="button"
            v-request-meta="track"
            class="flex w-full items-center gap-3 px-5 py-2 text-left transition-colors hover:bg-fg/5"
            @click="playFromQueue(player.queueIndex + 1 + index)"
          >
            <span class="w-6 shrink-0 text-right font-mono text-[11px] tabular-nums text-dim">{{ pad(index + 1) }}</span>
            <span
              class="grid h-9 w-9 shrink-0 place-items-center overflow-hidden text-sm font-black text-white/90"
              :style="{ backgroundColor: track.color }"
            >
              <img v-if="track.cover" :src="track.cover" alt="" loading="lazy" decoding="async" class="h-full w-full object-cover" />
              <LoaderCircle v-else-if="coverPending(track)" :size="14" :stroke-width="2" class="animate-spin" />
              <template v-else>{{ initial(track) }}</template>
            </span>
            <span class="min-w-0 flex-1">
              <strong class="block truncate text-[12px] font-semibold">{{ track.title }}</strong>
              <small class="block truncate text-[11px] text-muted">{{ track.artist }}</small>
            </span>
          </button>
        </template>
      </VList>
    </template>
  </div>
</template>
