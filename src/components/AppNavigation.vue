<!--
 Copyright 2026 Atemukesu
 SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { Activity, ChevronLeft, ChevronRight, Heart, HeartHandshake, Library, ListMusic, ListPlus, Menu, Music2, Plus, Settings2, Signal, Wifi, WifiOff, X } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { classificationNavItems, navItems } from "../lib/app";
import type { View } from "../lib/app";
import { useProfileStore } from "../stores/profile";
import { useNetworkStore } from "../stores/network";
import { useLicenseStore } from "../stores/license";
import PlaylistCover from "./PlaylistCover.vue";

const props = withDefaults(defineProps<{ activeView: View; placement: "header" | "sidebar" | "mobile"; collapsed?: boolean; activePlaylistId?: string | null; activeArtist?: string | null; activeAlbum?: string | null; activeSource?: string | null }>(), { collapsed: false, activePlaylistId: null, activeArtist: null, activeAlbum: null, activeSource: null });
const emit = defineEmits<{ navigate: [view: View]; toggle: []; createPlaylist: []; openPlaylist: [id: string] }>();
const { t } = useI18n();
const profile = useProfileStore();
const network = useNetworkStore();
const license = useLicenseStore();
const networkLabel = computed(() => (!network.available ? t("nav.networkOffline") : network.metered ? t("nav.networkCellular") : t("nav.networkWifi")));
/** Edition shown on the badge that links to the sponsor page. */
const editionLabel = computed(() => (license.isPro ? t("sponsor.compare.badgePro") : t("nav.freeEdition")));
const playlistsActive = computed(() => props.activeView === "playlists" || (props.activeView === "tracks" && !!props.activePlaylistId));
/** Mobile bottom bar: four primary destinations. Everything else lives in the left drawer. */
const mobileTabs = [
  { key: "library", labelKey: "nav.library", icon: Library },
  { key: "tracks", labelKey: "nav.tracks", icon: ListMusic },
  { key: "favorites", labelKey: "nav.favorites", icon: Heart },
  { key: "playlists", labelKey: "nav.playlists", icon: ListPlus },
];
const mobileDrawerOpen = ref(false);
function openMobileDrawer() { mobileDrawerOpen.value = true; }
function closeMobileDrawer() { mobileDrawerOpen.value = false; }
function navigateMobile(view: View) { closeMobileDrawer(); emit("navigate", view); }
function openMobilePlaylist(id: string) { closeMobileDrawer(); emit("openPlaylist", id); }
function createMobilePlaylist() { closeMobileDrawer(); emit("createPlaylist"); }
function onMobileTab(key: string) { navigateMobile(key as View); }
function isMobileTabActive(key: string): boolean {
  if (key === "library") return props.activeView === "library";
  if (key === "tracks") return props.activeView === "tracks" && !props.activePlaylistId && !props.activeArtist && !props.activeAlbum && !props.activeSource;
  if (key === "favorites") return props.activeView === "favorites";
  return props.activeView === "playlists" || props.activeView === "playlist-new" || (props.activeView === "tracks" && !!props.activePlaylistId);
}
watch(mobileDrawerOpen, (open, _previous, onCleanup) => {
  if (!open) return;
  const onKeydown = (event: KeyboardEvent) => { if (event.key === "Escape") closeMobileDrawer(); };
  window.addEventListener("keydown", onKeydown);
  onCleanup(() => window.removeEventListener("keydown", onKeydown));
});
function isClassificationActive(id: View): boolean {
  if (id === "artists") return props.activeView === "artists" || !!props.activeArtist;
  if (id === "albums") return props.activeView === "albums" || !!props.activeAlbum;
  if (id === "sources") return props.activeView === "sources" || props.activeView === "source-config" || props.activeView === "source-browser" || !!props.activeSource;
  return false;
}
function isActivePlaylist(id: string): boolean { return props.activePlaylistId === id; }
</script>

