<script setup lang="ts">
import { ChevronRight, Download, Heart, HeartOff, ListPlus, Pencil } from "@lucide/vue";
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import type { Track } from "../stores/player";
import { useProfileStore } from "../stores/profile";

const props = defineProps<{ track: Track; x: number; y: number; downloading: boolean }>();
const emit = defineEmits<{ close: []; downloadMetadata: [track: Track] }>();
const { t } = useI18n();
const profile = useProfileStore();
const visible = ref(true);
const showPlaylists = ref(false);
const favorite = computed(() => profile.isFavorite(props.track.path));

function close() { visible.value = false; }
function closeOnEscape(event: KeyboardEvent) { if (event.key === "Escape") close(); }
function downloadMetadata() { emit("downloadMetadata", props.track); close(); }
function toggleFavorite() { profile.toggleFavorite(props.track.path); close(); }
function addToPlaylist(id: string) { profile.addToPlaylist(id, props.track.path); close(); }
onMounted(() => window.addEventListener("keydown", closeOnEscape));
onBeforeUnmount(() => window.removeEventListener("keydown", closeOnEscape));
</script>

<template>
  <Teleport to="body"><Transition name="track-menu-backdrop"><div v-if="visible" class="fixed inset-0 z-[60]" @pointerdown="close"></div></Transition><Transition name="track-menu-close" @after-leave="emit('close')"><div v-if="visible" class="track-menu fixed z-[61] w-56" :style="{ left: `${x}px`, top: `${y}px` }" role="menu"><span class="track-menu__backdrop track-menu__backdrop--gray" aria-hidden="true"></span><span class="track-menu__backdrop track-menu__backdrop--white" aria-hidden="true"></span>        <div class="track-menu__content"><div class="track-menu__header"><p class="truncate text-xs font-semibold tracking-wide">{{ track.title }}</p><p class="truncate text-[11px] text-black/60">{{ track.artist }}</p></div><button type="button" role="menuitem" class="track-menu__item" @click="toggleFavorite"><HeartOff v-if="favorite" :size="16" /><Heart v-else :size="16" />{{ favorite ? t("library.menu.unfavorite") : t("library.menu.favorite") }}</button><button type="button" role="menuitem" class="track-menu__item" @click="showPlaylists = !showPlaylists"><ListPlus :size="16" /><span class="flex-1">{{ t("library.menu.addToPlaylist") }}</span><ChevronRight :size="14" class="transition-transform" :class="showPlaylists ? 'rotate-90' : ''" /></button><div v-if="showPlaylists" class="track-menu__submenu"><p v-if="profile.playlists.length === 0" class="px-3 py-2 text-[11px] text-black/50">{{ t("library.menu.noPlaylists") }}</p><button v-for="playlist in profile.playlists" :key="playlist.id" type="button" class="track-menu__subitem" @click="addToPlaylist(playlist.id)"><span class="truncate">{{ playlist.name }}</span><span class="ml-auto font-mono text-[10px] text-black/40">{{ playlist.tracks.length }}</span></button></div><button type="button" role="menuitem" class="track-menu__item" @click="close"><Pencil :size="16" />{{ t("library.menu.edit") }}</button><div class="track-menu__divider"></div><button type="button" role="menuitem" class="track-menu__item track-menu__item--last" :disabled="downloading || !track.path" @click="downloadMetadata"><Download :size="16" /><span>{{ downloading ? t("library.menu.downloadingMetadata") : t("library.menu.downloadMetadata") }}</span></button></div></div></Transition></Teleport>
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
