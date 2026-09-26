<script setup lang="ts">
import { computed, ref } from "vue";
import { Heart, ListChecks, Play } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { usePlayerStore } from "../stores/player";
import type { Track } from "../stores/player";
import { tracksForPaths, useProfileStore } from "../stores/profile";
import TrackList from "../components/TrackList.vue";
import TrackContextMenu from "../components/TrackContextMenu.vue";
import ViewModeToggle from "../components/ViewModeToggle.vue";

defineProps<{ loading: boolean; enriching: boolean; enrichDone: number; enrichTotal: number; downloadingTrackId?: number | null }>();
const emit = defineEmits<{ details: [track: Track]; downloadMetadata: [track: Track]; editLyrics: [track: Track] }>();
const { t } = useI18n();
const player = usePlayerStore();
const profile = useProfileStore();
const contextTrack = ref<Track | null>(null);
const contextPosition = ref({ x: 0, y: 0 });
const selectMode = ref(false);
const selected = ref<Set<string>>(new Set());
const tracks = computed(() => tracksForPaths(profile.favorites, player.tracks));

function toggleSelectMode() {
  selectMode.value = !selectMode.value;
  if (!selectMode.value) selected.value = new Set();
}
function play(track: Track) { player.playInQueue(tracks.value, track); }
function playFirst() {
  const pool = tracks.value.filter((track) => track.path);
  if (pool.length === 0) return;
  const first = player.shuffle ? pool[Math.floor(Math.random() * pool.length)] : pool[0];
  player.playInQueue(tracks.value, first);
}
function openContextMenu(event: MouseEvent, track: Track) { contextTrack.value = track; contextPosition.value = { x: event.clientX, y: event.clientY }; }
function downloadMetadata(track: Track) { emit("downloadMetadata", track); }
</script>

<template>
  <div class="mx-auto flex h-full w-full max-w-6xl flex-col px-6 py-6 lg:px-8 lg:py-8">
    <header class="shrink-0 border-b border-line pb-6">
      <div class="flex flex-wrap items-center gap-6">
        <span class="ak-frame grid h-32 w-32 shrink-0 place-items-center border border-line bg-accent/10 text-accent"><Heart :size="46" :stroke-width="2" /></span>
        <div class="min-w-0">
          <h1 class="truncate text-3xl font-black leading-tight tracking-tight sm:text-4xl">{{ t("nav.favorites") }}</h1>
          <p class="mt-2 font-mono text-sm uppercase tracking-[0.2em] text-dim">{{ t("library.tracks", { count: tracks.length }) }}</p>
          <div class="mt-4 flex items-center gap-2">
            <button type="button" class="ak-clip-tr flex h-10 items-center gap-2 bg-accent px-4 text-[13px] font-bold text-accent-fg transition-transform hover:scale-[1.02] active:scale-95 disabled:opacity-50" :disabled="!tracks.length" @click="playFirst"><Play :size="15" :stroke-width="2.2" />{{ t("library.play") }}</button>
          </div>
        </div>
        <div class="ml-auto flex items-center gap-3">
          <ViewModeToggle />
          <button type="button" class="grid h-10 w-10 shrink-0 place-items-center border transition-colors" :class="selectMode ? 'border-accent bg-accent text-accent-fg' : 'border-line text-dim hover:border-accent hover:text-accent'" :title="t('library.selectMode')" @click="toggleSelectMode"><ListChecks :size="16" :stroke-width="2" /></button>
        </div>
      </div>
    </header>

    <div class="mt-8 min-h-0 flex-1">
      <TrackList v-model:selected="selected" :tracks="tracks" empty-key="favorites.empty" :selectable="selectMode" @play="play" @menu="openContextMenu" @details="emit('details', $event)" />
    </div>

    <TrackContextMenu v-if="contextTrack" :track="contextTrack" :x="contextPosition.x" :y="contextPosition.y" :selected-paths="[...selected]" :downloading="downloadingTrackId === contextTrack.id" @close="contextTrack = null" @download-metadata="downloadMetadata" @details="emit('details', $event)" @edit-lyrics="emit('editLyrics', $event)" />
  </div>
</template>
