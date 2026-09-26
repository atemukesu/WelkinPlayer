<script setup lang="ts">
import { computed, ref } from "vue";
import { Activity, Music2, Play, TrendingUp } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { usePlayerStore } from "../stores/player";
import type { Track } from "../stores/player";
import { useProfileStore } from "../stores/profile";
import { initial, pad } from "../lib/format";
import TrackContextMenu from "../components/TrackContextMenu.vue";

withDefaults(defineProps<{ loading: boolean; downloadingTrackId?: number | null }>(), { downloadingTrackId: null });
const emit = defineEmits<{ downloadMetadata: [track: Track]; editLyrics: [track: Track] }>();
const { t } = useI18n();
const player = usePlayerStore();
const profile = useProfileStore();
const contextTrack = ref<Track | null>(null);
const contextPosition = ref({ x: 0, y: 0 });

interface RankedTrack {
  track: Track;
  count: number;
}

const ranked = computed<RankedTrack[]>(() => {
  const counts = profile.profile.playCounts;
  return player.tracks
    .filter((track) => track.path && (counts[track.path] ?? 0) > 0)
    .map((track) => ({ track, count: counts[track.path as string] ?? 0 }))
    .sort((a, b) => b.count - a.count);
});
const queue = computed(() => ranked.value.flatMap((item) => (item.track.path ? [item.track] : [])));
const totalPlays = computed(() => ranked.value.reduce((sum, item) => sum + item.count, 0));
const maxCount = computed(() => ranked.value[0]?.count ?? 0);
const summary = computed(() => [
  { key: "stats.totalPlays", value: totalPlays.value, icon: Activity },
  { key: "stats.tracksPlayed", value: ranked.value.length, icon: TrendingUp },
  { key: "stats.libraryTracks", value: player.tracks.length, icon: Music2 },
]);

function play(item: RankedTrack) { player.playInQueue(queue.value, item.track); }
function playAll() {
  if (ranked.value.length === 0) return;
  const index = player.shuffle ? Math.floor(Math.random() * ranked.value.length) : 0;
  player.playInQueue(queue.value, ranked.value[index].track);
}
function openContextMenu(event: MouseEvent, track: Track) { contextTrack.value = track; contextPosition.value = { x: event.clientX, y: event.clientY }; }
</script>

