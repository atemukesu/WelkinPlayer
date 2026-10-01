<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { ListChecks, Pencil, Play, RefreshCw, Search } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { usePlayerStore } from "../stores/player";
import type { Track } from "../stores/player";
import { tracksForPaths, useProfileStore } from "../stores/profile";
import { useSourcesStore } from "../stores/sources";
import { groupKeys, isUnknownGroup, type GroupKind } from "../lib/grouping";
import { sortTracks, useTrackSort, type TrackSortContext } from "../lib/sort";
import TrackList from "../components/TrackList.vue";
import TrackContextMenu from "../components/TrackContextMenu.vue";
import TrackSortMenu from "../components/TrackSortMenu.vue";
import ViewModeToggle from "../components/ViewModeToggle.vue";
import PlaylistCover from "../components/PlaylistCover.vue";

const props = withDefaults(defineProps<{ playlistId?: string | null; artist?: string | null; album?: string | null; sourceId?: string | null; loading: boolean; enriching: boolean; refreshing?: boolean; enrichDone: number; enrichTotal: number; downloadingTrackId?: number | null }>(), { playlistId: null, artist: null, album: null, sourceId: null, refreshing: false, downloadingTrackId: null });
const emit = defineEmits<{ refresh: []; edit: [id: string]; openArtist: [artist: string]; openAlbum: [album: string]; downloadMetadata: [track: Track]; editLyrics: [track: Track]; editInfo: [track: Track]; showInfo: [track: Track] }>();
const { t } = useI18n();
const player = usePlayerStore();
const profile = useProfileStore();
const sources = useSourcesStore();
const query = ref("");
const contextTrack = ref<Track | null>(null);
const contextPosition = ref({ x: 0, y: 0 });
const selectMode = ref(false);
const selected = ref<Set<string>>(new Set());
const scrollEl = ref<HTMLElement | null>(null);

const activePlaylist = computed(() => props.playlistId ? profile.playlists.find((item) => item.id === props.playlistId) ?? null : null);
/** A generated artist/album collection, selected from the classification views. */
const groupKind = computed<GroupKind>(() => (props.artist ? "artists" : "albums"));
const groupKeyValue = computed(() => props.artist ?? props.album ?? null);
const isGroup = computed(() => groupKeyValue.value !== null);
const sourceFilter = computed(() => props.sourceId ?? null);
const sourceName = computed(() => sources.sources.find((source) => source.id === sourceFilter.value)?.name ?? null);
const groupTracks = computed(() => {
  const key = groupKeyValue.value;
  if (key === null) return null;
  const kind = groupKind.value;
  return player.tracks.filter((track) => groupKeys(track, kind).includes(key));
});
const groupTitle = computed(() => {
  const key = groupKeyValue.value;
  if (key === null) return "";
  if (isUnknownGroup(key)) return t(props.artist ? "library.collections.unknownArtist" : "library.collections.unknownAlbum");
  return key;
});
/** Representative cover for a generated collection: the first track that has artwork. */
const groupCoverTrack = computed(() => groupTracks.value?.find((track) => track.cover) ?? groupTracks.value?.[0]);
const sourceTracks = computed(() => {
  if (activePlaylist.value) return tracksForPaths(activePlaylist.value.tracks, player.tracks);
  if (groupTracks.value) return groupTracks.value;
  if (sourceFilter.value) return player.tracks.filter((track) => track.sourceId === sourceFilter.value);
  return player.tracks;
});
/** Sort settings are per page: playlists, artists, albums and the full library remember their own. */
const sortContext = computed<TrackSortContext>(() => activePlaylist.value ? "playlist" : isGroup.value ? (groupKind.value === "artists" ? "artist" : "album") : "tracks");
const sortKey = computed(() => useTrackSort(sortContext.value).key.value);
const sortDir = computed(() => useTrackSort(sortContext.value).dir.value);
/** Source order with the chosen sort applied; the play queue follows this so "next" matches the screen. */
const orderedTracks = computed(() => sortTracks(sourceTracks.value, sortKey.value, sortDir.value));
const tracks = computed(() => {
  const value = query.value.trim().toLocaleLowerCase();
  return value ? orderedTracks.value.filter((track) => `${track.title} ${track.artist} ${track.album}`.toLocaleLowerCase().includes(value)) : orderedTracks.value;
});

