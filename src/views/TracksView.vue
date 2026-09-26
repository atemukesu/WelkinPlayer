<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { ListChecks, Pencil, Play, RefreshCw, Search } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { usePlayerStore } from "../stores/player";
import type { Track } from "../stores/player";
import { tracksForPaths, useProfileStore } from "../stores/profile";
import TrackList from "../components/TrackList.vue";
import TrackContextMenu from "../components/TrackContextMenu.vue";
import ViewModeToggle from "../components/ViewModeToggle.vue";
import PlaylistCover from "../components/PlaylistCover.vue";

const props = withDefaults(defineProps<{ playlistId?: string | null; loading: boolean; enriching: boolean; refreshing?: boolean; enrichDone: number; enrichTotal: number; downloadingTrackId?: number | null }>(), { playlistId: null, refreshing: false, downloadingTrackId: null });
const emit = defineEmits<{ refresh: []; edit: [id: string]; downloadMetadata: [track: Track]; editLyrics: [track: Track]; editInfo: [track: Track]; showInfo: [track: Track] }>();
const { t } = useI18n();
const player = usePlayerStore();
const profile = useProfileStore();
const query = ref("");
const contextTrack = ref<Track | null>(null);
const contextPosition = ref({ x: 0, y: 0 });
const selectMode = ref(false);
const selected = ref<Set<string>>(new Set());
const scrollEl = ref<HTMLElement | null>(null);

const activePlaylist = computed(() => props.playlistId ? profile.playlists.find((item) => item.id === props.playlistId) ?? null : null);
const sourceTracks = computed(() => activePlaylist.value ? tracksForPaths(activePlaylist.value.tracks, player.tracks) : player.tracks);
const tracks = computed(() => {
  const value = query.value.trim().toLocaleLowerCase();
  return value ? sourceTracks.value.filter((track) => `${track.title} ${track.artist} ${track.album}`.toLocaleLowerCase().includes(value)) : sourceTracks.value;
});

watch(() => props.playlistId, () => { selectMode.value = false; selected.value = new Set(); });

function toggleSelectMode() {
  selectMode.value = !selectMode.value;
  if (!selectMode.value) selected.value = new Set();
}
function play(track: Track) { player.playInQueue(sourceTracks.value, track); }
function playFirst() {
  const pool = tracks.value.filter((track) => track.path);
  if (pool.length === 0) return;
  const first = player.shuffle ? pool[Math.floor(Math.random() * pool.length)] : pool[0];
  player.playInQueue(sourceTracks.value, first);
}
function remove(track: Track) { if (activePlaylist.value && track.path) profile.removeFromPlaylist(activePlaylist.value.id, track.path); }
function openContextMenu(event: MouseEvent, track: Track) { contextTrack.value = track; contextPosition.value = { x: event.clientX, y: event.clientY }; }
function downloadMetadata(track: Track) { emit("downloadMetadata", track); }
</script>

