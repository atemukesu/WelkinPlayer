<!--
 Copyright 2026 Atemukesu
 SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { CircleCheck, HardDrive, ListChecks, Play, Search, Trash2 } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { useCacheStore } from "../stores/cache";
import { usePlayerStore } from "../stores/player";
import type { Track } from "../stores/player";
import { formatBytes } from "../lib/format";
import { splitTrackKey, trackKey } from "../lib/sources";
import { sortTracks, titleLetterIndex, useTrackSort } from "../lib/sort";
import TrackList from "../components/TrackList.vue";
import TrackSortMenu from "../components/TrackSortMenu.vue";
import ViewModeToggle from "../components/ViewModeToggle.vue";

const emit = defineEmits<{ openArtist: [artist: string]; openAlbum: [album: string] }>();
const { t } = useI18n();
const cache = useCacheStore();
const player = usePlayerStore();
const scrollEl = ref<HTMLElement | null>(null);
const query = ref("");
const selectMode = ref(false);
const selected = ref<Set<string>>(new Set());

/** Build a display track for a cache entry that is no longer in the library. */
function cachedPlaceholder(key: string, index: number): Track {
  const { sourceId, path } = splitTrackKey(key);
  const name = path.split("/").pop() ?? path;
  return {
    id: -(1_000_000 + index),
    title: name.replace(/\.[^./]+$/, ""),
    artist: "",
    album: "",
    duration: "",
    color: "",
    sourceId,
    path,
  };
}

/**
 * Resolve every completed cache entry to a library track. Keys are matched
 * directly first, then by path (older caches may have been written under a
 * different profile source id). Entries with no library match still appear as
 * placeholders so the count and the list never disagree.
 */
const sourceTracks = computed<Track[]>(() => {
  const byKey = new Map<string, Track>();
  const byPath = new Map<string, Track[]>();
  for (const track of player.tracks) {
    const key = trackKey(track);
    if (key) byKey.set(key, track);
    if (!track.path) continue;
    const list = byPath.get(track.path);
    if (list) list.push(track);
    else byPath.set(track.path, [track]);
  }
  const used = new Set<string>();
  return cache.completedItems.map((item, index) => {
    const direct = byKey.get(item.key);
    if (direct) {
      const key = trackKey(direct);
      if (key) used.add(key);
      return direct;
    }
    const { path } = splitTrackKey(item.key);
    const fallback = (byPath.get(path) ?? []).find((track) => {
      const key = trackKey(track);
      return key ? !used.has(key) : false;
    });
    if (fallback) {
      const key = trackKey(fallback);
      if (key) used.add(key);
      return fallback;
    }
    return cachedPlaceholder(item.key, index);
  });
});

const { key: sortKey, dir: sortDir } = useTrackSort("downloaded");
const orderedTracks = computed(() => sortTracks(sourceTracks.value, sortKey.value, sortDir.value));
const tracks = computed(() => {
  const value = query.value.trim().toLocaleLowerCase();
  return value ? orderedTracks.value.filter((track) => `${track.title} ${track.artist} ${track.album}`.toLocaleLowerCase().includes(value)) : orderedTracks.value;
});
const letterIndex = computed(() => (sortKey.value === "title" && !selectMode.value ? titleLetterIndex(tracks.value) : null));
const selectedCount = computed(() => selected.value.size);
const totalBytes = computed(() => cache.completedBytes);
const summary = computed(() => [
  { key: "downloads.cachedCount", value: tracks.value.length, icon: CircleCheck },
  { key: "downloads.totalSize", value: formatBytes(totalBytes.value), icon: HardDrive },
]);

watch(tracks, (list) => {
  const available = new Set(list.map((track) => trackKey(track)).filter((key): key is string => !!key));
  const next = new Set([...selected.value].filter((key) => available.has(key)));
  if (next.size !== selected.value.size) selected.value = next;
});

function toggleSelectMode() {
  selectMode.value = !selectMode.value;
  if (!selectMode.value) selected.value = new Set();
}
function play(track: Track) { player.playInQueue(orderedTracks.value, track); }
function playFirst() {
  const pool = tracks.value.filter((track) => track.path);
  if (pool.length === 0) return;
  const first = player.shuffle ? pool[Math.floor(Math.random() * pool.length)] : pool[0];
  player.playInQueue(orderedTracks.value, first);
}
async function removeSelected() {
  if (selected.value.size === 0) return;
  const keys = [...selected.value];
  selected.value = new Set();
  await cache.unpinMany(keys);
}
</script>