<template>
  <header v-if="placement === 'header'" class="flex h-[calc(3.5rem_+_env(safe-area-inset-top))] items-center justify-between border-b border-line bg-surface px-3 pt-[env(safe-area-inset-top)] md:hidden">
    <span class="flex min-w-0 items-center gap-2"><button type="button" class="grid h-9 w-9 shrink-0 place-items-center text-muted transition-colors hover:text-fg" :aria-label="t('nav.menu')" @click="openMobileDrawer"><Menu :size="20" :stroke-width="1.8" /></button><span class="flex items-center gap-2 text-sm font-black uppercase tracking-[0.25em]"><span class="h-3 w-3 shrink-0 bg-accent"></span>Welkin</span></span>
    <span class="flex items-center gap-3"><span class="grid place-items-center text-dim" :title="networkLabel"><WifiOff v-if="!network.available" :size="16" :stroke-width="1.8" /><Wifi v-else-if="!network.metered" :size="16" :stroke-width="1.8" /><Signal v-else :size="16" :stroke-width="1.8" /></span><button type="button" class="bg-accent px-2 py-1 text-xs font-bold uppercase leading-none tracking-[0.2em] text-accent-fg transition-opacity hover:opacity-80" @click="emit('navigate', 'sponsor')">{{ editionLabel }}</button></span>
  </header>
  <aside v-if="placement === 'sidebar'" class="sidebar-shell hidden min-h-0 shrink-0 flex-col overflow-y-auto overscroll-contain border-r border-line bg-surface md:flex" :class="{ 'is-collapsed': collapsed }">
    <div class="sidebar-brand flex items-center border-b border-line pb-5" :class="collapsed ? 'justify-center' : 'gap-3'"><span class="grid h-9 w-9 shrink-0 place-items-center bg-accent text-accent-fg"><Music2 :size="18" :stroke-width="2.2" /></span><div class="sidebar-brand-copy"><p class="text-sm font-black uppercase tracking-[0.25em] leading-none">Welkin</p><button type="button" class="mt-2 w-fit bg-accent px-2 py-1 text-xs font-bold uppercase leading-none tracking-[0.2em] text-accent-fg transition-opacity hover:opacity-80" @click="emit('navigate', 'sponsor')">{{ editionLabel }}</button></div><span class="ml-auto grid shrink-0 self-center place-items-center text-dim" :class="collapsed ? 'hidden' : ''" :title="networkLabel"><WifiOff v-if="!network.available" :size="15" :stroke-width="1.8" /><Wifi v-else-if="!network.metered" :size="15" :stroke-width="1.8" /><Signal v-else :size="15" :stroke-width="1.8" /></span></div>
    <nav class="flex flex-col gap-px" :class="collapsed ? 'mt-4' : 'mt-6'"><button v-for="item in navItems" :key="item.id" class="relative flex h-11 items-center border border-transparent text-left text-[13px] font-semibold uppercase tracking-[0.2em] transition-colors" :class="[collapsed ? 'justify-center px-0' : 'gap-2 px-3', activeView === item.id ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg']" :title="collapsed ? t(item.labelKey) : undefined" @click="emit('navigate', item.id)"><component :is="item.icon" :size="16" :stroke-width="1.8" /><span class="sidebar-nav-label truncate">{{ t(item.labelKey) }}</span></button></nav>
    <div class="mt-5 border-t border-line pt-4">
      <button class="relative flex h-11 w-full items-center border border-transparent text-left text-[13px] font-semibold uppercase tracking-[0.2em] transition-colors" :class="[collapsed ? 'justify-center px-0' : 'gap-2 px-3', activeView === 'favorites' ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg']" :title="collapsed ? t('nav.favorites') : undefined" @click="emit('navigate', 'favorites')"><Heart :size="16" :stroke-width="1.8" /><span class="sidebar-nav-label truncate">{{ t("nav.favorites") }}</span></button>
      <button class="relative mt-1.5 flex h-11 w-full items-center border border-transparent text-left text-[13px] font-semibold uppercase tracking-[0.2em] transition-colors" :class="[collapsed ? 'justify-center px-0' : 'gap-2 px-3', playlistsActive ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg']" :title="collapsed ? t('nav.playlists') : undefined" @click="emit('navigate', 'playlists')"><ListPlus :size="16" :stroke-width="1.8" /><span class="sidebar-nav-label truncate">{{ t("nav.playlists") }}</span></button>
      <div class="my-3 border-t border-line"></div>
      <button class="relative flex h-11 w-full items-center border border-transparent text-left text-[13px] font-semibold uppercase tracking-[0.2em] transition-colors" :class="[collapsed ? 'justify-center px-0' : 'gap-2 px-3', activeView === 'tracks' && !activePlaylistId && !activeArtist && !activeAlbum && !activeSource ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg']" :title="collapsed ? t('nav.tracks') : undefined" @click="emit('navigate', 'tracks')"><ListMusic :size="16" :stroke-width="1.8" /><span class="sidebar-nav-label truncate">{{ t("nav.tracks") }}</span></button>
      <button v-for="item in classificationNavItems" :key="item.id" class="relative mt-1.5 flex h-11 w-full items-center border border-transparent text-left text-[13px] font-semibold uppercase tracking-[0.2em] transition-colors" :class="[collapsed ? 'justify-center px-0' : 'gap-2 px-3', isClassificationActive(item.id) ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg']" :title="collapsed ? t(item.labelKey) : undefined" @click="emit('navigate', item.id)"><component :is="item.icon" :size="16" :stroke-width="1.8" /><span class="sidebar-nav-label truncate">{{ t(item.labelKey) }}</span></button>
      <button class="relative mt-1.5 flex h-11 w-full items-center border border-transparent text-left text-[13px] font-semibold uppercase tracking-[0.2em] transition-colors" :class="[collapsed ? 'justify-center px-0' : 'gap-2 px-3', activeView === 'stats' ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg']" :title="collapsed ? t('nav.stats') : undefined" @click="emit('navigate', 'stats')"><Activity :size="16" :stroke-width="1.8" /><span class="sidebar-nav-label truncate">{{ t("nav.stats") }}</span></button>
    </div>
    <div class="mt-auto border-t border-line pt-3">
      <button class="relative flex h-11 w-full items-center border border-transparent text-left text-[13px] font-semibold uppercase tracking-[0.2em] transition-colors" :class="[collapsed ? 'justify-center px-0' : 'gap-2 px-3', activeView === 'settings' ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg']" :title="collapsed ? t('nav.settings') : undefined" @click="emit('navigate', 'settings')"><Settings2 :size="16" :stroke-width="1.8" /><span class="sidebar-nav-label truncate">{{ t("nav.settings") }}</span></button>
      <button type="button" class="relative mt-1.5 flex h-11 w-full items-center border border-transparent text-left text-[13px] font-semibold uppercase tracking-[0.2em] transition-colors" :class="[collapsed ? 'justify-center px-0' : 'gap-2 px-3', activeView === 'sponsor' ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg']" :title="collapsed ? t('nav.sponsor') : undefined" @click="emit('navigate', 'sponsor')"><HeartHandshake :size="16" :stroke-width="1.8" /><span class="sidebar-nav-label truncate">{{ t("nav.sponsor") }}</span></button>
      <button type="button" class="relative mt-1 flex h-11 w-full items-center border border-transparent text-left text-[13px] font-semibold uppercase tracking-[0.2em] text-muted transition-colors hover:bg-fg/5 hover:text-fg" :class="collapsed ? 'justify-center px-0' : 'gap-2 px-3'" :title="t(collapsed ? 'controls.expand' : 'controls.collapse')" @click="emit('toggle')"><ChevronRight v-if="collapsed" :size="16" :stroke-width="1.8" /><ChevronLeft v-else :size="16" :stroke-width="1.8" /><span class="sidebar-nav-label truncate">{{ t(collapsed ? 'controls.expand' : 'controls.collapse') }}</span></button>
    </div>
  </aside>
  <template v-if="placement === 'mobile'">
    <nav class="flex shrink-0 border-t border-line bg-surface pb-[env(safe-area-inset-bottom)] md:hidden">
      <button v-for="tab in mobileTabs" :key="tab.key" type="button" class="flex h-14 flex-1 flex-col items-center justify-center gap-1 text-[10px] font-semibold leading-none transition-colors" :class="isMobileTabActive(tab.key) ? 'ak-select' : 'text-muted'" @click="onMobileTab(tab.key)"><component :is="tab.icon" :size="18" :stroke-width="1.8" /><span class="whitespace-nowrap">{{ t(tab.labelKey) }}</span></button>
    </nav>
    <Teleport to="body">
      <Transition name="drawer-backdrop"><div v-if="mobileDrawerOpen" class="fixed inset-0 z-[70] bg-black/50 md:hidden" @pointerdown="closeMobileDrawer"></div></Transition>
      <Transition name="drawer">
        <aside v-if="mobileDrawerOpen" class="fixed inset-y-0 left-0 z-[71] flex w-[84vw] max-w-[320px] flex-col overflow-y-auto overscroll-contain border-r border-line bg-surface pt-[env(safe-area-inset-top)] pb-[env(safe-area-inset-bottom)] shadow-2xl md:hidden" role="dialog" aria-modal="true" :aria-label="t('nav.menu')">
          <div class="flex items-center gap-3 border-b border-line px-4 py-4">
            <span class="grid h-9 w-9 shrink-0 place-items-center bg-accent text-accent-fg"><Music2 :size="18" :stroke-width="2.2" /></span>
            <div class="min-w-0 flex-1">
              <p class="text-sm font-black uppercase leading-none tracking-[0.25em]">Welkin</p>
              <button type="button" class="mt-2 w-fit bg-accent px-2 py-1 text-xs font-bold uppercase leading-none tracking-[0.2em] text-accent-fg transition-opacity hover:opacity-80" @click="navigateMobile('sponsor')">{{ editionLabel }}</button>
            </div>
            <button type="button" class="grid h-9 w-9 shrink-0 place-items-center text-dim transition-colors hover:text-fg" :aria-label="t('controls.close')" @click="closeMobileDrawer"><X :size="18" :stroke-width="1.8" /></button>
          </div>
          <div class="flex items-center gap-2 border-b border-line px-4 py-2.5 text-[11px] font-semibold uppercase tracking-[0.2em] text-dim">
            <WifiOff v-if="!network.available" :size="14" :stroke-width="1.8" />
            <Wifi v-else-if="!network.metered" :size="14" :stroke-width="1.8" />
            <Signal v-else :size="14" :stroke-width="1.8" />
            <span>{{ networkLabel }}</span>
          </div>
          <nav class="flex flex-col gap-px border-b border-line px-2 py-3">
            <button v-for="tab in mobileTabs" :key="tab.key" type="button" class="flex h-11 items-center gap-3 px-3 text-left text-[13px] font-semibold uppercase tracking-[0.15em] transition-colors" :class="isMobileTabActive(tab.key) ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg'" @click="onMobileTab(tab.key)"><component :is="tab.icon" :size="16" :stroke-width="1.8" /><span class="truncate">{{ t(tab.labelKey) }}</span></button>
          </nav>
          <nav class="flex flex-col gap-px border-b border-line px-2 py-3">
            <button v-for="item in classificationNavItems" :key="item.id" type="button" class="flex h-11 items-center gap-3 px-3 text-left text-[13px] font-semibold uppercase tracking-[0.15em] transition-colors" :class="isClassificationActive(item.id) ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg'" @click="navigateMobile(item.id)"><component :is="item.icon" :size="16" :stroke-width="1.8" /><span class="truncate">{{ t(item.labelKey) }}</span></button>
            <button type="button" class="flex h-11 items-center gap-3 px-3 text-left text-[13px] font-semibold uppercase tracking-[0.15em] transition-colors" :class="activeView === 'stats' ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg'" @click="navigateMobile('stats')"><Activity :size="16" :stroke-width="1.8" /><span class="truncate">{{ t("nav.stats") }}</span></button>
          </nav>
          <div class="border-b border-line px-2 py-3">
            <div class="flex items-center justify-between px-3 pb-2">
              <span class="text-[11px] font-bold uppercase tracking-[0.25em] text-dim">{{ t("nav.playlists") }}</span>
              <button type="button" class="grid h-7 w-7 place-items-center text-muted transition-colors hover:text-fg" :aria-label="t('library.playlists.new')" @click="createMobilePlaylist"><Plus :size="15" :stroke-width="2" /></button>
            </div>
            <p v-if="profile.playlists.length === 0" class="px-3 py-1 text-xs text-muted">{{ t("library.noPlaylists") }}</p>
            <button v-for="playlist in profile.playlists" :key="playlist.id" type="button" class="flex min-w-0 items-center gap-3 px-3 py-2 text-left text-[13px] font-semibold transition-colors" :class="isActivePlaylist(playlist.id) ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg'" @click="openMobilePlaylist(playlist.id)"><PlaylistCover :playlist="playlist" class="h-8 w-8" :icon-size="15" /><span class="min-w-0 flex-1 truncate">{{ playlist.name }}</span><span class="shrink-0 font-mono text-[10px] tabular-nums text-dim">{{ t("library.playlists.count", { count: playlist.tracks.length }) }}</span></button>
            <button type="button" class="mt-1 flex h-10 w-full items-center gap-3 px-3 text-left text-[12px] font-semibold uppercase tracking-[0.15em] text-muted transition-colors hover:bg-fg/5 hover:text-fg" @click="navigateMobile('playlists')"><ListMusic :size="15" :stroke-width="1.8" /><span>{{ t("nav.allPlaylists") }}</span></button>
          </div>
          <nav class="mt-auto flex flex-col gap-px px-2 py-3">
            <button type="button" class="flex h-11 items-center gap-3 px-3 text-left text-[13px] font-semibold uppercase tracking-[0.15em] transition-colors" :class="activeView === 'settings' ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg'" @click="navigateMobile('settings')"><Settings2 :size="16" :stroke-width="1.8" /><span class="truncate">{{ t("nav.settings") }}</span></button>
            <button type="button" class="flex h-11 items-center gap-3 px-3 text-left text-[13px] font-semibold uppercase tracking-[0.15em] transition-colors" :class="activeView === 'sponsor' ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg'" @click="navigateMobile('sponsor')"><HeartHandshake :size="16" :stroke-width="1.8" /><span class="truncate">{{ t("nav.sponsor") }}</span></button>
          </nav>
        </aside>
      </Transition>
    </Teleport>
  </template>
