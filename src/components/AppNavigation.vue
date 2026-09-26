<script setup lang="ts">
import { ref } from "vue";
import { Activity, ChevronLeft, ChevronRight, Ellipsis, Heart, Library, ListMusic, ListPlus, Music2, Plus, Settings2, X } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { navItems } from "../lib/app";
import type { View } from "../lib/app";
import { useProfileStore } from "../stores/profile";
import PlaylistCover from "./PlaylistCover.vue";

const props = withDefaults(defineProps<{ activeView: View; placement: "header" | "sidebar" | "mobile"; collapsed?: boolean; activePlaylistId?: string | null }>(), { collapsed: false, activePlaylistId: null });
const emit = defineEmits<{ navigate: [view: View]; toggle: []; createPlaylist: []; openPlaylist: [id: string] }>();
const { t } = useI18n();
const profile = useProfileStore();
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
  if (key === "tracks") return props.activeView === "tracks" && !props.activePlaylistId;
  if (key === "favorites") return props.activeView === "favorites";
  if (key === "playlists") return props.activeView === "playlist-new" || (props.activeView === "tracks" && !!props.activePlaylistId);
  return props.activeView === "stats" || props.activeView === "settings";
}
function isActivePlaylist(id: string): boolean { return props.activePlaylistId === id; }
</script>