<template>
  <div ref="scrollEl" class="h-full w-full overflow-y-auto">
    <div class="mx-auto flex w-full max-w-6xl flex-col px-4 py-5 sm:px-6 sm:py-6 lg:px-8 lg:py-8">
      <header class="shrink-0 border-b border-line pb-6">
        <div class="flex flex-col gap-4 sm:flex-row sm:flex-wrap sm:items-stretch sm:gap-6">
          <span class="ak-frame grid h-32 w-32 shrink-0 place-items-center border border-line bg-accent/10 text-accent"><CircleCheck :size="46" :stroke-width="2" /></span>
          <div class="@container flex min-w-0 flex-col sm:flex-1">
            <p class="flex items-center gap-2 text-[13px] font-semibold uppercase tracking-[0.35em] text-accent"><span class="h-2 w-2 bg-accent"></span>{{ t("downloads.eyebrow") }}</p>
            <h1 class="mt-4 truncate text-3xl font-black leading-tight tracking-tight sm:text-4xl">{{ t("downloads.downloadedTitle") }}</h1>
            <p class="mt-2 max-w-xl text-sm text-muted">{{ t("downloads.downloadedSubtitle") }}</p>
            <p class="mt-3 font-mono text-sm uppercase tracking-[0.2em] text-dim">{{ t("library.tracks", { count: tracks.length }) }} · {{ formatBytes(totalBytes) }}</p>

            <div class="mt-auto flex flex-wrap items-center gap-3 pt-4">
              <button type="button" class="ak-clip-tr order-1 flex h-10 shrink-0 items-center gap-2 bg-accent px-4 text-[13px] font-bold text-accent-fg transition-transform hover:scale-[1.02] active:scale-95 disabled:opacity-50" :disabled="!tracks.length" @click="playFirst"><Play :size="15" :stroke-width="2.2" />{{ t("downloads.playAll") }}</button>
              <label class="order-2 flex h-10 min-w-0 flex-1 items-center gap-2 border border-line bg-surface px-3 @2xl:order-3 @2xl:ml-auto @2xl:w-64 @2xl:flex-none"><Search :size="16" class="text-dim" /><input v-model="query" class="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-dim" :placeholder="t('downloads.search')" aria-label="Search downloaded tracks" /></label>
              <div class="order-3 flex w-full items-center gap-3 @2xl:order-4 @2xl:w-auto">
                <TrackSortMenu context="downloaded" />
                <ViewModeToggle />
                <button v-if="selectMode" type="button" class="ak-clip-tr flex h-10 shrink-0 items-center gap-2 border border-red-500/60 px-4 text-[13px] font-semibold text-red-500 transition-colors hover:bg-red-500/10 disabled:opacity-50" :disabled="selectedCount === 0" @click="removeSelected"><Trash2 :size="15" />{{ selectedCount ? t("downloads.removeSelectedCount", { count: selectedCount }) : t("downloads.removeSelected") }}</button>
                <button type="button" class="grid h-10 w-10 shrink-0 place-items-center border transition-colors" :class="selectMode ? 'border-accent bg-accent text-accent-fg' : 'border-line text-dim hover:border-accent hover:text-accent'" :title="t('library.selectMode')" @click="toggleSelectMode"><ListChecks :size="16" :stroke-width="2" /></button>
              </div>
            </div>
          </div>
        </div>
      </header>

      <section class="mt-6 grid grid-cols-1 gap-4 sm:grid-cols-2">
        <div v-for="item in summary" :key="item.key" class="ak-frame flex items-center gap-4 border border-line bg-surface p-4">
          <span class="grid h-10 w-10 shrink-0 place-items-center bg-accent/10 text-accent"><component :is="item.icon" :size="18" :stroke-width="2" /></span>
          <div class="min-w-0">
            <p class="font-mono text-2xl font-black tabular-nums leading-none">{{ item.value }}</p>
            <p class="mt-1.5 truncate text-[11px] uppercase tracking-[0.2em] text-dim">{{ t(item.key) }}</p>
          </div>
        </div>
      </section>

      <div class="mt-8 min-h-0 flex-1">
        <TrackList v-model:selected="selected" :tracks="tracks" :scroller="scrollEl" :letter-index="letterIndex" empty-key="downloads.noCached" :selectable="selectMode" @play="play" @menu="() => {}" @open-artist="emit('openArtist', $event)" @open-album="emit('openAlbum', $event)" />
      </div>
    </div>
  </div>
</template>
