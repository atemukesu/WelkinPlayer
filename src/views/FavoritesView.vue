<script setup lang="ts">
import { computed, ref } from "vue";
import { Heart } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { usePlayerStore } from "../stores/player";
import type { Track } from "../stores/player";
import { tracksForPaths, useProfileStore } from "../stores/profile";
import TrackList from "../components/TrackList.vue";
import TrackContextMenu from "../components/TrackContextMenu.vue";
import ViewModeToggle from "../components/ViewModeToggle.vue";

defineProps<{ loading: boolean; enriching: boolean; enrichDone: number; enrichTotal: number; downloadingTrackId?: number | null }>();
const emit = defineEmits<{ details: [track: Track]; downloadMetadata: [track: Track] }>();
const { t } = useI18n();
const player = usePlayerStore();
const profile = useProfileStore();
const contextTrack = ref<Track | null>(null);
const contextPosition = ref({ x: 0, y: 0 });
const tracks = computed(() => tracksForPaths(profile.favorites, player.tracks));

function play(track: Track) { player.selectTrack(track); }
function openContextMenu(event: MouseEvent, track: Track) { const width = 224; const height = 240; contextTrack.value = track; contextPosition.value = { x: Math.min(event.clientX, window.innerWidth - width - 8), y: Math.min(event.clientY, window.innerHeight - height - 8) }; }
function downloadMetadata(track: Track) { emit("downloadMetadata", track); }
</script>

<template>
  <div class="mx-auto w-full max-w-6xl px-6 py-6 lg:px-8 lg:py-8">
    <header class="flex items-center gap-4 border-b border-line pb-6">
      <span class="grid h-12 w-12 shrink-0 place-items-center bg-accent text-accent-fg"><Heart :size="22" :stroke-width="2.2" /></span>
      <div class="min-w-0">
        <h1 class="text-4xl font-black leading-none tracking-tight sm:text-5xl">{{ t("nav.favorites") }}</h1>
        <p class="mt-2 font-mono text-sm uppercase tracking-[0.2em] text-dim">{{ t("library.tracks", { count: tracks.length }) }}</p>
      </div>
      <ViewModeToggle class="ml-auto shrink-0" />
    </header>

    <div class="mt-8">
      <TrackList :tracks="tracks" empty-key="favorites.empty" @play="play" @menu="openContextMenu" @details="emit('details', $event)" />
    </div>

    <TrackContextMenu v-if="contextTrack" :track="contextTrack" :x="contextPosition.x" :y="contextPosition.y" :downloading="downloadingTrackId === contextTrack.id" @close="contextTrack = null" @download-metadata="downloadMetadata" />
  </div>
</template>
