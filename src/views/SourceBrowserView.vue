<!--
  Copyright 2026 Atemukesu
  SPDX-License-Identifier: GPL-3.0-only
 -->

<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import { requestMeta } from "../directives/requestMeta";
import { ChevronRight, Folder, FolderOpen, Play, Search, Server, HardDrive, ArrowUp, ArrowLeft } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { usePlayerStore } from "../stores/player";
import type { Track } from "../stores/player";
import { useSourcesStore } from "../stores/sources";
import { trackFromEntry, type RemoteEntry } from "../lib/remote";
import { sourceLabel } from "../lib/sources";
import { coverPending, initial } from "../lib/format";
import { LoaderCircle } from "@lucide/vue";
import TrackContextMenu from "../components/TrackContextMenu.vue";
import { describeError } from "../api";
import { pushToast } from "../lib/toast";

/** A node in the breadcrumb trail; root is represented by an empty path. */
interface Crumb {
  name: string;
  path: string;
}

const props = withDefaults(defineProps<{ sourceId: string; loading?: boolean; downloadingTrackId?: number | null }>(), { loading: false, downloadingTrackId: null });
const emit = defineEmits<{ back: []; downloadMetadata: [track: Track]; editLyrics: [track: Track]; editInfo: [track: Track]; showInfo: [track: Track]; openArtist: [artist: string]; openAlbum: [album: string] }>();
const { t } = useI18n();
const player = usePlayerStore();
const sources = useSourcesStore();

const vRequestMeta = requestMeta;
const source = computed(() => sources.sources.find((item) => item.id === props.sourceId) ?? null);
const sourceName = computed(() => source.value ? sourceLabel(source.value) : "");

/** The current directory path ("" is the source root). */
const currentPath = ref("");
const entries = ref<RemoteEntry[]>([]);
const loadingDir = ref(false);
const error = ref("");
const query = ref("");
const contextTrack = ref<Track | null>(null);
const contextPosition = ref({ x: 0, y: 0 });

/** Build the breadcrumb trail from the current path segments. */
const crumbs = computed<Crumb[]>(() => {
  const segments = currentPath.value.split("/").filter(Boolean);
  const trail: Crumb[] = [{ name: sourceName.value || t("nav.sources"), path: "" }];
  let accumulated = "";
  for (const segment of segments) {
    accumulated = accumulated ? `${accumulated}/${segment}` : segment;
    trail.push({ name: segment, path: accumulated });
  }
  return trail;
});

const filteredEntries = computed(() => {
  const value = query.value.trim().toLocaleLowerCase();
  const list = entries.value;
  if (!value) return list;
  return list.filter((entry) => entry.name.toLocaleLowerCase().includes(value));
});

const folderCount = computed(() => entries.value.filter((entry) => entry.isDir).length);
const fileCount = computed(() => entries.value.filter((entry) => !entry.isDir).length);

/**
 * The library has already indexed this source and resolved cached covers for the
 * exact same files. Rather than show a blank icon for every audio row, map each
 * entry's path to its library track so the cached cover (and title/color) can be
 * reused directly. Matched by source + path; missing entries fall back to the
 * remote listing's own fields.
 */
const trackByPath = computed(() => {
  const map = new Map<string, Track>();
  for (const track of player.tracks) {
    if (track.sourceId === props.sourceId && track.path) map.set(track.path, track);
  }
  return map;
});

/** Resolve the cached cover for an audio entry, reusing the library's resolved art. */
function audioCover(entry: RemoteEntry): string | undefined {
  return trackByPath.value.get(entry.path)?.cover;
}
function audioColor(entry: RemoteEntry): string {
  return trackByPath.value.get(entry.path)?.color ?? "#9ca3af";
}

async function load(path: string) {
  currentPath.value = path;
  loadingDir.value = true;
  error.value = "";
  try {
    entries.value = await sources.listDirectory(props.sourceId, path);
  } catch (cause) {
    entries.value = [];
    error.value = describeError(cause);
  } finally {
    loadingDir.value = false;
  }
}

/** Open a folder: drill into it and reset the search box. */
function openFolder(entry: RemoteEntry) {
  query.value = "";
  void load(entry.localPath);
}

/** Jump to a breadcrumb segment. */
function navigateCrumb(crumb: Crumb) {
  if (crumb.path === currentPath.value) return;
  query.value = "";
  void load(crumb.path);
}

/**
 * Resolve the library Track for an audio entry. The directory listing is a raw,
 * metadata-free view of the files; the library has already downloaded and cached
 * each track's cover and tags. Prefer that real Track so playback shows the same
 * cover/metadata elsewhere in the app (e.g. the player bar) instead of a
 * freshly synthesised row with a MIME type for an album name. Fall back to a
 * synthesised track only when the file is not yet in the library.
 */
