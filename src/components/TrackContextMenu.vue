<script setup lang="ts">
import { ArrowDown, ArrowUp, ChevronRight, Download, Heart, HeartOff, Info, ListPlus, ListX, Pencil } from "@lucide/vue";
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import type { Track } from "../stores/player";
import { useProfileStore } from "../stores/profile";

const props = withDefaults(defineProps<{ track: Track; x: number; y: number; downloading: boolean; selectedPaths?: string[]; playlistId?: string | null }>(), { selectedPaths: () => [], playlistId: null });
const emit = defineEmits<{ close: []; downloadMetadata: [track: Track]; details: [track: Track] }>();
const { t } = useI18n();
const profile = useProfileStore();
const visible = ref(true);
const showPlaylists = ref(false);

const paths = computed(() => props.selectedPaths.length > 0 ? props.selectedPaths : (props.track.path ? [props.track.path] : []));
const isSelection = computed(() => props.selectedPaths.length > 0);
const allFavorite = computed(() => paths.value.length > 0 && paths.value.every((path) => profile.isFavorite(path)));
const favorite = computed(() => !isSelection.value && profile.isFavorite(props.track.path));

const playlistPos = computed(() => {
  if (!props.playlistId || !props.track.path || isSelection.value) return null;
  return profile.getTrackIndexInPlaylist(props.playlistId, props.track.path);
});
const canMoveUp = computed(() => playlistPos.value !== null && playlistPos.value.index > 0);
const canMoveDown = computed(() => playlistPos.value !== null && playlistPos.value.index < playlistPos.value.total - 1);

function close() { visible.value = false; }
function closeOnEscape(event: KeyboardEvent) { if (event.key === "Escape") close(); }
function downloadMetadata() { emit("downloadMetadata", props.track); close(); }
function viewDetails() { emit("details", props.track); close(); }
function toggleFavorite() {
  const desired = !allFavorite.value;
  for (const path of paths.value) {
    if (profile.isFavorite(path) !== desired) profile.toggleFavorite(path);
  }
  close();
}
function addToPlaylist(id: string) {
  for (const path of paths.value) profile.addToPlaylist(id, path);
  close();
}
function removeFromPlaylist() {
  if (!props.playlistId) return;
  for (const path of paths.value) profile.removeFromPlaylist(props.playlistId, path);
  close();
}
function moveTrack(direction: "up" | "down") {
  if (!props.playlistId || !props.track.path) return;
  profile.moveTrackInPlaylist(props.playlistId, props.track.path, direction);
  close();
}
onMounted(() => window.addEventListener("keydown", closeOnEscape));
onBeforeUnmount(() => window.removeEventListener("keydown", closeOnEscape));
</script>

<template>
  <Teleport to="body"><Transition name="track-menu-backdrop"><div v-if="visible" class="fixed inset-0 z-[60]" @pointerdown="close"></div></Transition><Transition name="track-menu-close" @after-leave="emit('close')"><div v-if="visible" class="track-menu fixed z-[61] w-56" :style="{ left: `${x}px`, top: `${y}px` }" role="menu"><span class="track-menu__backdrop track-menu__backdrop--gray" aria-hidden="true"></span><span class="track-menu__backdrop track-menu__backdrop--white" aria-hidden="true"></span>        <div class="track-menu__content">
          <div class="track-menu__header">
            <template v-if="isSelection"><p class="truncate text-xs font-semibold tracking-wide">{{ t("library.menu.selectedCount", { count: paths.length }) }}</p></template>
            <template v-else><p class="truncate text-xs font-semibold tracking-wide">{{ track.title }}</p><p class="truncate text-[11px] text-black/60">{{ track.artist }}</p></template>
          </div>
          <button v-if="paths.length" type="button" role="menuitem" class="track-menu__item" @click="toggleFavorite"><HeartOff v-if="isSelection ? allFavorite : favorite" :size="16" /><Heart v-else :size="16" />{{ (isSelection ? allFavorite : favorite) ? t("library.menu.unfavorite") : t("library.menu.favorite") }}</button>
          <button type="button" role="menuitem" class="track-menu__item" @click="showPlaylists = !showPlaylists"><ListPlus :size="16" /><span class="flex-1">{{ t("library.menu.addToPlaylist") }}</span><ChevronRight :size="14" class="transition-transform" :class="showPlaylists ? 'rotate-90' : ''" /></button>
          <div v-if="showPlaylists" class="track-menu__submenu"><p v-if="profile.playlists.length === 0" class="px-3 py-2 text-[11px] text-black/50">{{ t("library.menu.noPlaylists") }}</p><button v-for="playlist in profile.playlists" :key="playlist.id" type="button" class="track-menu__subitem" @click="addToPlaylist(playlist.id)"><span class="truncate">{{ playlist.name }}</span><span class="ml-auto font-mono text-[10px] text-black/40">{{ playlist.tracks.length }}</span></button></div>
          <button v-if="playlistId && !isSelection && canMoveUp" type="button" role="menuitem" class="track-menu__item" @click="moveTrack('up')"><ArrowUp :size="16" />{{ t("library.menu.moveUp") }}</button>
          <button v-if="playlistId && !isSelection && canMoveDown" type="button" role="menuitem" class="track-menu__item" @click="moveTrack('down')"><ArrowDown :size="16" />{{ t("library.menu.moveDown") }}</button>
          <button v-if="playlistId" type="button" role="menuitem" class="track-menu__item" @click="removeFromPlaylist"><ListX :size="16" />{{ t("library.menu.removeFromPlaylist") }}</button>
          <button v-if="!isSelection" type="button" role="menuitem" class="track-menu__item" @click="viewDetails"><Info :size="16" />{{ t("library.menu.properties") }}</button>
          <button v-if="!isSelection" type="button" role="menuitem" class="track-menu__item" @click="close"><Pencil :size="16" />{{ t("library.menu.edit") }}</button>
          <div class="track-menu__divider"></div>
          <button type="button" role="menuitem" class="track-menu__item track-menu__item--last" :disabled="downloading || !track.path" @click="downloadMetadata"><Download :size="16" /><span>{{ downloading ? t("library.menu.downloadingMetadata") : t("library.menu.downloadMetadata") }}</span></button>
        </div>
      </div></Transition></Teleport>
