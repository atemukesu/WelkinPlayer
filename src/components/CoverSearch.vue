<!--
  Copyright 2026 Atemukesu
  SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { Search, X } from "@lucide/vue";
import LayeredSelect from "./LayeredSelect.vue";
import { invoke } from "../api";
import { pushToast } from "../lib/toast";

interface SongHit {
  id: string;
  title: string;
  artist: string;
  album: string;
  coverUrl: string;
  albumId?: number;
  source: "qq" | "netease";
}

const props = defineProps<{ open: boolean; initialQuery?: string }>();
const emit = defineEmits<{ close: []; select: [url: string] }>();
const { t } = useI18n();

const source = ref<"qq" | "netease">("qq");
const query = ref("");
const searching = ref(false);
const results = ref<SongHit[]>([]);
const selectedId = ref<string | null>(null);
const fetching = ref(false);
const coverUrls = ref<Record<string, string | null>>({});

const sourceOptions = [
  { value: "qq" as const, label: "QQ Music" },
  { value: "netease" as const, label: "NetEase" },
];

watch(() => props.open, (open) => {
  if (open && props.initialQuery) {
    query.value = props.initialQuery;
    void doSearch();
  }
}, { immediate: false });

async function doSearch() {
  const text = query.value.trim();
  if (!text) return;
  searching.value = true;
  results.value = [];
  coverUrls.value = {};
  selectedId.value = null;
  try {
    if (source.value === "qq") {
      const result = await invoke<{ songs: { songMid: string; title: string; artist: string; album: string; albumMid: string }[] }>("qq_search_songs", { text });
      results.value = (result.songs ?? []).map((s) => ({
        id: s.albumMid,
        title: s.title,
        artist: s.artist,
        album: s.album,
        coverUrl: `https://y.gtimg.cn/music/photo_new/T002R180x180M000${s.albumMid}.jpg?max_age=2592000`,
        source: "qq" as const,
      }));
    } else {
      const result = await invoke<{ songs: { songId: number; albumId: number; title: string; artist: string; album: string; coverUrl: string }[] }>("netease_search_songs", { text });
      results.value = (result.songs ?? []).map((s) => ({
        id: String(s.songId),
        title: s.title,
        artist: s.artist,
        album: s.album,
        coverUrl: s.coverUrl,
        albumId: s.albumId,
        source: "netease" as const,
      }));
    }
    if (results.value.length === 0) {
      pushToast("error", t("trackEditor.searchCoverEmpty"));
    }
  } catch {
    pushToast("error", t("trackEditor.searchCoverEmpty"));
  } finally {
    searching.value = false;
  }
}

async function pickSong(song: SongHit) {
  if (fetching.value) return;
  fetching.value = true;
  selectedId.value = song.id;
  try {
    let dataUrl: string;
    if (song.source === "qq") {
      const bytes = await invoke<number[] | null>("qq_fetch_cover", { albumMid: song.id, size: 300 });
      if (!bytes || bytes.length === 0) {
        pushToast("error", t("trackEditor.searchCoverNoResult"));
        return;
      }
      const base64 = btoa(String.fromCharCode(...bytes));
      dataUrl = `data:image/jpeg;base64,${base64}`;
    } else {
      const bytes = await invoke<number[] | null>("netease_fetch_cover", { albumId: song.albumId });
      if (!bytes || bytes.length === 0) {
        pushToast("error", t("trackEditor.searchCoverNoResult"));
        return;
      }
      const base64 = btoa(String.fromCharCode(...bytes));
      dataUrl = `data:image/jpeg;base64,${base64}`;
    }
    coverUrls.value = { [song.id]: dataUrl };
    emit("select", dataUrl);
  } catch {
    pushToast("error", t("trackEditor.searchCoverNoResult"));
  } finally {
    fetching.value = false;
  }
}

function onClose() {
  query.value = "";
  results.value = [];
  selectedId.value = null;
  coverUrls.value = {};
  emit("close");
}
</script>

<template>
  <div v-if="open" class="fixed inset-0 z-[70] grid place-items-center bg-black/70 p-4 overflow-hidden" @pointerdown.self="onClose">
    <div class="ak-frame w-full max-w-lg max-h-[85vh] flex flex-col border border-line bg-surface p-5 overflow-hidden">
      <div class="flex items-center justify-between">
        <h3 class="text-sm font-bold uppercase tracking-[0.2em]">{{ t("trackEditor.searchCover") }}</h3>
        <button type="button" class="grid h-8 w-8 place-items-center text-dim hover:text-fg" @click="onClose"><X :size="16" /></button>
      </div>
      <p class="mt-2 text-[12px] text-dim">{{ t("trackEditor.searchCoverHint") }}</p>

      <div class="mt-4 flex gap-2">
        <LayeredSelect v-model="source" :options="sourceOptions" label="Source" variant="box" class="w-32" />
        <form class="flex flex-1 gap-2" @submit.prevent="doSearch">
          <input v-model="query" :placeholder="t('trackEditor.searchCoverPlaceholder')" class="h-10 flex-1 border border-line bg-bg px-3 text-sm text-fg outline-none focus:border-accent" />
          <button type="submit" class="ak-clip-tr flex h-10 items-center gap-2 border border-line px-4 text-[13px] font-semibold transition-colors hover:border-accent hover:text-accent" :disabled="searching"><Search :size="15" />{{ searching ? t("trackEditor.searchCoverSearching") : t("library.search") }}</button>
        </form>
      </div>

      <div class="mt-4 min-h-0 flex-1 overflow-y-auto">
        <div v-if="searching" class="flex items-center gap-2 font-mono text-[12px] uppercase tracking-[0.2em] text-accent"><span class="ak-pulse" style="width: 12px; height: 12px"></span>{{ t("trackEditor.searchCoverSearching") }}</div>
        <div v-else-if="results.length === 0" class="py-8 text-center text-xs text-muted">{{ t("trackEditor.searchCoverEmpty") }}</div>
        <div v-else class="grid gap-2">
          <button
            v-for="song in results"
            :key="song.id"
            type="button"
            class="ak-clip-tr flex items-center gap-3 border border-line p-2 text-left transition-colors hover:border-accent min-w-0"
            :class="{ 'border-accent bg-accent/5': selectedId === song.id }"
            @click="pickSong(song)"
          >
            <span class="grid h-10 w-10 shrink-0 place-items-center overflow-hidden border border-line bg-bg text-lg font-black text-white/90" :style="{ backgroundColor: '#4c72a6' }">
              <img v-if="song.coverUrl" :src="song.coverUrl" alt="" class="h-full w-full object-cover" @error="coverUrls[song.id] = null" />
              <template v-if="!song.coverUrl || coverUrls[song.id] === null">{{ song.title.charAt(0) }}</template>
            </span>
            <span class="min-w-0">
              <span class="block break-all text-sm font-semibold">{{ song.title }}</span>
              <span class="block break-all text-xs text-muted">{{ song.artist }} · {{ song.album }}</span>
            </span>
            <span v-if="fetching && selectedId === song.id" class="ml-auto text-[11px] text-accent">{{ t("trackEditor.searchCoverSearching") }}</span>
          </button>
        </div>
      </div>

      <div class="mt-5 flex justify-end gap-3">
        <button type="button" class="ak-clip-tr h-10 border border-line px-4 text-[13px] font-semibold" @click="onClose">{{ t("lyricsEditor.close") }}</button>
      </div>
    </div>
  </div>
</template>