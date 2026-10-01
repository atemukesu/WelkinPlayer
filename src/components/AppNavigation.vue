<script setup lang="ts">
import { computed, ref } from "vue";
import { Activity, ChevronLeft, ChevronRight, Ellipsis, Heart, HeartHandshake, Library, ListMusic, ListPlus, Music2, Plus, Settings2, Signal, Wifi, WifiOff, X } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { classificationNavItems, navItems } from "../lib/app";
import type { View } from "../lib/app";
import { useProfileStore } from "../stores/profile";
import { useNetworkStore } from "../stores/network";
import PlaylistCover from "./PlaylistCover.vue";

const props = withDefaults(defineProps<{ activeView: View; placement: "header" | "sidebar" | "mobile"; collapsed?: boolean; activePlaylistId?: string | null; activeArtist?: string | null; activeAlbum?: string | null; activeSource?: string | null }>(), { collapsed: false, activePlaylistId: null, activeArtist: null, activeAlbum: null, activeSource: null });
const emit = defineEmits<{ navigate: [view: View]; toggle: []; createPlaylist: []; openPlaylist: [id: string] }>();
const { t } = useI18n();
const profile = useProfileStore();
const network = useNetworkStore();
const networkLabel = computed(() => (!network.available ? t("nav.networkOffline") : network.metered ? t("nav.networkCellular") : t("nav.networkWifi")));
const playlistsActive = computed(() => props.activeView === "playlists" || (props.activeView === "tracks" && !!props.activePlaylistId));
/** Mobile bottom bar: four primary destinations plus a "more" overflow sheet. */
const mobileTabs = [
  { key: "library", labelKey: "nav.library", icon: Library },
  { key: "tracks", labelKey: "nav.tracks", icon: ListMusic },
  { key: "favorites", labelKey: "nav.favorites", icon: Heart },
  { key: "playlists", labelKey: "nav.playlists", icon: ListPlus },
  { key: "more", labelKey: "nav.more", icon: Ellipsis },
];
type MobileSheet = "playlists" | "more";
const mobileSheet = ref<MobileSheet | null>(null);
function toggleMobileSheet(sheet: MobileSheet) { mobileSheet.value = mobileSheet.value === sheet ? null : sheet; }
function closeMobileSheet() { mobileSheet.value = null; }
function navigateMobile(view: View) { closeMobileSheet(); emit("navigate", view); }
function openMobilePlaylist(id: string) { closeMobileSheet(); emit("openPlaylist", id); }
function createMobilePlaylist() { closeMobileSheet(); emit("createPlaylist"); }
function onMobileTab(key: string) { if (key === "playlists" || key === "more") toggleMobileSheet(key); else navigateMobile(key as View); }
function isMobileTabActive(key: string): boolean {
  if (key === "library") return props.activeView === "library";
  if (key === "tracks") return props.activeView === "tracks" && !props.activePlaylistId && !props.activeArtist && !props.activeAlbum && !props.activeSource;
  if (key === "favorites") return props.activeView === "favorites";
  if (key === "playlists") return props.activeView === "playlists" || props.activeView === "playlist-new" || (props.activeView === "tracks" && !!props.activePlaylistId);
  return props.activeView === "stats" || props.activeView === "settings" || props.activeView === "artists" || props.activeView === "albums" || props.activeView === "sources" || props.activeView === "source-config" || !!props.activeArtist || !!props.activeAlbum || !!props.activeSource;
}
function isClassificationActive(id: View): boolean {
  if (id === "artists") return props.activeView === "artists" || !!props.activeArtist;
  if (id === "albums") return props.activeView === "albums" || !!props.activeAlbum;
  if (id === "sources") return props.activeView === "sources" || props.activeView === "source-config" || !!props.activeSource;
  return false;
}
function isActivePlaylist(id: string): boolean { return props.activePlaylistId === id; }
</script>

