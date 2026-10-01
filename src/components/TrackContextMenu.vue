<script setup lang="ts">
import { ArrowDown, ArrowUp, ChevronRight, CircleX, Download, Eye, EyeOff, FileText, HardDriveDownload, Heart, HeartOff, Info, ListPlus, ListX, Pencil } from "@lucide/vue";
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import type { Track } from "../stores/player";
import { usePlayerStore } from "../stores/player";
import { useProfileStore } from "../stores/profile";
import { useLyricsStore } from "../stores/lyrics";
import { useCacheStore } from "../stores/cache";
import { trackKey } from "../lib/sources";

const props = withDefaults(defineProps<{ track: Track; x: number; y: number; downloading: boolean; selectedPaths?: string[]; playlistId?: string | null }>(), { selectedPaths: () => [], playlistId: null });
const emit = defineEmits<{ close: []; downloadMetadata: [track: Track]; editLyrics: [track: Track]; editInfo: [track: Track]; showInfo: [track: Track] }>();
const { t } = useI18n();
const profile = useProfileStore();
const player = usePlayerStore();
const lyrics = useLyricsStore();
const cache = useCacheStore();
const visible = ref(true);
const showPlaylists = ref(false);
const menuRef = ref<HTMLElement | null>(null);
const position = ref({ x: props.x, y: props.y });

// Keep the menu fully inside the viewport, measuring its real rendered size.
function clampToViewport() {
  const el = menuRef.value;
  if (!el) {
    position.value = { x: props.x, y: props.y };
    return;
  }
  // Extra margin covers the offset shadow and rounded/angled corners.
  const margin = 8;
  const width = el.offsetWidth;
  const height = el.offsetHeight;
  let x = props.x;
  let y = props.y;
  if (x + width + margin > window.innerWidth) x = window.innerWidth - width - margin;
  if (y + height + margin > window.innerHeight) y = window.innerHeight - height - margin;
  if (x < margin) x = margin;
  if (y < margin) y = margin;
  position.value = { x, y };
}

function scheduleClamp() {
  void nextTick(clampToViewport);
}

function onWindowResize() {
  scheduleClamp();
}

const currentKey = computed(() => trackKey(props.track));
const paths = computed(() => props.selectedPaths.length > 0 ? props.selectedPaths : (currentKey.value ? [currentKey.value] : []));
const isSelection = computed(() => props.selectedPaths.length > 0);
const allFavorite = computed(() => paths.value.length > 0 && paths.value.every((key) => profile.isFavorite(key)));
const favorite = computed(() => !isSelection.value && profile.isFavorite(currentKey.value));
const lyricsDisabled = computed(() => !isSelection.value && !!currentKey.value && profile.isLyricsDisabled(currentKey.value));
const pinned = computed(() => cache.isPinned(currentKey.value));

const playlistPos = computed(() => {
  if (!props.playlistId || !currentKey.value || isSelection.value) return null;
  return profile.getTrackIndexInPlaylist(props.playlistId, currentKey.value);
});
const canMoveUp = computed(() => playlistPos.value !== null && playlistPos.value.index > 0);
const canMoveDown = computed(() => playlistPos.value !== null && playlistPos.value.index < playlistPos.value.total - 1);

function close() { visible.value = false; }
function closeOnEscape(event: KeyboardEvent) { if (event.key === "Escape") close(); }
function downloadMetadata() { emit("downloadMetadata", props.track); close(); }
function editLyrics() { emit("editLyrics", props.track); close(); }
function editInfo() { if (!props.track.path) return; emit("editInfo", props.track); close(); }
function showInfo() { if (!props.track.path) return; emit("showInfo", props.track); close(); }
function toggleLyrics() {
  const key = currentKey.value;
  if (!key) return;
  profile.toggleLyricsDisabled(key);
  // Reflect the change immediately when the toggled track is the one playing.
  const current = player.currentTrack;
  if (current && trackKey(current) === key) void lyrics.loadForTrack(current);
  close();
}
function toggleFavorite() {
  const desired = !allFavorite.value;
  for (const key of paths.value) {
    if (profile.isFavorite(key) !== desired) profile.toggleFavorite(key);
  }
  close();
}
function toggleCache() {
  const key = currentKey.value;
  if (!key) return;
  cache.togglePin(key);
  close();
}
function addToPlaylist(id: string) {
  for (const key of paths.value) profile.addToPlaylist(id, key);
  close();
}
function removeFromPlaylist() {
  if (!props.playlistId) return;
  for (const key of paths.value) profile.removeFromPlaylist(props.playlistId, key);
  close();
}
function moveTrack(direction: "up" | "down") {
  const key = currentKey.value;
  if (!props.playlistId || !key) return;
  profile.moveTrackInPlaylist(props.playlistId, key, direction);
  close();
}
watch(() => [props.x, props.y], scheduleClamp);
watch(showPlaylists, scheduleClamp);
watch(visible, (isVisible) => { if (isVisible) scheduleClamp(); });
onMounted(() => {
  window.addEventListener("keydown", closeOnEscape);
  window.addEventListener("resize", onWindowResize);
  scheduleClamp();
});
onBeforeUnmount(() => {
  window.removeEventListener("keydown", closeOnEscape);
  window.removeEventListener("resize", onWindowResize);
});
</script>

<template>
  <Teleport to="body"><Transition name="track-menu-backdrop"><div v-if="visible" class="fixed inset-0 z-[60]" @pointerdown="close"></div></Transition><Transition name="track-menu-close" @after-leave="emit('close')"><div v-if="visible" class="track-menu fixed z-[61] w-56" ref="menuRef" :style="{ left: `${position.x}px`, top: `${position.y}px` }" role="menu"><span class="track-menu__backdrop track-menu__backdrop--gray" aria-hidden="true"></span><span class="track-menu__backdrop track-menu__backdrop--white" aria-hidden="true"></span>        <div class="track-menu__content">
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
          <button v-if="!isSelection && track.path" type="button" role="menuitem" class="track-menu__item" @click="showInfo"><Info :size="16" />{{ t("library.menu.properties") }}</button>
          <button v-if="!isSelection && track.path" type="button" role="menuitem" class="track-menu__item" @click="editInfo"><Pencil :size="16" />{{ t("library.menu.edit") }}</button>
          <button v-if="!isSelection && track.path" type="button" role="menuitem" class="track-menu__item" @click="editLyrics"><FileText :size="16" />{{ t("library.menu.editLyrics") }}</button>
          <button v-if="!isSelection && track.path" type="button" role="menuitem" class="track-menu__item" @click="toggleLyrics"><EyeOff v-if="!lyricsDisabled" :size="16" /><Eye v-else :size="16" />{{ lyricsDisabled ? t("library.menu.enableLyrics") : t("library.menu.disableLyrics") }}</button>
          <button v-if="!isSelection && track.path" type="button" role="menuitem" class="track-menu__item" @click="toggleCache"><CircleX v-if="pinned" :size="16" /><HardDriveDownload v-else :size="16" />{{ pinned ? t("library.menu.uncacheTrack") : t("library.menu.cacheTrack") }}</button>
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
.track-menu__content { position: relative; z-index: 2; max-height: calc(100vh - 16px); overflow-y: auto; -webkit-mask-image: linear-gradient(#000, #000); mask-image: linear-gradient(#000, #000); -webkit-mask-repeat: no-repeat; mask-repeat: no-repeat; animation: ak-mask-h 320ms cubic-bezier(0.2, 0.8, 0.2, 1) both; }
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
