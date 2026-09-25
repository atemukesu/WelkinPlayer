<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { Component } from "vue";
import { Activity, AudioLines, Ellipsis, Heart, ImagePlus, Library, ListMusic, Play, Plus, RefreshCw, Shuffle, Trash2, X } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { usePlayerStore } from "../stores/player";
import type { Track } from "../stores/player";
import { tracksForPaths, useProfileStore } from "../stores/profile";
import { initial } from "../lib/format";
import type { View } from "../lib/app";
import PlaylistCover from "../components/PlaylistCover.vue";
import TrackCard from "../components/TrackCard.vue";
import TrackContextMenu from "../components/TrackContextMenu.vue";

withDefaults(defineProps<{ loading: boolean; enriching: boolean; enrichDone: number; enrichTotal: number; downloadingTrackId?: number | null }>(), { downloadingTrackId: null });
const emit = defineEmits<{ settings: []; details: [track: Track]; downloadMetadata: [track: Track]; navigate: [view: View]; openPlaylist: [id: string]; editPlaylist: [id: string] }>();
const { t } = useI18n();
const player = usePlayerStore();
const profile = useProfileStore();
const contextTrack = ref<Track | null>(null);
const contextPosition = ref({ x: 0, y: 0 });
const recommended = ref<Track[]>([]);
const creating = ref(false);
const newName = ref("");
const editingId = ref<string | null>(null);
const editingName = ref("");
const pendingDelete = ref<string | null>(null);

const RECOMMEND_LIMIT = 5;
const TOP_LIMIT = 5;

const greetingKey = computed(() => {
  const hour = new Date().getHours();
  if (hour < 6) return "home.greetingNight";
  if (hour < 11) return "home.greetingMorning";
  if (hour < 14) return "home.greetingNoon";
  if (hour < 18) return "home.greetingAfternoon";
  return "home.greetingEvening";
});
const greetingText = computed(() => profile.nickname ? `${t(greetingKey.value)}，${profile.nickname}` : t(greetingKey.value));

const heroTrack = computed(() => player.currentTrack);
const totalPlays = computed(() => Object.values(profile.profile.playCounts).reduce((sum, value) => sum + value, 0));
const stats = computed<{ key: string; value: number; icon: Component; view?: View }[]>(() => [
  { key: "home.statsTracks", value: player.tracks.length, icon: Library, view: "tracks" },
  { key: "home.statsPlaylists", value: profile.playlists.length, icon: ListMusic },
  { key: "home.statsFavorites", value: profile.favorites.length, icon: Heart, view: "favorites" },
  { key: "home.statsPlays", value: totalPlays.value, icon: Activity },
]);

const topTracks = computed(() => {
  const counts = profile.profile.playCounts;
  return [...player.tracks]
    .filter((track) => track.path && (counts[track.path] ?? 0) > 0)
    .sort((a, b) => (counts[b.path as string] ?? 0) - (counts[a.path as string] ?? 0))
    .slice(0, TOP_LIMIT);
});
const recentTracks = computed(() => tracksForPaths(profile.recent, player.tracks).slice(0, RECOMMEND_LIMIT));
const favoriteTracks = computed(() => tracksForPaths(profile.favorites, player.tracks));

function pickRandom(list: Track[], count: number): Track[] {
  const pool = list.filter((track) => track.path);
  const used = new Set<number>();
  const result: Track[] = [];
  while (result.length < count && used.size < pool.length) {
    const index = Math.floor(Math.random() * pool.length);
    if (used.has(index)) continue;
    used.add(index);
    result.push(pool[index]);
  }
  return result;
}
function refreshRecommended() { recommended.value = pickRandom(player.tracks, RECOMMEND_LIMIT); }
watch(() => player.tracks, refreshRecommended, { immediate: true });

function play(track: Track) { player.selectTrack(track); }
function shuffleAll() {
  const pool = player.tracks.filter((track) => track.path);
  if (pool.length === 0) return;
  play(pool[Math.floor(Math.random() * pool.length)]);
}
function openContextMenu(event: MouseEvent, track: Track) { const width = 224; const height = 240; contextTrack.value = track; contextPosition.value = { x: Math.min(event.clientX, window.innerWidth - width - 8), y: Math.min(event.clientY, window.innerHeight - height - 8) }; }
function downloadMetadata(track: Track) { emit("downloadMetadata", track); }

