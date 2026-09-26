<script setup lang="ts">
import { Activity, ChevronLeft, ChevronRight, Heart, ListMusic, Music2, Plus, Settings2 } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { navItems, settingsNavItem } from "../lib/app";
import type { View } from "../lib/app";
import { useProfileStore } from "../stores/profile";
import PlaylistCover from "./PlaylistCover.vue";

const props = withDefaults(defineProps<{ activeView: View; placement: "header" | "sidebar" | "mobile"; collapsed?: boolean; activePlaylistId?: string | null }>(), { collapsed: false, activePlaylistId: null });
const emit = defineEmits<{ navigate: [view: View]; toggle: []; createPlaylist: []; openPlaylist: [id: string] }>();
const { t } = useI18n();
const profile = useProfileStore();
/** Mobile bar order: primary nav followed by the settings entry. */
const mobileNavItems = [...navItems, settingsNavItem];
function isActivePlaylist(id: string): boolean { return props.activePlaylistId === id; }
</script>

<template>
  <header v-if="placement === 'header'" class="flex h-14 items-center justify-between border-b border-line bg-surface px-4 md:hidden">
    <span class="flex items-center gap-2 text-sm font-black uppercase tracking-[0.25em]"><span class="h-3 w-3 bg-accent"></span>Welkin</span>
    <span class="font-mono text-[10px] uppercase tracking-[0.2em] text-dim">{{ t("brand.subtitle") }}</span>
  </header>
  <aside v-if="placement === 'sidebar'" class="sidebar-shell hidden shrink-0 flex-col border-r border-line bg-surface md:flex" :class="{ 'is-collapsed': collapsed }">
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
  <nav v-if="placement === 'mobile'" class="flex h-14 shrink-0 border-t border-line bg-surface md:hidden"><button v-for="item in mobileNavItems" :key="item.id" class="flex flex-1 flex-col items-center justify-center gap-1 text-[9px] font-semibold uppercase tracking-[0.15em] transition-colors" :class="activeView === item.id ? 'ak-select' : 'text-muted'" @click="emit('navigate', item.id)"><component :is="item.icon" :size="18" :stroke-width="1.8" />{{ t(item.labelKey) }}</button></nav>
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
</style>
