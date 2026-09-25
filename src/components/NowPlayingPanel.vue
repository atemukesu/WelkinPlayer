<script setup lang="ts">
import { computed } from "vue";
import { ChevronRight } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { usePlayerStore } from "../stores/player";
import { initial, pad } from "../lib/format";

const props = defineProps<{ collapsed: boolean; width: number; resizing: boolean }>();
const emit = defineEmits<{ toggle: []; resize: [event: PointerEvent] }>();
const { t } = useI18n();
const player = usePlayerStore();
const attributes = computed(() => player.currentTrack ? [
  { key: "nowPlaying.track", value: pad(player.currentIndex) }, { key: "nowPlaying.artist", value: player.currentTrack.artist },
  { key: "nowPlaying.album", value: player.currentTrack.album }, { key: "nowPlaying.duration", value: player.currentTrack.duration }, { key: "nowPlaying.source", value: "WebDAV" },
] : []);
</script>

<template>
  <aside v-if="player.currentTrack" class="relative hidden shrink-0 overflow-hidden bg-surface lg:flex" :class="[collapsed ? 'border-transparent' : 'border-l border-line', resizing ? '' : 'transition-[width] duration-300 ease-[cubic-bezier(0.2,0.8,0.2,1)]']" :style="{ width: (collapsed ? 0 : width) + 'px' }">
    <div class="flex shrink-0 flex-col overflow-y-auto p-5" :style="{ width: width + 'px' }"><div class="flex items-center justify-between font-mono text-[10px] uppercase tracking-[0.3em] text-dim">{{ t("nowPlaying.label") }}<button class="text-dim transition-colors hover:text-fg" :title="t('controls.collapse')" @click="emit('toggle')"><ChevronRight :size="16" :stroke-width="1.8" /></button></div>
      <div class="ak-frame mt-4 aspect-square w-full overflow-hidden" :style="{ backgroundColor: player.currentTrack.color }"><img v-if="player.currentTrack.cover" :src="player.currentTrack.cover" alt="" decoding="async" class="h-full w-full object-cover" /><span v-else class="grid h-full w-full place-items-center text-6xl font-black text-white/90">{{ initial(player.currentTrack) }}</span></div>
      <h3 class="mt-4 truncate text-sm font-bold tracking-wide">{{ player.currentTrack.title }}</h3><div class="mt-6"><p class="mb-3 font-mono text-[10px] uppercase tracking-[0.3em] text-dim">{{ t("nowPlaying.attributes") }}</p><dl class="flex flex-col border-t border-line"><div v-for="row in attributes" :key="row.key" class="flex items-baseline justify-between gap-4 border-b border-line py-3"><dt class="shrink-0 font-mono text-[10px] uppercase tracking-[0.25em] text-dim">{{ t(row.key) }}</dt><dd class="min-w-0 truncate text-right text-[13px] font-semibold">{{ row.value }}</dd></div></dl></div></div>
    <div v-if="!collapsed" class="absolute inset-y-0 left-0 z-10 w-1.5 cursor-col-resize transition-colors hover:bg-accent/50" @pointerdown="emit('resize', $event)"></div>
  </aside>
</template>