</template>

<style scoped>
.sidebar-shell {
  width: 248px;
  padding: 1.25rem;
  transition: width 360ms cubic-bezier(0.2, 0.8, 0.2, 1), padding 360ms cubic-bezier(0.2, 0.8, 0.2, 1);
}

.sidebar-shell.is-collapsed {
  width: 72px;
  padding-right: 0.75rem;
  padding-left: 0.75rem;
}

.sidebar-brand-copy,
.sidebar-nav-label {
  overflow: hidden;
  max-width: 180px;
  opacity: 1;
  transform: translateX(0);
  transition: max-width 300ms cubic-bezier(0.2, 0.8, 0.2, 1), opacity 180ms ease, transform 300ms cubic-bezier(0.2, 0.8, 0.2, 1);
}

.sidebar-nav-label {
  max-width: 160px;
}

.sidebar-shell.is-collapsed .sidebar-brand-copy,
.sidebar-shell.is-collapsed .sidebar-nav-label {
  max-width: 0;
  opacity: 0;
  transform: translateX(-8px);
  pointer-events: none;
}

.drawer-enter-active,
.drawer-leave-active {
  transition: transform 300ms cubic-bezier(0.2, 0.8, 0.2, 1);
}

.drawer-enter-from,
.drawer-leave-to {
  transform: translateX(-100%);
}

.drawer-backdrop-enter-active,
.drawer-backdrop-leave-active {
  transition: opacity 220ms ease;
}

.drawer-backdrop-enter-from,
.drawer-backdrop-leave-to {
  opacity: 0;
}
</style>
