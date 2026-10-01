<script setup lang="ts">
import { computed, ref } from "vue";
import { Heart, ListChecks, Play, Search } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { usePlayerStore } from "../stores/player";
import type { Track } from "../stores/player";
import { tracksForPaths, useProfileStore } from "../stores/profile";
import { sortTracks, useTrackSort } from "../lib/sort";
import TrackList from "../components/TrackList.vue";
import TrackContextMenu from "../components/TrackContextMenu.vue";
import TrackSortMenu from "../components/TrackSortMenu.vue";
import ViewModeToggle from "../components/ViewModeToggle.vue";

defineProps<{ loading: boolean; downloadingTrackId?: number | null }>();
const emit = defineEmits<{ openArtist: [artist: string]; openAlbum: [album: string]; downloadMetadata: [track: Track]; editLyrics: [track: Track]; editInfo: [track: Track]; showInfo: [track: Track] }>();
const { t } = useI18n();
const player = usePlayerStore();
const profile = useProfileStore();
const contextTrack = ref<Track | null>(null);
const contextPosition = ref({ x: 0, y: 0 });
const selectMode = ref(false);
const selected = ref<Set<string>>(new Set());
const query = ref("");
const scrollEl = ref<HTMLElement | null>(null);
const sourceTracks = computed(() => tracksForPaths(profile.favorites, player.tracks));
/** Source order with the chosen sort applied; the play queue follows this so "next" matches the screen. */
const { key: sortKey, dir: sortDir } = useTrackSort("favorites");
const orderedTracks = computed(() => sortTracks(sourceTracks.value, sortKey.value, sortDir.value));
const tracks = computed(() => {
  const value = query.value.trim().toLocaleLowerCase();
  return value ? orderedTracks.value.filter((track) => `${track.title} ${track.artist} ${track.album}`.toLocaleLowerCase().includes(value)) : orderedTracks.value;
});

function toggleSelectMode() {
  selectMode.value = !selectMode.value;
  if (!selectMode.value) selected.value = new Set();
}
function play(track: Track) { player.playInQueue(orderedTracks.value, track); }
function playFirst() {
  const pool = tracks.value.filter((track) => track.path);
  if (pool.length === 0) return;
  const first = player.shuffle ? pool[Math.floor(Math.random() * pool.length)] : pool[0];
  player.playInQueue(sourceTracks.value, first);
}
function openContextMenu(event: MouseEvent, track: Track) { contextTrack.value = track; contextPosition.value = { x: event.clientX, y: event.clientY }; }
function downloadMetadata(track: Track) { emit("downloadMetadata", track); }
</script>

<template>
  <div ref="scrollEl" class="h-full w-full overflow-y-auto">
    <div class="mx-auto flex w-full max-w-6xl flex-col px-4 py-5 sm:px-6 sm:py-6 lg:px-8 lg:py-8">
      <header class="shrink-0 border-b border-line pb-6">
        <div class="flex flex-col gap-4 sm:flex-row sm:flex-wrap sm:items-stretch sm:gap-6">
          <span class="ak-frame grid h-32 w-32 shrink-0 place-items-center border border-line bg-accent/10 text-accent"><Heart :size="46" :stroke-width="2" /></span>
          <div class="@container flex min-w-0 flex-col sm:flex-1">
            <h1 class="truncate text-3xl font-black leading-tight tracking-tight sm:text-4xl">{{ t("nav.favorites") }}</h1>
            <p class="mt-2 font-mono text-sm uppercase tracking-[0.2em] text-dim">{{ t("library.tracks", { count: tracks.length }) }}</p>
            <div class="mt-auto flex flex-wrap items-center gap-3 pt-4">
              <button type="button" class="ak-clip-tr order-1 flex h-10 shrink-0 items-center gap-2 bg-accent px-4 text-[13px] font-bold text-accent-fg transition-transform hover:scale-[1.02] active:scale-95 disabled:opacity-50" :disabled="!tracks.length" @click="playFirst"><Play :size="15" :stroke-width="2.2" />{{ t("library.play") }}</button>
              <label class="order-2 flex h-10 min-w-0 flex-1 items-center gap-2 border border-line bg-surface px-3 @2xl:order-3 @2xl:ml-auto @2xl:w-64 @2xl:flex-none"><Search :size="16" class="text-dim" /><input v-model="query" class="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-dim" :placeholder="t('library.search')" aria-label="Search library" /></label>
              <div class="order-3 flex w-full items-center gap-3 @2xl:order-4 @2xl:w-auto">
                <TrackSortMenu context="favorites" />
                <ViewModeToggle />
                <button type="button" class="grid h-10 w-10 shrink-0 place-items-center border transition-colors" :class="selectMode ? 'border-accent bg-accent text-accent-fg' : 'border-line text-dim hover:border-accent hover:text-accent'" :title="t('library.selectMode')" @click="toggleSelectMode"><ListChecks :size="16" :stroke-width="2" /></button>
              </div>
            </div>
          </div>
        </div>
      </header>

      <div class="mt-8 min-h-0 flex-1">
        <TrackList v-model:selected="selected" :tracks="tracks" :scroller="scrollEl" empty-key="favorites.empty" :selectable="selectMode" @play="play" @menu="openContextMenu" @open-artist="emit('openArtist', $event)" @open-album="emit('openAlbum', $event)" />
      </div>

      <TrackContextMenu v-if="contextTrack" :track="contextTrack" :x="contextPosition.x" :y="contextPosition.y" :selected-paths="[...selected]" :downloading="downloadingTrackId === contextTrack.id" @close="contextTrack = null" @download-metadata="downloadMetadata" @edit-lyrics="emit('editLyrics', $event)" @edit-info="emit('editInfo', $event)" @show-info="emit('showInfo', $event)" />
    </div>
  </div>
</template>