function selectPlaylist(id: string) { emit("openPlaylist", id); }
function editPlaylist(id: string) { emit("editPlaylist", id); }
function startCreate() { creating.value = true; newName.value = ""; }
function confirmCreate() { const name = newName.value.trim(); if (name) profile.createPlaylist(name); creating.value = false; newName.value = ""; }
function startRename(id: string, name: string) { editingId.value = id; editingName.value = name; }
function confirmRename() { if (editingId.value) profile.renamePlaylist(editingId.value, editingName.value); editingId.value = null; }
function removePlaylist(id: string) {
  if (pendingDelete.value !== id) {
    pendingDelete.value = id;
    window.setTimeout(() => { if (pendingDelete.value === id) pendingDelete.value = null; }, 3000);
    return;
  }
  profile.deletePlaylist(id);
  pendingDelete.value = null;
}
</script>

<template>
  <div class="mx-auto w-full max-w-6xl px-6 py-6 lg:px-8 lg:py-8">
    <div v-if="player.tracks.length === 0" class="ak-frame grid place-items-center gap-4 border border-dashed border-line bg-surface/50 px-6 py-24 text-center">
      <template v-if="loading"><span class="ak-pulse"></span><p class="text-sm font-semibold uppercase tracking-[0.2em] text-muted">{{ t("library.loading") }}</p></template>
      <template v-else><AudioLines :size="36" class="text-dim" /><p class="text-base font-semibold uppercase tracking-[0.2em]">{{ t("library.emptyTitle") }}</p><p class="max-w-md text-sm text-muted">{{ t("library.emptyDesc") }}</p><button type="button" class="ak-clip-tr h-10 bg-accent px-5 text-[13px] font-bold uppercase tracking-[0.25em] text-accent-fg" @click="emit('settings')">{{ t("library.emptyAction") }}</button></template>
    </div>

    <template v-else>
      <section class="ak-frame relative overflow-hidden border border-line bg-surface">
        <div v-if="heroTrack" class="pointer-events-none absolute inset-0 opacity-25" :style="{ background: `radial-gradient(circle at 88% 10%, ${heroTrack.color}, transparent 58%)` }"></div>
        <div class="relative grid gap-8 p-6 lg:grid-cols-[minmax(0,1fr)_auto] lg:items-center lg:p-8">
          <div class="min-w-0">
            <h1 class="text-4xl font-black leading-[1.05] tracking-tight sm:text-5xl">{{ greetingText }}</h1>
            <p class="mt-4 max-w-xl text-sm text-muted">{{ t("home.heroSubtitle") }}</p>
            <div class="mt-6 flex flex-wrap items-center gap-3">
              <button type="button" class="ak-clip-tr flex h-11 items-center gap-2 bg-accent px-5 text-sm font-bold text-accent-fg transition-transform hover:scale-[1.02] active:scale-95" @click="shuffleAll"><Shuffle :size="17" :stroke-width="2.2" />{{ t("home.shuffleAll") }}</button>
              <button v-if="heroTrack" type="button" class="ak-clip-tr flex h-11 items-center gap-2 border border-line px-5 text-sm font-semibold transition-colors hover:border-accent hover:text-accent" @click="play(heroTrack)"><Play :size="16" :stroke-width="2.2" />{{ t("home.continue") }}</button>
            </div>
          </div>
          <div v-if="heroTrack" class="flex items-center gap-4">
            <span class="ak-frame grid h-28 w-28 shrink-0 place-items-center overflow-hidden text-4xl font-black text-white/90" :style="{ backgroundColor: heroTrack.color }">
              <img v-if="heroTrack.cover" :src="heroTrack.cover" alt="" decoding="async" class="h-full w-full object-cover" />
              <template v-else>{{ initial(heroTrack) }}</template>
            </span>
            <div class="hidden max-w-[200px] sm:block">
              <p class="flex items-center gap-2 font-mono text-[10px] uppercase tracking-[0.25em] text-dim"><span class="h-2 w-2" :class="player.isPlaying ? 'bg-accent ak-pulse' : 'bg-dim'"></span>{{ t("home.nowPlaying") }}</p>
              <p class="mt-2 truncate text-sm font-bold">{{ heroTrack.title }}</p>
              <p class="truncate text-xs text-muted">{{ heroTrack.artist }}</p>
              <p class="mt-1 truncate text-xs text-dim">{{ heroTrack.album }}</p>
            </div>
          </div>
        </div>
      </section>

      <section class="mt-6 grid grid-cols-2 gap-4 lg:grid-cols-4">
        <button v-for="stat in stats" :key="stat.key" type="button" :disabled="!stat.view" class="ak-frame flex items-center gap-4 border border-line bg-surface p-4 text-left transition-colors" :class="stat.view ? 'hover:border-accent' : 'cursor-default'" @click="stat.view && emit('navigate', stat.view)">
          <span class="grid h-10 w-10 shrink-0 place-items-center bg-accent/10 text-accent"><component :is="stat.icon" :size="18" :stroke-width="2" /></span>
          <div class="min-w-0">
            <p class="font-mono text-2xl font-black tabular-nums leading-none">{{ stat.value }}</p>
            <p class="mt-1.5 truncate text-[11px] uppercase tracking-[0.2em] text-dim">{{ t(stat.key) }}</p>
          </div>
        </button>
      </section>

      <section v-if="recommended.length" class="mt-10">
        <div class="mb-4 flex items-end justify-between">
          <h2 class="flex items-center gap-3 text-sm font-bold uppercase tracking-[0.25em]"><span class="h-3 w-1 bg-accent"></span>{{ t("home.recommended") }}</h2>
          <button type="button" class="flex items-center gap-2 text-[12px] font-semibold uppercase tracking-[0.2em] text-dim transition-colors hover:text-fg" @click="refreshRecommended"><RefreshCw :size="14" />{{ t("home.refresh") }}</button>
        </div>
        <div class="grid grid-cols-2 gap-4 sm:grid-cols-3 xl:grid-cols-5"><TrackCard v-for="track in recommended" :key="`rec-${track.id}`" :track="track" :active="track.id === player.currentTrack?.id" :playing="player.isPlaying" @play="play" @menu="openContextMenu" /></div>
      </section>

      <section v-if="recentTracks.length" class="mt-10">
        <div class="mb-4 flex items-end justify-between"><h2 class="flex items-center gap-3 text-sm font-bold uppercase tracking-[0.25em]"><span class="h-3 w-1 bg-accent"></span>{{ t("home.recent") }}</h2></div>
        <div class="grid grid-cols-2 gap-4 sm:grid-cols-3 xl:grid-cols-5"><TrackCard v-for="track in recentTracks" :key="`recent-${track.id}`" :track="track" :active="track.id === player.currentTrack?.id" :playing="player.isPlaying" @play="play" @menu="openContextMenu" /></div>
      </section>

      <section v-if="topTracks.length" class="mt-10">
        <div class="mb-4 flex items-end justify-between"><h2 class="flex items-center gap-3 text-sm font-bold uppercase tracking-[0.25em]"><span class="h-3 w-1 bg-accent"></span>{{ t("home.mostPlayed") }}</h2></div>
        <div class="grid grid-cols-2 gap-4 sm:grid-cols-3 xl:grid-cols-5"><TrackCard v-for="track in topTracks" :key="`top-${track.id}`" :track="track" :active="track.id === player.currentTrack?.id" :playing="player.isPlaying" @play="play" @menu="openContextMenu" /></div>
      </section>

      <section v-if="favoriteTracks.length" class="mt-10">
        <div class="mb-4 flex items-end justify-between">
          <h2 class="flex items-center gap-3 text-sm font-bold uppercase tracking-[0.25em]"><span class="h-3 w-1 bg-accent"></span>{{ t("home.favorites") }}</h2>
          <button type="button" class="text-[12px] font-semibold uppercase tracking-[0.2em] text-dim transition-colors hover:text-fg" @click="emit('navigate', 'favorites')">{{ t("home.viewAll") }}</button>
        </div>
        <div class="grid grid-cols-2 gap-4 sm:grid-cols-3 xl:grid-cols-5"><TrackCard v-for="track in favoriteTracks.slice(0, RECOMMEND_LIMIT)" :key="`fav-${track.id}`" :track="track" :active="track.id === player.currentTrack?.id" :playing="player.isPlaying" @play="play" @menu="openContextMenu" /></div>
      </section>

      <section class="mt-10">
        <div class="mb-4 flex items-end justify-between">
          <h2 class="flex items-center gap-3 text-sm font-bold uppercase tracking-[0.25em]"><span class="h-3 w-1 bg-accent"></span>{{ t("home.playlists") }}</h2>
          <form v-if="creating" class="flex items-center gap-2" @submit.prevent="confirmCreate">
            <input v-model="newName" autofocus :placeholder="t('library.playlists.namePlaceholder')" class="h-8 border border-line bg-bg px-2 text-[13px] outline-none focus:border-accent" />
            <button type="submit" class="ak-clip-tr h-8 bg-accent px-3 text-[12px] font-bold text-accent-fg">{{ t("library.playlists.create") }}</button>
            <button type="button" class="grid h-8 w-8 place-items-center text-dim hover:text-fg" @click="creating = false"><X :size="15" /></button>
          </form>
          <button v-else type="button" class="flex items-center gap-2 text-[12px] font-semibold uppercase tracking-[0.2em] text-dim transition-colors hover:text-fg" @click="startCreate"><Plus :size="14" />{{ t("library.playlists.new") }}</button>
        </div>
        <p v-if="profile.playlists.length === 0 && !creating" class="text-sm text-muted">{{ t("library.playlists.empty") }}</p>
        <div v-else class="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-3">
          <article v-for="playlist in profile.playlists" :key="playlist.id" class="group relative flex cursor-pointer items-center gap-3 border border-line bg-surface p-4 transition-all hover:-translate-y-0.5 hover:border-accent" @click="selectPlaylist(playlist.id)">
            <PlaylistCover :playlist="playlist" class="h-11 w-11" :icon-size="20" />
            <div class="min-w-0 flex-1">
              <form v-if="editingId === playlist.id" @submit.prevent="confirmRename" @click.stop>
                <input v-model="editingName" autofocus class="h-8 w-full border border-line bg-bg px-2 text-sm outline-none focus:border-accent" />
              </form>
              <template v-else>
                <p class="truncate text-sm font-semibold">{{ playlist.name }}</p>
                <p class="font-mono text-[11px] uppercase tracking-[0.15em] text-dim">{{ t("library.playlists.count", { count: playlist.tracks.length }) }}</p>
              </template>
            </div>
            <div v-if="editingId === playlist.id" class="flex items-center gap-1" @click.stop>
              <button type="button" class="ak-clip-tr h-8 bg-accent px-2 text-[11px] font-bold text-accent-fg" @click="confirmRename">{{ t("library.playlists.save") }}</button>
            </div>
            <div v-else class="flex items-center gap-1 opacity-0 transition-opacity group-hover:opacity-100">
              <button type="button" class="grid h-8 w-8 place-items-center text-dim hover:text-fg" :title="t('playlistEditor.editTitle')" @click.stop="editPlaylist(playlist.id)"><ImagePlus :size="16" /></button>
              <button type="button" class="grid h-8 w-8 place-items-center text-dim hover:text-fg" :title="t('library.playlists.rename')" @click.stop="startRename(playlist.id, playlist.name)"><Ellipsis :size="16" /></button>
              <button type="button" class="grid h-8 w-8 place-items-center transition-colors" :class="pendingDelete === playlist.id ? 'bg-red-500 text-white' : 'text-dim hover:text-red-500'" :title="pendingDelete === playlist.id ? t('library.playlists.confirmDelete') : t('library.playlists.delete')" @click.stop="removePlaylist(playlist.id)"><Trash2 :size="15" /></button>
            </div>
          </article>
        </div>
      </section>
    </template>

    <TrackContextMenu v-if="contextTrack" :track="contextTrack" :x="contextPosition.x" :y="contextPosition.y" :downloading="downloadingTrackId === contextTrack.id" @close="contextTrack = null" @download-metadata="downloadMetadata" />
  </div>
</template>