<template>
  <div ref="scrollEl" class="h-full w-full overflow-y-auto">
    <div class="mx-auto flex w-full max-w-6xl flex-col px-4 py-5 sm:px-6 sm:py-6 lg:px-8 lg:py-8">
      <header class="shrink-0 border-b border-line pb-6">
        <div class="flex flex-col gap-4 sm:flex-row sm:flex-wrap sm:items-stretch sm:gap-6">
          <PlaylistCover v-if="activePlaylist" :playlist="activePlaylist" class="ak-frame h-32 w-32 shrink-0 border border-line text-5xl" :icon-size="40" />
          <div class="flex min-w-0 flex-col sm:flex-1">
            <template v-if="activePlaylist">
              <h1 class="truncate text-3xl font-black leading-tight tracking-tight sm:text-4xl">{{ activePlaylist.name }}</h1>
              <p class="mt-2 font-mono text-sm uppercase tracking-[0.2em] text-dim">{{ t("library.playlists.count", { count: activePlaylist.tracks.length }) }}</p>
            </template>
            <template v-else>
              <p class="flex items-center gap-2 text-[13px] font-semibold uppercase tracking-[0.35em] text-accent"><span class="h-2 w-2 bg-accent"></span>{{ t("nav.tracks") }}</p>
              <h1 class="mt-4 truncate text-4xl font-black leading-none tracking-tight sm:text-5xl">{{ t("nav.tracks") }}</h1>
              <p class="mt-3 font-mono text-sm uppercase tracking-[0.2em] text-dim">{{ t("library.tracks", { count: player.tracks.length }) }}</p>
            </template>

            <div class="mt-auto flex flex-wrap items-center gap-3 pt-4">
              <template v-if="activePlaylist">
                <button type="button" class="ak-clip-tr flex h-10 items-center gap-2 bg-accent px-4 text-[13px] font-bold text-accent-fg transition-transform hover:scale-[1.02] active:scale-95 disabled:opacity-50" :disabled="!tracks.length" @click="playFirst"><Play :size="15" :stroke-width="2.2" />{{ t("library.play") }}</button>
                <button type="button" class="ak-clip-tr flex h-10 items-center gap-2 border border-line px-4 text-[13px] font-semibold transition-colors hover:border-accent hover:text-accent" @click="emit('edit', activePlaylist.id)"><Pencil :size="15" />{{ t("library.edit") }}</button>
              </template>
              <div class="ml-auto flex w-full items-center gap-3 sm:w-auto">
                <label class="flex h-10 min-w-0 flex-1 items-center gap-2 border border-line bg-surface px-3 sm:w-64"><Search :size="16" class="text-dim" /><input v-model="query" class="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-dim" :placeholder="t('library.search')" aria-label="Search library" /></label>
                <button v-if="!activePlaylist" type="button" class="grid h-10 w-10 shrink-0 place-items-center border border-line text-dim transition-colors hover:border-accent hover:text-accent disabled:opacity-50" :title="t('library.refresh')" :disabled="refreshing" @click="emit('refresh')"><RefreshCw :size="16" :stroke-width="2" :class="refreshing ? 'animate-spin' : ''" /></button>
                <ViewModeToggle />
                <button type="button" class="grid h-10 w-10 shrink-0 place-items-center border transition-colors" :class="selectMode ? 'border-accent bg-accent text-accent-fg' : 'border-line text-dim hover:border-accent hover:text-accent'" :title="t('library.selectMode')" @click="toggleSelectMode"><ListChecks :size="16" :stroke-width="2" /></button>
              </div>
            </div>
          </div>
        </div>
      </header>

      <div v-if="enriching" class="mb-3 mt-8 flex shrink-0 items-center gap-2 font-mono text-[11px] uppercase tracking-[0.2em] text-accent"><span class="ak-pulse" style="width: 12px; height: 12px"></span>{{ t("library.enriching", { done: enrichDone, total: enrichTotal }) }}</div>

      <div v-if="player.tracks.length === 0" class="ak-frame grid shrink-0 place-items-center gap-4 border border-dashed border-line bg-surface/50 px-6 py-20 text-center">
        <template v-if="loading"><span class="ak-pulse"></span><p class="text-sm font-semibold uppercase tracking-[0.2em] text-muted">{{ t("library.loading") }}</p></template>
        <template v-else><p class="max-w-md text-sm text-muted">{{ t("library.emptyDesc") }}</p></template>
      </div>
      <div v-else class="min-h-0 flex-1"><TrackList v-model:selected="selected" :tracks="tracks" :scroller="scrollEl" empty-key="library.playlists.noTracks" :removable="!!activePlaylist" :selectable="selectMode" @play="play" @menu="openContextMenu" @remove="remove" /></div>

      <TrackContextMenu v-if="contextTrack" :track="contextTrack" :x="contextPosition.x" :y="contextPosition.y" :selected-paths="[...selected]" :playlist-id="activePlaylist?.id ?? null" :downloading="downloadingTrackId === contextTrack.id" @close="contextTrack = null" @download-metadata="downloadMetadata" @edit-lyrics="emit('editLyrics', $event)" @edit-info="emit('editInfo', $event)" @show-info="emit('showInfo', $event)" />
    </div>
  </div>
</template>
