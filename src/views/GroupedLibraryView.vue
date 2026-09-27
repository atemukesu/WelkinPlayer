<script setup lang="ts">
import { computed, ref } from "vue";
import { Disc3, Mic2, Search } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { usePlayerStore } from "../stores/player";
import type { Track } from "../stores/player";
import { groupTracks, isUnknownGroup, type GroupKind } from "../lib/grouping";
import { sortGroups, useGroupSort } from "../lib/sort";
import GroupSortMenu from "../components/GroupSortMenu.vue";

const props = withDefaults(defineProps<{ kind: GroupKind; loading?: boolean }>(), { loading: false });
const emit = defineEmits<{ open: [key: string] }>();
const { t } = useI18n();
const player = usePlayerStore();
const query = ref("");
// Each grid page keeps its own persistent choice; the component remounts per kind.
const { key: sortKey, dir: sortDir } = useGroupSort(props.kind);

const isArtists = computed(() => props.kind === "artists");
const allGroups = computed(() => groupTracks(player.tracks, props.kind));
const total = computed(() => allGroups.value.length);

interface GroupCard {
  key: string;
  name: string;
  count: number;
  coverTrack?: Track;
  secondary: string;
}

const cards = computed<GroupCard[]>(() => {
  const value = query.value.trim().toLocaleLowerCase();
  const list = allGroups.value.map((group) => {
    const name = isUnknownGroup(group.key)
      ? t(isArtists.value ? "library.collections.unknownArtist" : "library.collections.unknownAlbum")
      : group.key;
    const coverTrack = group.tracks.find((track) => track.cover) ?? group.tracks[0];
    const count = group.tracks.length;
    const label = t("library.collections.trackCount", { count });
    const secondary = !isArtists.value && coverTrack?.artist ? `${coverTrack.artist} · ${label}` : label;
    return { key: group.key, name, count, coverTrack, secondary };
  });
  const filtered = value ? list.filter((card) => card.name.toLocaleLowerCase().includes(value)) : list;
  return sortGroups(filtered, sortKey.value, sortDir.value);
});
</script>

<template>
  <div class="h-full w-full overflow-y-auto">
    <div class="mx-auto flex w-full max-w-6xl flex-col px-4 py-5 sm:px-6 sm:py-6 lg:px-8 lg:py-8">
      <header class="shrink-0 border-b border-line pb-6">
        <p class="flex items-center gap-2 text-[13px] font-semibold uppercase tracking-[0.35em] text-accent"><span class="h-2 w-2 bg-accent"></span>{{ t(isArtists ? "library.collections.artistsEyebrow" : "library.collections.albumsEyebrow") }}</p>
        <h1 class="mt-4 truncate text-4xl font-black leading-none tracking-tight sm:text-5xl">{{ t(isArtists ? "library.collections.artistsTitle" : "library.collections.albumsTitle") }}</h1>
        <p class="mt-3 text-sm text-muted">{{ t(isArtists ? "library.collections.artistsSubtitle" : "library.collections.albumsSubtitle") }}</p>
        <div class="mt-5 flex flex-wrap items-center gap-3">
          <label class="flex h-10 min-w-0 flex-1 items-center gap-2 border border-line bg-surface px-3 sm:w-64 sm:flex-none"><Search :size="16" class="text-dim" /><input v-model="query" class="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-dim" :placeholder="t(isArtists ? 'library.collections.searchArtist' : 'library.collections.searchAlbum')" /></label>
          <GroupSortMenu :context="kind" />
          <span class="font-mono text-sm uppercase tracking-[0.2em] text-dim">{{ t(isArtists ? "library.collections.artistCount" : "library.collections.albumCount", { count: total }) }}</span>
        </div>
      </header>

      <div v-if="player.tracks.length === 0" class="ak-frame mt-8 grid shrink-0 place-items-center gap-4 border border-dashed border-line bg-surface/50 px-6 py-20 text-center">
        <template v-if="loading"><span class="ak-pulse"></span><p class="text-sm font-semibold uppercase tracking-[0.2em] text-muted">{{ t("library.loading") }}</p></template>
        <template v-else><component :is="isArtists ? Mic2 : Disc3" :size="36" class="text-dim" /><p class="max-w-md text-sm text-muted">{{ t("library.collections.empty") }}</p></template>
      </div>
      <p v-else-if="cards.length === 0" class="py-10 text-center text-sm text-muted">{{ t("library.collections.empty") }}</p>

      <section v-else class="grid grid-cols-2 gap-4 pt-8 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
        <button v-for="card in cards" :key="card.key" type="button" class="group ak-frame flex flex-col border border-line bg-surface p-3 text-left transition-all hover:-translate-y-0.5 hover:border-accent" @click="emit('open', card.key)">
          <span class="relative aspect-square w-full overflow-hidden" :style="{ backgroundColor: card.coverTrack?.color ?? 'var(--accent)' }">
            <img v-if="card.coverTrack?.cover" :src="card.coverTrack.cover" alt="" loading="lazy" decoding="async" class="absolute inset-0 h-full w-full object-cover" />
            <span v-else class="absolute inset-0 grid place-items-center text-5xl font-black text-white/90">{{ card.name.charAt(0) }}</span>
          </span>
          <p class="mt-3 truncate text-sm font-semibold tracking-wide">{{ card.name }}</p>
          <p class="truncate text-xs text-muted">{{ card.secondary }}</p>
        </button>
      </section>
    </div>
  </div>
</template>