<template>
  <div class="mx-auto w-full max-w-6xl px-6 py-6 lg:px-8 lg:py-8">
    <header class="border-b border-line pb-6">
      <div class="flex flex-wrap items-center gap-6">
        <span class="ak-frame grid h-32 w-32 shrink-0 place-items-center border border-line bg-accent/10 text-accent"><Activity :size="46" :stroke-width="2" /></span>
        <div class="min-w-0">
          <p class="flex items-center gap-2 text-[13px] font-semibold uppercase tracking-[0.35em] text-accent"><span class="h-2 w-2 bg-accent"></span>{{ t("stats.eyebrow") }}</p>
          <h1 class="mt-4 truncate text-4xl font-black leading-none tracking-tight sm:text-5xl">{{ t("stats.title") }}</h1>
          <p class="mt-3 max-w-xl text-sm text-muted">{{ t("stats.subtitle") }}</p>
        </div>
        <button type="button" class="ak-clip-tr ml-auto flex h-10 items-center gap-2 bg-accent px-4 text-[13px] font-bold text-accent-fg transition-transform hover:scale-[1.02] active:scale-95 disabled:opacity-50" :disabled="!ranked.length" @click="playAll"><Play :size="15" :stroke-width="2.2" />{{ t("stats.playAll") }}</button>
      </div>
    </header>

    <section class="mt-6 grid grid-cols-1 gap-4 sm:grid-cols-3">
      <div v-for="item in summary" :key="item.key" class="ak-frame flex items-center gap-4 border border-line bg-surface p-4">
        <span class="grid h-10 w-10 shrink-0 place-items-center bg-accent/10 text-accent"><component :is="item.icon" :size="18" :stroke-width="2" /></span>
        <div class="min-w-0">
          <p class="font-mono text-2xl font-black tabular-nums leading-none">{{ item.value }}</p>
          <p class="mt-1.5 truncate text-[11px] uppercase tracking-[0.2em] text-dim">{{ t(item.key) }}</p>
        </div>
      </div>
    </section>

    <section class="mt-10">
      <div class="mb-4 flex items-end justify-between">
        <h2 class="flex items-center gap-3 text-sm font-bold uppercase tracking-[0.25em]"><span class="h-3 w-1 bg-accent"></span>{{ t("stats.title") }}</h2>
        <span class="font-mono text-[12px] uppercase tracking-[0.2em] text-dim">{{ ranked.length }}</span>
      </div>

      <div v-if="player.tracks.length === 0" class="ak-frame grid place-items-center gap-4 border border-dashed border-line bg-surface/50 px-6 py-20 text-center">
        <template v-if="loading"><span class="ak-pulse"></span><p class="text-sm font-semibold uppercase tracking-[0.2em] text-muted">{{ t("library.loading") }}</p></template>
        <p v-else class="max-w-md text-sm text-muted">{{ t("library.emptyDesc") }}</p>
      </div>
      <p v-else-if="ranked.length === 0" class="border border-dashed border-line bg-surface/50 px-6 py-20 text-center text-sm text-muted">{{ t("stats.empty") }}</p>

      <div v-else class="border border-line bg-surface">
        <div
          v-for="(item, index) in ranked"
          :key="item.track.id"
          role="button"
          tabindex="0"
          class="group grid cursor-pointer grid-cols-[40px_44px_minmax(0,1fr)_auto] items-center gap-4 border-t border-line px-3 py-2 text-left first:border-t-0"
          :class="item.track.id === player.currentTrack?.id ? 'ak-select relative z-10 border-transparent' : 'hover:bg-fg/5'"
          @click="play(item)"
          @keydown.enter="play(item)"
          @keydown.space.prevent="play(item)"
          @contextmenu.prevent="openContextMenu($event, item.track)"
        >
          <span class="font-mono text-sm font-bold tabular-nums" :class="index < 3 ? 'text-accent' : 'text-dim'">{{ pad(index + 1) }}</span>
          <span class="grid h-11 w-11 place-items-center overflow-hidden text-lg font-black text-white/90" :style="{ backgroundColor: item.track.color }"><img v-if="item.track.cover" :src="item.track.cover" alt="" loading="lazy" decoding="async" class="h-full w-full object-cover" /><template v-else>{{ initial(item.track) }}</template></span>
          <span class="grid min-w-0 gap-1.5">
            <span class="flex min-w-0 items-baseline gap-2"><strong class="truncate text-sm font-semibold tracking-wide" :class="item.track.id === player.currentTrack?.id ? 'text-accent' : 'text-fg'">{{ item.track.title }}</strong><small class="truncate text-xs text-muted">{{ item.track.artist }}</small></span>
            <span class="block h-1 w-full overflow-hidden bg-fg/5"><span class="block h-full transition-[width]" :class="item.track.id === player.currentTrack?.id ? 'bg-white' : 'bg-accent'" :style="{ width: `${maxCount ? (item.count / maxCount) * 100 : 0}%` }"></span></span>
          </span>
          <span class="flex items-baseline gap-1 whitespace-nowrap"><span class="font-mono text-sm tabular-nums text-fg">{{ item.count }}</span><span class="text-[10px] uppercase tracking-[0.15em] text-dim">{{ t("stats.unit") }}</span></span>
        </div>
      </div>
    </section>

    <TrackContextMenu v-if="contextTrack" :track="contextTrack" :x="contextPosition.x" :y="contextPosition.y" :downloading="downloadingTrackId === contextTrack.id" @close="contextTrack = null" @download-metadata="emit('downloadMetadata', $event)" @edit-lyrics="emit('editLyrics', $event)" />
  </div>
</template>