watch([() => props.playlistId, () => props.artist, () => props.album], () => { selectMode.value = false; selected.value = new Set(); });

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
          <PlaylistCover v-if="activePlaylist" :playlist="activePlaylist" class="ak-frame h-32 w-32 shrink-0 border border-line text-5xl" :icon-size="40" />
          <span v-else-if="isGroup" class="ak-frame grid h-32 w-32 shrink-0 place-items-center overflow-hidden border border-line text-5xl font-black text-white/90" :style="{ backgroundColor: groupCoverTrack?.color ?? 'var(--accent)' }">
            <img v-if="groupCoverTrack?.cover" :src="groupCoverTrack.cover" alt="" decoding="async" class="h-full w-full object-cover" />
            <template v-else>{{ groupTitle.charAt(0) }}</template>
          </span>
          <div class="@container flex min-w-0 flex-col sm:flex-1">
            <template v-if="activePlaylist">
              <h1 class="truncate text-3xl font-black leading-tight tracking-tight sm:text-4xl">{{ activePlaylist.name }}</h1>
              <p class="mt-2 font-mono text-sm uppercase tracking-[0.2em] text-dim">{{ t("library.playlists.count", { count: activePlaylist.tracks.length }) }}</p>
            </template>
            <template v-else-if="isGroup">
              <p class="flex items-center gap-2 text-[13px] font-semibold uppercase tracking-[0.35em] text-accent"><span class="h-2 w-2 bg-accent"></span>{{ t(groupKind === "artists" ? "nav.artists" : "nav.albums") }}</p>
              <h1 class="mt-4 truncate text-3xl font-black leading-tight tracking-tight sm:text-4xl">{{ groupTitle }}</h1>
              <p class="mt-3 font-mono text-sm uppercase tracking-[0.2em] text-dim">{{ t("library.tracks", { count: sourceTracks.length }) }}</p>
            </template>
            <template v-else>
              <p class="flex items-center gap-2 text-[13px] font-semibold uppercase tracking-[0.35em] text-accent"><span class="h-2 w-2 bg-accent"></span>{{ sourceFilter ? t("nav.sources") : t("nav.tracks") }}</p>
              <h1 class="mt-4 truncate text-4xl font-black leading-none tracking-tight sm:text-5xl">{{ sourceFilter ? (sourceName ?? t("nav.tracks")) : t("nav.tracks") }}</h1>
              <p class="mt-3 font-mono text-sm uppercase tracking-[0.2em] text-dim">{{ t("library.tracks", { count: sourceTracks.length }) }}</p>
            </template>

            <div class="mt-auto flex flex-wrap items-center gap-3 pt-4">
              <div class="flex w-full min-w-0 items-center gap-3 @2xl:order-2 @2xl:ml-auto @2xl:w-auto" :class="activePlaylist ? 'order-2' : 'order-1'">
                <label class="flex h-10 min-w-0 flex-1 items-center gap-2 border border-line bg-surface px-3 @2xl:w-64 @2xl:flex-none"><Search :size="16" class="text-dim" /><input v-model="query" class="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-dim" :placeholder="t('library.search')" aria-label="Search library" /></label>
                <button v-if="!activePlaylist && !isGroup" type="button" class="grid h-10 w-10 shrink-0 place-items-center border border-line text-dim transition-colors hover:border-accent hover:text-accent disabled:opacity-50" :title="t('library.refresh')" :disabled="refreshing" @click="emit('refresh')"><RefreshCw :size="16" :stroke-width="2" :class="refreshing ? 'animate-spin' : ''" /></button>
              </div>
              <div v-if="activePlaylist || isGroup" class="flex items-center gap-3 @2xl:order-1" :class="activePlaylist ? 'order-1' : 'order-2'">
                <template v-if="activePlaylist">
                  <button type="button" class="ak-clip-tr flex h-10 shrink-0 items-center gap-2 bg-accent px-4 text-[13px] font-bold text-accent-fg transition-transform hover:scale-[1.02] active:scale-95 disabled:opacity-50" :disabled="!tracks.length" @click="playFirst"><Play :size="15" :stroke-width="2.2" />{{ t("library.play") }}</button>
                  <button type="button" class="ak-clip-tr flex h-10 shrink-0 items-center gap-2 border border-line px-4 text-[13px] font-semibold transition-colors hover:border-accent hover:text-accent" @click="emit('edit', activePlaylist.id)"><Pencil :size="15" />{{ t("library.edit") }}</button>
                </template>
                <button v-else type="button" class="ak-clip-tr flex h-10 shrink-0 items-center gap-2 bg-accent px-4 text-[13px] font-bold text-accent-fg transition-transform hover:scale-[1.02] active:scale-95 disabled:opacity-50" :disabled="!tracks.length" @click="playFirst"><Play :size="15" :stroke-width="2.2" />{{ t("library.collections.playAll") }}</button>
              </div>
              <div class="order-3 flex items-center gap-3">
                <TrackSortMenu :context="sortContext" />
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
      <div v-else class="min-h-0 flex-1"><TrackList v-model:selected="selected" :tracks="tracks" :scroller="scrollEl" empty-key="library.playlists.noTracks" :selectable="selectMode" @play="play" @menu="openContextMenu" @open-artist="emit('openArtist', $event)" @open-album="emit('openAlbum', $event)" /></div>

      <TrackContextMenu v-if="contextTrack" :track="contextTrack" :x="contextPosition.x" :y="contextPosition.y" :selected-paths="[...selected]" :playlist-id="activePlaylist?.id ?? null" :downloading="downloadingTrackId === contextTrack.id" @close="contextTrack = null" @download-metadata="downloadMetadata" @edit-lyrics="emit('editLyrics', $event)" @edit-info="emit('editInfo', $event)" @show-info="emit('showInfo', $event)" />
    </div>
  </div>
</template>