</template>

<style scoped>
.track-menu { isolation: isolate; }
.track-menu__backdrop { position: absolute; inset: 0; pointer-events: none; clip-path: polygon(0 0, calc(100% - 10px) 0, 100% 10px, 100% 100%, 10px 100%, 0 calc(100% - 10px)); -webkit-mask-image: linear-gradient(#000, #000); mask-image: linear-gradient(#000, #000); -webkit-mask-repeat: no-repeat; mask-repeat: no-repeat; animation: ak-mask-h 320ms cubic-bezier(0.2, 0.8, 0.2, 1) both; }
.track-menu__backdrop--gray { z-index: 0; background: #9ca3af; transform: translate(6px, 6px); animation-delay: 90ms; }
.track-menu__backdrop--white { z-index: 1; background: #fff; }
.track-menu__content { position: relative; z-index: 2; -webkit-mask-image: linear-gradient(#000, #000); mask-image: linear-gradient(#000, #000); -webkit-mask-repeat: no-repeat; mask-repeat: no-repeat; animation: ak-mask-h 320ms cubic-bezier(0.2, 0.8, 0.2, 1) both; }
.track-menu__header { border-bottom: 1px solid #d1d5db; padding: 0.5rem 0.75rem; color: #000; }
.track-menu__item { display: flex; width: 100%; align-items: center; gap: 0.625rem; padding: 0.65rem 0.75rem; color: #000; font-size: 0.75rem; font-weight: 700; text-align: left; text-transform: uppercase; letter-spacing: 0.1em; transition: background-color 180ms cubic-bezier(0.2, 0.8, 0.2, 1); }
.track-menu__item:hover:not(:disabled) { background: #d1d5db; color: #000; }
.track-menu__item:disabled { cursor: progress; opacity: 0.5; }
.track-menu__item--last { clip-path: polygon(0 0, 100% 0, 100% 100%, 10px 100%, 0 calc(100% - 10px)); }
.track-menu__divider { margin: 0.25rem 0; border-top: 1px solid #d1d5db; }
.track-menu__submenu { max-height: 168px; overflow-y: auto; border-top: 1px solid #d1d5db; background: #f3f4f6; }
.track-menu__subitem { display: flex; width: 100%; align-items: center; gap: 0.5rem; padding: 0.5rem 1.1rem; color: #000; font-size: 0.72rem; text-align: left; transition: background-color 160ms; }
.track-menu__subitem:hover { background: #d1d5db; }
.track-menu-close-enter-active, .track-menu-close-leave-active { transition: opacity 180ms cubic-bezier(0.2, 0.8, 0.2, 1), transform 180ms cubic-bezier(0.2, 0.8, 0.2, 1); }
.track-menu-close-enter-from, .track-menu-close-leave-to { opacity: 0; transform: translate(-5px, -5px); }
.track-menu-backdrop-leave-active { transition: opacity 180ms cubic-bezier(0.2, 0.8, 0.2, 1); }
.track-menu-backdrop-leave-to { opacity: 0; }
</style>
