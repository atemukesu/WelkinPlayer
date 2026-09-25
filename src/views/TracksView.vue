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
const emit = defineEmits<{ refresh: []; edit: [id: string]; details: [track: Track]; downloadMetadata: [track: Track] }>();
const { t } = useI18n();
const player = usePlayerStore();
const profile = useProfileStore();
const query = ref("");
const contextTrack = ref<Track | null>(null);
const contextPosition = ref({ x: 0, y: 0 });
const selectMode = ref(false);
const selected = ref<Set<string>>(new Set());

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
function toggleSelect(track: Track) {
  if (!track.path) return;
  const next = new Set(selected.value);
  if (next.has(track.path)) next.delete(track.path);
  else next.add(track.path);
  selected.value = next;
}

function play(track: Track) { player.selectTrack(track); }
function playFirst() { const first = tracks.value.find((track) => track.path); if (first) player.selectTrack(first); }
function remove(track: Track) { if (activePlaylist.value && track.path) profile.removeFromPlaylist(activePlaylist.value.id, track.path); }
function openContextMenu(event: MouseEvent, track: Track) { const width = 224; const height = 240; contextTrack.value = track; contextPosition.value = { x: Math.min(event.clientX, window.innerWidth - width - 8), y: Math.min(event.clientY, window.innerHeight - height - 8) }; }
function downloadMetadata(track: Track) { emit("downloadMetadata", track); }
</script>

<template>
  <div class="mx-auto flex h-full w-full max-w-6xl flex-col px-6 py-6 lg:px-8 lg:py-8">
    <header class="shrink-0 border-b border-line pb-6">
      <div class="flex flex-wrap items-center gap-6">
        <template v-if="activePlaylist">
          <PlaylistCover :playlist="activePlaylist" class="ak-frame h-32 w-32 shrink-0 border border-line text-5xl" :icon-size="40" />
          <div class="min-w-0">
            <h1 class="truncate text-3xl font-black leading-tight tracking-tight sm:text-4xl">{{ activePlaylist.name }}</h1>
            <p class="mt-2 font-mono text-sm uppercase tracking-[0.2em] text-dim">{{ t("library.playlists.count", { count: activePlaylist.tracks.length }) }}</p>
            <div class="mt-4 flex items-center gap-2">
              <button type="button" class="ak-clip-tr flex h-10 items-center gap-2 bg-accent px-4 text-[13px] font-bold text-accent-fg transition-transform hover:scale-[1.02] active:scale-95 disabled:opacity-50" :disabled="!tracks.length" @click="playFirst"><Play :size="15" :stroke-width="2.2" />{{ t("library.play") }}</button>
              <button type="button" class="ak-clip-tr flex h-10 items-center gap-2 border border-line px-4 text-[13px] font-semibold transition-colors hover:border-accent hover:text-accent" @click="emit('edit', activePlaylist.id)"><Pencil :size="15" />{{ t("library.edit") }}</button>
            </div>
          </div>
        </template>

        <div v-else class="min-w-0">
          <p class="flex items-center gap-2 text-[13px] font-semibold uppercase tracking-[0.35em] text-accent"><span class="h-2 w-2 bg-accent"></span>{{ t("nav.tracks") }}</p>
          <h1 class="mt-4 truncate text-4xl font-black leading-none tracking-tight sm:text-5xl">{{ t("nav.tracks") }}</h1>
          <p class="mt-3 font-mono text-sm uppercase tracking-[0.2em] text-dim">{{ t("library.tracks", { count: player.tracks.length }) }}</p>
        </div>

        <div class="ml-auto flex w-full items-center gap-3 sm:w-auto">
          <label class="flex h-10 flex-1 items-center gap-2 border border-line bg-surface px-3 sm:w-64"><Search :size="16" class="text-dim" /><input v-model="query" class="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-dim" :placeholder="t('library.search')" aria-label="Search library" /></label>
          <button v-if="!activePlaylist" type="button" class="grid h-10 w-10 shrink-0 place-items-center border border-line text-dim transition-colors hover:border-accent hover:text-accent disabled:opacity-50" :title="t('library.refresh')" :disabled="refreshing" @click="emit('refresh')"><RefreshCw :size="16" :stroke-width="2" :class="refreshing ? 'animate-spin' : ''" /></button>
          <ViewModeToggle />
          <button type="button" class="grid h-10 w-10 shrink-0 place-items-center border transition-colors" :class="selectMode ? 'border-accent bg-accent text-accent-fg' : 'border-line text-dim hover:border-accent hover:text-accent'" :title="t('library.selectMode')" @click="toggleSelectMode"><ListChecks :size="16" :stroke-width="2" /></button>
        </div>
      </div>
    </header>

    <div v-if="enriching" class="mb-3 mt-8 flex shrink-0 items-center gap-2 font-mono text-[11px] uppercase tracking-[0.2em] text-accent"><span class="ak-pulse" style="width: 14px; height: 9px"></span>{{ t("library.enriching", { done: enrichDone, total: enrichTotal }) }}</div>

    <div v-if="player.tracks.length === 0" class="ak-frame grid shrink-0 place-items-center gap-4 border border-dashed border-line bg-surface/50 px-6 py-20 text-center">
      <template v-if="loading"><span class="ak-pulse"></span><p class="text-sm font-semibold uppercase tracking-[0.2em] text-muted">{{ t("library.loading") }}</p></template>
      <template v-else><p class="max-w-md text-sm text-muted">{{ t("library.emptyDesc") }}</p></template>
    </div>
    <div v-else class="min-h-0 flex-1"><TrackList :tracks="tracks" empty-key="library.playlists.noTracks" :removable="!!activePlaylist" :selectable="selectMode" :selected="selected" @play="play" @menu="openContextMenu" @details="emit('details', $event)" @remove="remove" @toggle="toggleSelect" /></div>

    <TrackContextMenu v-if="contextTrack" :track="contextTrack" :x="contextPosition.x" :y="contextPosition.y" :selected-paths="[...selected]" :playlist-id="activePlaylist?.id ?? null" :downloading="downloadingTrackId === contextTrack.id" @close="contextTrack = null" @download-metadata="downloadMetadata" />
  </div>
</template>