<template>
  <header v-if="placement === 'header'" class="flex h-[calc(3.5rem_+_env(safe-area-inset-top))] items-center justify-between border-b border-line bg-surface px-4 pt-[env(safe-area-inset-top)] md:hidden">
    <span class="flex items-center gap-2 text-sm font-black uppercase tracking-[0.25em]"><span class="h-3 w-3 bg-accent"></span>Welkin</span>
    <span class="flex items-center gap-3"><span class="grid place-items-center text-dim" :title="networkLabel"><WifiOff v-if="!network.available" :size="16" :stroke-width="1.8" /><Wifi v-else-if="!network.metered" :size="16" :stroke-width="1.8" /><Signal v-else :size="16" :stroke-width="1.8" /></span><button type="button" class="bg-accent px-2 py-1 text-xs font-bold uppercase leading-none tracking-[0.2em] text-accent-fg transition-opacity hover:opacity-80" @click="emit('navigate', 'sponsor')">{{ t("nav.freeEdition") }}</button></span>
  </header>
  <aside v-if="placement === 'sidebar'" class="sidebar-shell hidden min-h-0 shrink-0 flex-col overflow-y-auto overscroll-contain border-r border-line bg-surface md:flex" :class="{ 'is-collapsed': collapsed }">
    <div class="sidebar-brand flex items-center border-b border-line pb-5" :class="collapsed ? 'justify-center' : 'gap-3'"><span class="grid h-9 w-9 shrink-0 place-items-center bg-accent text-accent-fg"><Music2 :size="18" :stroke-width="2.2" /></span><div class="sidebar-brand-copy"><p class="text-sm font-black uppercase tracking-[0.25em] leading-none">Welkin</p><button type="button" class="mt-2 w-fit bg-accent px-2 py-1 text-xs font-bold uppercase leading-none tracking-[0.2em] text-accent-fg transition-opacity hover:opacity-80" @click="emit('navigate', 'sponsor')">{{ t("nav.freeEdition") }}</button></div></div>
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
      <div class="flex h-9 items-center text-[11px] font-semibold uppercase tracking-[0.2em] text-dim" :class="collapsed ? 'justify-center px-0' : 'gap-2 px-3'" :title="networkLabel"><WifiOff v-if="!network.available" :size="15" :stroke-width="1.8" /><Wifi v-else-if="!network.metered" :size="15" :stroke-width="1.8" /><Signal v-else :size="15" :stroke-width="1.8" /><span class="sidebar-nav-label truncate">{{ networkLabel }}</span></div>
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
      <Transition name="sheet-backdrop"><div v-if="mobileSheet" class="fixed inset-0 z-[70] bg-black/40 md:hidden" @pointerdown="closeMobileSheet"></div></Transition>
      <Transition name="sheet">
        <section v-if="mobileSheet" class="fixed inset-x-0 bottom-0 z-[71] max-h-[70vh] overflow-y-auto overscroll-contain border-t border-line bg-surface pb-[calc(env(safe-area-inset-bottom)_+_0.5rem)] md:hidden" role="dialog" aria-modal="true">
          <div class="flex items-center justify-between px-5 py-4">
            <h2 class="flex items-center gap-3 text-sm font-bold uppercase tracking-[0.25em]"><span class="h-3 w-1 bg-accent"></span>{{ t(mobileSheet === 'playlists' ? 'nav.playlists' : 'nav.more') }}</h2>
            <button type="button" class="text-dim transition-colors hover:text-fg" :aria-label="t('controls.close')" @click="closeMobileSheet"><X :size="16" :stroke-width="1.8" /></button>
          </div>
          <template v-if="mobileSheet === 'playlists'">
            <p v-if="profile.playlists.length === 0" class="px-5 pb-4 text-sm text-muted">{{ t("library.noPlaylists") }}</p>
            <template v-else>
              <div class="grid gap-px px-3 pb-2">
                <button v-for="playlist in profile.playlists" :key="playlist.id" type="button" class="flex min-w-0 items-center gap-3 px-2 py-2 text-left text-[13px] font-semibold transition-colors" :class="isActivePlaylist(playlist.id) ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg'" @click="openMobilePlaylist(playlist.id)"><PlaylistCover :playlist="playlist" class="h-8 w-8" :icon-size="15" /><span class="min-w-0 flex-1 truncate">{{ playlist.name }}</span><span class="shrink-0 font-mono text-[10px] tabular-nums text-dim">{{ t("library.playlists.count", { count: playlist.tracks.length }) }}</span></button>
              </div>
            </template>
            <div class="grid gap-px px-3 pb-2">
              <button type="button" class="flex h-11 w-full items-center gap-3 px-2 text-left text-[13px] font-semibold uppercase tracking-[0.15em] text-muted transition-colors hover:bg-fg/5 hover:text-fg" @click="createMobilePlaylist"><Plus :size="16" :stroke-width="2" /><span>{{ t("library.playlists.new") }}</span></button>
              <button type="button" class="flex h-11 w-full items-center gap-3 px-2 text-left text-[13px] font-semibold uppercase tracking-[0.15em] text-muted transition-colors hover:bg-fg/5 hover:text-fg" @click="navigateMobile('playlists')"><ListMusic :size="16" :stroke-width="1.8" /><span>{{ t("nav.allPlaylists") }}</span></button>
            </div>
          </template>
          <div v-else class="grid gap-px px-3 pb-2">
            <button v-for="item in classificationNavItems" :key="item.id" type="button" class="flex h-11 items-center gap-3 px-2 text-left text-[13px] font-semibold uppercase tracking-[0.15em] transition-colors" :class="isClassificationActive(item.id) ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg'" @click="navigateMobile(item.id)"><component :is="item.icon" :size="16" :stroke-width="1.8" /><span>{{ t(item.labelKey) }}</span></button>
            <button type="button" class="flex h-11 items-center gap-3 px-2 text-left text-[13px] font-semibold uppercase tracking-[0.15em] transition-colors" :class="activeView === 'stats' ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg'" @click="navigateMobile('stats')"><Activity :size="16" :stroke-width="1.8" /><span>{{ t("nav.stats") }}</span></button>
            <button type="button" class="flex h-11 items-center gap-3 px-2 text-left text-[13px] font-semibold uppercase tracking-[0.15em] transition-colors" :class="activeView === 'playlist-new' ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg'" @click="createMobilePlaylist"><Plus :size="16" :stroke-width="2" /><span>{{ t("library.playlists.new") }}</span></button>
            <button type="button" class="flex h-11 items-center gap-3 px-2 text-left text-[13px] font-semibold uppercase tracking-[0.15em] transition-colors" :class="activeView === 'settings' ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg'" @click="navigateMobile('settings')"><Settings2 :size="16" :stroke-width="1.8" /><span>{{ t("nav.settings") }}</span></button>
            <button type="button" class="flex h-11 items-center gap-3 px-2 text-left text-[13px] font-semibold uppercase tracking-[0.15em] transition-colors" :class="activeView === 'sponsor' ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg'" @click="navigateMobile('sponsor')"><HeartHandshake :size="16" :stroke-width="1.8" /><span>{{ t("nav.sponsor") }}</span></button>
          </div>
        </section>
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

.sheet-enter-active,
.sheet-leave-active {
  transition: transform 300ms cubic-bezier(0.2, 0.8, 0.2, 1);
}

.sheet-enter-from,
.sheet-leave-to {
  transform: translateY(100%);
}

.sheet-backdrop-enter-active,
.sheet-backdrop-leave-active {
  transition: opacity 220ms ease;
}

.sheet-backdrop-enter-from,
.sheet-backdrop-leave-to {
  opacity: 0;
}
</style>