function toTrack(entry: RemoteEntry): Track {
  const existing = trackByPath.value.get(entry.path);
  if (existing) return existing;
  return trackFromEntry(entry, 0, source.value ? { id: source.value.id, name: source.value.name, kind: source.value.kind } : undefined);
}

/** Play every audio file nested under the current directory, starting at `entry`. */
async function playFolder(entry: RemoteEntry) {
  loadingDir.value = true;
  try {
    const files = await sources.listAudioUnder(props.sourceId, entry.localPath);
    const tracks = files.map((file) => toTrack(file));
    if (tracks.length === 0) {
      pushToast("info", t("sourceBrowser.emptyFolder"));
      return;
    }
    const start = tracks.find((track) => track.path === entry.path) ?? tracks[0];
    player.playInQueue(tracks, start);
  } catch (cause) {
    pushToast("error", describeError(cause));
  } finally {
    loadingDir.value = false;
  }
}

/**
 * Play a single file, but queue the entire folder's audio (siblings only, not
 * recursing into subfolders) so the next/prev controls walk the same directory.
 * The clicked file is the starting track.
 */
function openContextMenu(event: MouseEvent, entry: RemoteEntry) { contextTrack.value = toTrack(entry); contextPosition.value = { x: event.clientX, y: event.clientY }; }
function playFile(entry: RemoteEntry) {
  const siblings = entries.value.filter((item) => !item.isDir).map((item) => toTrack(item));
  if (siblings.length === 0) return;
  const start = toTrack(entry);
  player.playInQueue(siblings, start);
}

watch(
  () => props.sourceId,
  (id) => { if (id) void load(""); },
  { immediate: true },
);

function onBack() {
  emit("back");
}

const scrollEl = ref<HTMLElement | null>(null);
watch(() => currentPath.value, () => void nextTick(() => scrollEl.value?.scrollTo({ top: 0 })));
</script>

