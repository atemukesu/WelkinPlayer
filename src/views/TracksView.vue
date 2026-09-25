<script setup lang="ts">
import { computed, ref } from "vue";
import { Search } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { usePlayerStore } from "../stores/player";
import type { Track } from "../stores/player";
import { tracksForPaths, useProfileStore } from "../stores/profile";
import TrackList from "../components/TrackList.vue";
import TrackContextMenu from "../components/TrackContextMenu.vue";
import ViewModeToggle from "../components/ViewModeToggle.vue";

const props = withDefaults(defineProps<{ playlistId?: string | null; loading: boolean; enriching: boolean; enrichDone: number; enrichTotal: number; downloadingTrackId?: number | null }>(), { playlistId: null, downloadingTrackId: null });
const emit = defineEmits<{ details: [track: Track]; downloadMetadata: [track: Track] }>();
const { t } = useI18n();
const player = usePlayerStore();
const profile = useProfileStore();
const query = ref("");
const contextTrack = ref<Track | null>(null);
const contextPosition = ref({ x: 0, y: 0 });

const activePlaylist = computed(() => props.playlistId ? profile.playlists.find((item) => item.id === props.playlistId) ?? null : null);
const sourceTracks = computed(() => activePlaylist.value ? tracksForPaths(activePlaylist.value.tracks, player.tracks) : player.tracks);
const tracks = computed(() => {
  const value = query.value.trim().toLocaleLowerCase();
  return value ? sourceTracks.value.filter((track) => `${track.title} ${track.artist} ${track.album}`.toLocaleLowerCase().includes(value)) : sourceTracks.value;
});

function play(track: Track) { player.selectTrack(track); }
function remove(track: Track) { if (activePlaylist.value && track.path) profile.removeFromPlaylist(activePlaylist.value.id, track.path); }
function openContextMenu(event: MouseEvent, track: Track) { const width = 224; const height = 240; contextTrack.value = track; contextPosition.value = { x: Math.min(event.clientX, window.innerWidth - width - 8), y: Math.min(event.clientY, window.innerHeight - height - 8) }; }
function downloadMetadata(track: Track) { emit("downloadMetadata", track); }
</script>

<template>
  <div class="mx-auto w-full max-w-6xl px-6 py-6 lg:px-8 lg:py-8">
    <header class="border-b border-line pb-6">
      <div class="flex flex-wrap items-end justify-between gap-6">
        <div class="min-w-0">
          <p v-if="!activePlaylist" class="flex items-center gap-2 text-[13px] font-semibold uppercase tracking-[0.35em] text-accent"><span class="h-2 w-2 bg-accent"></span>{{ t("nav.tracks") }}</p>
          <h1 class="truncate text-4xl font-black leading-none tracking-tight sm:text-5xl" :class="activePlaylist ? '' : 'mt-4'">{{ activePlaylist ? activePlaylist.name : t("nav.tracks") }}</h1>
          <p class="mt-3 font-mono text-sm uppercase tracking-[0.2em] text-dim">{{ activePlaylist ? t("library.playlists.count", { count: activePlaylist.tracks.length }) : t("library.tracks", { count: player.tracks.length }) }}</p>
        </div>
        <div class="flex w-full items-center gap-3 sm:w-auto"><label class="flex h-10 flex-1 items-center gap-2 border border-line bg-surface px-3 sm:w-72"><Search :size="16" class="text-dim" /><input v-model="query" class="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-dim" :placeholder="t('library.search')" aria-label="Search library" /></label><ViewModeToggle /></div>
      </div>
    </header>

    <div v-if="enriching" class="mb-3 mt-8 flex items-center gap-2 font-mono text-[11px] uppercase tracking-[0.2em] text-accent"><span class="ak-pulse" style="width: 14px; height: 9px"></span>{{ t("library.enriching", { done: enrichDone, total: enrichTotal }) }}</div>

    <div v-if="player.tracks.length === 0" class="ak-frame grid place-items-center gap-4 border border-dashed border-line bg-surface/50 px-6 py-20 text-center">
      <template v-if="loading"><span class="ak-pulse"></span><p class="text-sm font-semibold uppercase tracking-[0.2em] text-muted">{{ t("library.loading") }}</p></template>
      <template v-else><p class="max-w-md text-sm text-muted">{{ t("library.emptyDesc") }}</p></template>
    </div>
    <TrackList v-else :tracks="tracks" empty-key="library.playlists.noTracks" :removable="!!activePlaylist" @play="play" @menu="openContextMenu" @details="emit('details', $event)" @remove="remove" />

    <TrackContextMenu v-if="contextTrack" :track="contextTrack" :x="contextPosition.x" :y="contextPosition.y" :downloading="downloadingTrackId === contextTrack.id" @close="contextTrack = null" @download-metadata="downloadMetadata" />
  </div>
</template>