<template>
  <header v-if="placement === 'header'" class="flex h-14 items-center justify-between border-b border-line bg-surface px-4 md:hidden">
    <span class="flex items-center gap-2 text-sm font-black uppercase tracking-[0.25em]"><span class="h-3 w-3 bg-accent"></span>Welkin</span>
    <span class="font-mono text-[10px] uppercase tracking-[0.2em] text-dim">{{ t("brand.subtitle") }}</span>
  </header>
  <aside v-if="placement === 'sidebar'" class="sidebar-shell hidden min-h-0 shrink-0 flex-col overflow-y-auto overscroll-contain border-r border-line bg-surface md:flex" :class="{ 'is-collapsed': collapsed }">
    <div class="sidebar-brand flex items-center border-b border-line pb-5" :class="collapsed ? 'justify-center' : 'gap-3'"><span class="grid h-9 w-9 shrink-0 place-items-center bg-accent text-accent-fg"><Music2 :size="18" :stroke-width="2.2" /></span><div class="sidebar-brand-copy"><p class="text-sm font-black uppercase tracking-[0.25em] leading-none">Welkin</p><p class="mt-1 whitespace-nowrap font-mono text-[10px] uppercase tracking-[0.2em] text-dim">{{ t("brand.subtitle") }}</p></div></div>
    <nav class="flex flex-col gap-px" :class="collapsed ? 'mt-4' : 'mt-6'"><button v-for="item in navItems" :key="item.id" class="relative flex h-11 items-center border border-transparent text-left text-[13px] font-semibold uppercase tracking-[0.2em] transition-colors" :class="[collapsed ? 'justify-center px-0' : 'gap-2 px-3', activeView === item.id ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg']" :title="collapsed ? t(item.labelKey) : undefined" @click="emit('navigate', item.id)"><component :is="item.icon" :size="16" :stroke-width="1.8" /><span class="sidebar-nav-label truncate">{{ t(item.labelKey) }}</span></button></nav>
    <div class="mt-5 border-t border-line pt-4">
      <button class="relative flex h-11 w-full items-center border border-transparent text-left text-[13px] font-semibold uppercase tracking-[0.2em] transition-colors" :class="[collapsed ? 'justify-center px-0' : 'gap-2 px-3', activeView === 'favorites' ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg']" :title="collapsed ? t('nav.favorites') : undefined" @click="emit('navigate', 'favorites')"><Heart :size="16" :stroke-width="1.8" /><span class="sidebar-nav-label truncate">{{ t("nav.favorites") }}</span></button>
      <div v-if="profile.playlists.length" class="mt-1.5 -mx-1.5 grid max-h-60 gap-px overflow-y-auto px-1.5 pt-1.5">
        <button v-for="playlist in profile.playlists" :key="playlist.id" class="relative flex h-11 w-full min-w-0 items-center border border-transparent text-left text-[13px] font-semibold tracking-[0.1em] transition-colors" :class="[collapsed ? 'justify-center px-0' : 'gap-2 px-3', isActivePlaylist(playlist.id) ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg']" :title="collapsed ? playlist.name : undefined" @click="emit('openPlaylist', playlist.id)"><PlaylistCover :playlist="playlist" class="h-6 w-6" :icon-size="13" /><span class="sidebar-nav-label min-w-0 truncate">{{ playlist.name }}</span></button>
      </div>
      <button class="relative mt-3 flex h-11 w-full items-center border border-transparent text-left text-[13px] font-semibold uppercase tracking-[0.2em] transition-colors" :class="[collapsed ? 'justify-center px-0' : 'gap-2 px-3', activeView === 'playlist-new' ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg']" :title="collapsed ? t('library.playlists.new') : undefined" @click="emit('createPlaylist')"><Plus :size="16" :stroke-width="2" /><span class="sidebar-nav-label truncate">{{ t("library.playlists.new") }}</span></button>
      <div class="my-3 border-t border-line"></div>
      <button class="relative flex h-11 w-full items-center border border-transparent text-left text-[13px] font-semibold uppercase tracking-[0.2em] transition-colors" :class="[collapsed ? 'justify-center px-0' : 'gap-2 px-3', activeView === 'tracks' && !activePlaylistId ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg']" :title="collapsed ? t('nav.tracks') : undefined" @click="emit('navigate', 'tracks')"><ListMusic :size="16" :stroke-width="1.8" /><span class="sidebar-nav-label truncate">{{ t("nav.tracks") }}</span></button>
      <button class="relative mt-1.5 flex h-11 w-full items-center border border-transparent text-left text-[13px] font-semibold uppercase tracking-[0.2em] transition-colors" :class="[collapsed ? 'justify-center px-0' : 'gap-2 px-3', activeView === 'stats' ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg']" :title="collapsed ? t('nav.stats') : undefined" @click="emit('navigate', 'stats')"><Activity :size="16" :stroke-width="1.8" /><span class="sidebar-nav-label truncate">{{ t("nav.stats") }}</span></button>
    </div>
    <div class="mt-auto border-t border-line pt-3">
      <button class="relative flex h-11 w-full items-center border border-transparent text-left text-[13px] font-semibold uppercase tracking-[0.2em] transition-colors" :class="[collapsed ? 'justify-center px-0' : 'gap-2 px-3', activeView === 'settings' ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg']" :title="collapsed ? t('nav.settings') : undefined" @click="emit('navigate', 'settings')"><Settings2 :size="16" :stroke-width="1.8" /><span class="sidebar-nav-label truncate">{{ t("nav.settings") }}</span></button>
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
            <div v-else class="grid gap-px px-3 pb-2">
              <button v-for="playlist in profile.playlists" :key="playlist.id" type="button" class="flex min-w-0 items-center gap-3 px-2 py-2 text-left text-[13px] font-semibold transition-colors" :class="isActivePlaylist(playlist.id) ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg'" @click="openMobilePlaylist(playlist.id)"><PlaylistCover :playlist="playlist" class="h-8 w-8" :icon-size="15" /><span class="min-w-0 flex-1 truncate">{{ playlist.name }}</span><span class="shrink-0 font-mono text-[10px] tabular-nums text-dim">{{ t("library.playlists.count", { count: playlist.tracks.length }) }}</span></button>
            </div>
            <div class="px-3">
              <button type="button" class="flex h-11 w-full items-center gap-3 px-2 text-left text-[13px] font-semibold uppercase tracking-[0.15em] text-muted transition-colors hover:bg-fg/5 hover:text-fg" @click="createMobilePlaylist"><Plus :size="16" :stroke-width="2" /><span>{{ t("library.playlists.new") }}</span></button>
            </div>
          </template>
          <div v-else class="grid gap-px px-3 pb-2">
            <button type="button" class="flex h-11 items-center gap-3 px-2 text-left text-[13px] font-semibold uppercase tracking-[0.15em] transition-colors" :class="activeView === 'stats' ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg'" @click="navigateMobile('stats')"><Activity :size="16" :stroke-width="1.8" /><span>{{ t("nav.stats") }}</span></button>
            <button type="button" class="flex h-11 items-center gap-3 px-2 text-left text-[13px] font-semibold uppercase tracking-[0.15em] transition-colors" :class="activeView === 'playlist-new' ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg'" @click="createMobilePlaylist"><Plus :size="16" :stroke-width="2" /><span>{{ t("library.playlists.new") }}</span></button>
            <button type="button" class="flex h-11 items-center gap-3 px-2 text-left text-[13px] font-semibold uppercase tracking-[0.15em] transition-colors" :class="activeView === 'settings' ? 'ak-select' : 'text-muted hover:bg-fg/5 hover:text-fg'" @click="navigateMobile('settings')"><Settings2 :size="16" :stroke-width="1.8" /><span>{{ t("nav.settings") }}</span></button>
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
