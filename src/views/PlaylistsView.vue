<script setup lang="ts">
import { computed, ref } from "vue";
import { ListMusic, Pencil, Plus, Search } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { useProfileStore } from "../stores/profile";
import PlaylistCover from "../components/PlaylistCover.vue";

const emit = defineEmits<{ openPlaylist: [id: string]; editPlaylist: [id: string]; createPlaylist: [] }>();
const { t } = useI18n();
const profile = useProfileStore();
const query = ref("");

const playlists = computed(() => {
  const value = query.value.trim().toLocaleLowerCase();
  return value ? profile.playlists.filter((item) => item.name.toLocaleLowerCase().includes(value)) : profile.playlists;
});
</script>

<template>
  <div class="h-full w-full overflow-y-auto">
    <div class="mx-auto flex w-full max-w-6xl flex-col px-4 py-5 sm:px-6 sm:py-6 lg:px-8 lg:py-8">
      <header class="shrink-0 border-b border-line pb-6">
        <p class="flex items-center gap-2 text-[13px] font-semibold uppercase tracking-[0.35em] text-accent"><span class="h-2 w-2 bg-accent"></span>{{ t("playlistsPage.eyebrow") }}</p>
        <h1 class="mt-4 truncate text-4xl font-black leading-none tracking-tight sm:text-5xl">{{ t("playlistsPage.title") }}</h1>
        <p class="mt-3 text-sm text-muted">{{ t("playlistsPage.subtitle") }}</p>
        <div class="mt-5 flex flex-wrap items-center gap-3">
          <label class="flex h-10 min-w-0 flex-1 items-center gap-2 border border-line bg-surface px-3 sm:w-64 sm:flex-none"><Search :size="16" class="text-dim" /><input v-model="query" class="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-dim" :placeholder="t('library.search')" /></label>
          <button type="button" class="ak-clip-tr flex h-10 items-center gap-2 bg-accent px-4 text-[13px] font-bold text-accent-fg transition-transform hover:scale-[1.02] active:scale-95" @click="emit('createPlaylist')"><Plus :size="15" :stroke-width="2.2" />{{ t("playlistsPage.create") }}</button>
          <span class="font-mono text-sm uppercase tracking-[0.2em] text-dim">{{ t("playlistsPage.total", { count: playlists.length }) }}</span>
        </div>
      </header>

      <div v-if="profile.playlists.length === 0" class="ak-frame mt-8 grid place-items-center gap-4 border border-dashed border-line bg-surface/50 px-6 py-20 text-center">
        <ListMusic :size="36" class="text-dim" />
        <p class="max-w-md text-sm text-muted">{{ t("playlistsPage.empty") }}</p>
        <button type="button" class="ak-clip-tr h-10 bg-accent px-5 text-[13px] font-bold uppercase tracking-[0.25em] text-accent-fg" @click="emit('createPlaylist')">{{ t("playlistsPage.create") }}</button>
      </div>
      <p v-else-if="playlists.length === 0" class="py-10 text-center text-sm text-muted">{{ t("playlistsPage.empty") }}</p>

      <section v-else class="grid grid-cols-2 gap-4 pt-8 sm:grid-cols-3 lg:grid-cols-4">
        <article v-for="playlist in playlists" :key="playlist.id" class="group ak-frame flex flex-col border border-line bg-surface p-3 transition-all hover:-translate-y-0.5 hover:border-accent">
          <button type="button" class="relative aspect-square w-full overflow-hidden" :title="t('playlistsPage.open')" @click="emit('openPlaylist', playlist.id)">
            <PlaylistCover :playlist="playlist" class="h-full w-full text-4xl" :icon-size="40" />
          </button>
          <div class="mt-3 flex items-start gap-2">
            <button type="button" class="min-w-0 flex-1 text-left" @click="emit('openPlaylist', playlist.id)">
              <p class="truncate text-sm font-semibold">{{ playlist.name }}</p>
              <p class="font-mono text-[11px] uppercase tracking-[0.15em] text-dim">{{ t("playlistsPage.count", { count: playlist.tracks.length }) }}</p>
            </button>
            <div class="flex shrink-0 items-center gap-0.5">
              <button type="button" class="grid h-8 w-8 place-items-center text-dim transition-colors hover:text-fg" :title="t('playlistsPage.edit')" @click="emit('editPlaylist', playlist.id)"><Pencil :size="15" /></button>
            </div>
          </div>
        </article>
      </section>
    </div>
  </div>
</template>