<template>
  <div ref="scrollEl" class="h-full w-full overflow-y-auto">
    <div class="mx-auto flex w-full max-w-6xl flex-col px-4 py-5 sm:px-6 sm:py-6 lg:px-8 lg:py-8">
      <header class="shrink-0 border-b border-line pb-6">
        <div class="flex flex-wrap items-end justify-between gap-4">
          <div class="min-w-0">
            <p class="flex items-center gap-2 text-[13px] font-semibold uppercase tracking-[0.35em] text-accent"><span class="h-2 w-2 bg-accent"></span>{{ t("nav.sources") }}</p>
            <h1 class="mt-4 flex items-center gap-3 text-3xl font-black leading-tight tracking-tight sm:text-4xl">
              <Server v-if="source?.kind === 'webdav'" :size="26" :stroke-width="2" class="text-accent" />
              <HardDrive v-else :size="26" :stroke-width="2" class="text-accent" />
              <span class="truncate">{{ sourceName }}</span>
            </h1>
            <p class="mt-3 font-mono text-sm uppercase tracking-[0.2em] text-dim">{{ folderCount }} {{ t("sourceBrowser.folders") }} · {{ fileCount }} {{ t("sourceBrowser.files") }}</p>
          </div>
          <button type="button" class="ak-clip-tr flex h-10 items-center gap-2 border border-line px-4 text-[13px] font-semibold uppercase tracking-[0.2em] transition-colors hover:border-accent hover:text-accent" @click="onBack"><ArrowLeft :size="15" />{{ t("controls.return") }}</button>
        </div>

        <nav class="mt-5 flex flex-wrap items-center gap-1 overflow-x-auto text-[13px] font-semibold uppercase tracking-[0.15em] text-dim">
          <button v-for="(crumb, index) in crumbs" :key="crumb.path" type="button" class="flex shrink-0 items-center gap-1 transition-colors hover:text-accent" :class="index === crumbs.length - 1 ? 'text-accent' : ''" @click="navigateCrumb(crumb)">
            <span class="truncate">{{ crumb.name }}</span>
            <ChevronRight v-if="index < crumbs.length - 1" :size="14" class="text-line-strong" />
          </button>
        </nav>

        <div class="mt-4 flex items-center gap-3">
          <button v-if="currentPath" type="button" class="grid h-10 w-10 shrink-0 place-items-center border border-line text-dim transition-colors hover:border-accent hover:text-accent" :title="t('sourceBrowser.up')" @click="navigateCrumb(crumbs[crumbs.length - 2])"><ArrowUp :size="16" :stroke-width="2" /></button>
          <label class="flex h-10 min-w-0 flex-1 items-center gap-2 border border-line bg-surface px-3 @2xl:w-72 @2xl:flex-none"><Search :size="16" class="text-dim" /><input v-model="query" class="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-dim" :placeholder="t('sourceBrowser.search')" aria-label="Search folder" /></label>
        </div>
      </header>

      <div v-if="error" class="ak-frame mt-6 border border-red-500/40 bg-red-500/5 p-4 text-sm text-red-500">{{ error }}</div>

      <div v-if="loadingDir" class="mt-12 grid place-items-center gap-3 text-dim"><span class="ak-pulse"></span><p class="text-sm font-semibold uppercase tracking-[0.2em]">{{ t("sourceBrowser.loading") }}</p></div>

      <div v-else-if="filteredEntries.length === 0" class="ak-frame mt-8 grid place-items-center gap-3 border border-dashed border-line bg-surface/50 px-6 py-20 text-center">
        <FolderOpen :size="36" class="text-dim" />
        <p class="text-base font-semibold uppercase tracking-[0.2em]">{{ query ? t("sourceBrowser.noMatch") : t("sourceBrowser.empty") }}</p>
        <p v-if="!query" class="max-w-md text-sm text-muted">{{ t("sourceBrowser.emptyDesc") }}</p>
      </div>

      <ul v-else class="mt-6 divide-y divide-line border-t border-line">
        <li v-for="entry in filteredEntries" :key="entry.localPath">
          <button v-if="entry.isDir" type="button" class="group flex w-full items-center gap-4 px-2 py-3 text-left transition-colors hover:bg-fg/5" @click="openFolder(entry)">
            <span class="grid h-11 w-11 shrink-0 place-items-center bg-accent/10 text-accent"><Folder :size="20" :stroke-width="2" /></span>
            <span class="min-w-0 flex-1">
              <p class="truncate text-sm font-bold">{{ entry.name }}</p>
              <p class="mt-0.5 font-mono text-[11px] uppercase tracking-[0.15em] text-dim">{{ t("sourceBrowser.folder") }}</p>
            </span>
            <span class="flex items-center gap-2">
              <span class="ak-clip grid h-9 w-9 shrink-0 place-items-center border border-line text-dim opacity-0 transition-opacity group-hover:opacity-100 hover:border-accent hover:text-accent" :title="t('sourceBrowser.playFolder')" @click.stop="playFolder(entry)"><Play :size="15" :stroke-width="2.2" /></span>
              <ChevronRight :size="16" class="text-dim" />
            </span>
          </button>
          <button v-else type="button" class="group flex w-full items-center gap-4 px-2 py-3 text-left transition-colors hover:bg-fg/5" :class="player.currentTrack?.path === entry.path ? 'ak-select relative z-10 border-transparent' : ''" v-request-meta="toTrack(entry)" @click="playFile(entry)" @contextmenu.prevent="openContextMenu($event, entry)">
            <span class="grid h-11 w-11 shrink-0 place-items-center overflow-hidden text-lg font-black text-white/90" :style="{ backgroundColor: audioColor(entry) }">
              <img v-if="audioCover(entry)" :src="audioCover(entry)" alt="" loading="lazy" decoding="async" class="h-full w-full object-cover" />
              <LoaderCircle v-else-if="coverPending(toTrack(entry))" :size="18" :stroke-width="2" class="animate-spin" />
              <template v-else>{{ initial(toTrack(entry)) }}</template>
            </span>
            <span class="min-w-0 flex-1">
              <p class="truncate text-sm font-bold">{{ entry.name }}</p>
              <p class="mt-0.5 font-mono text-[11px] uppercase tracking-[0.15em] text-dim">{{ entry.size ? `${(entry.size / 1_048_576).toFixed(1)} MB` : t("sourceBrowser.audio") }}</p>
            </span>
            <span class="ak-clip grid h-9 w-9 shrink-0 place-items-center bg-accent text-accent-fg opacity-0 transition-opacity group-hover:opacity-100" :title="t('library.play')"><Play :size="15" :stroke-width="2.2" /></span>
          </button>
        </li>
      </ul>
    </div>

    <TrackContextMenu v-if="contextTrack" :track="contextTrack" :x="contextPosition.x" :y="contextPosition.y" :selected-paths="[]" :downloading="downloadingTrackId === contextTrack.id" @close="contextTrack = null" @download-metadata="emit('downloadMetadata', $event)" @edit-lyrics="emit('editLyrics', $event)" @edit-info="emit('editInfo', $event)" @show-info="emit('showInfo', $event)" />
  </div>
</template>
