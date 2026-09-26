<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { ArrowLeft, Download, ExternalLink, Eye, EyeOff, FileText, PenLine, RotateCcw, Save, Search, X } from "@lucide/vue";
import { openUrl } from "@tauri-apps/plugin-opener";
import { describeError, invoke } from "../api";
import { detectFormat } from "lyric-kit";
import { readLyricSource, tagLyric } from "../lib/lyricTag";
import { lyricPlatformLink, lyricSidecarPath, parseAmllAddress, parseNeteaseId, parseQqIds } from "../lib/lyricLink";
import { lyricFromAmll, lyricFromNetease, lyricFromQqBest, LyricResolver, searchNetease, searchQq } from "../lib/lyricSources";
import { pushToast } from "../lib/toast";
import { usePlayerStore } from "../stores/player";
import { useWebdavStore } from "../stores/webdav";
import { useProfileStore } from "../stores/profile";
import { useLyricsStore } from "../stores/lyrics";
import LayeredSelect from "../components/LayeredSelect.vue";

import type { LyricFetchResult } from "../lib/lyricSources";

const props = defineProps<{ path: string | null }>();
const emit = defineEmits<{ back: []; friendlyError: [error: unknown] }>();
const { t } = useI18n();
const player = usePlayerStore();
const webdav = useWebdavStore();
const profile = useProfileStore();
const lyrics = useLyricsStore();

const content = ref("");
const savedSnapshot = ref("");
const loading = ref(false);
const saving = ref(false);
const loadError = ref("");
const editorOpen = ref(false);
const downloadProvider = ref<"netease" | "qq" | "amll">("netease");
const downloadInput = ref("");
const downloading = ref(false);
const searching = ref(false);

const track = computed(() => player.tracks.find((item) => item.path === props.path) ?? null);
const tag = computed(() => readLyricSource(content.value));
const source = computed(() => tag.value?.source?.trim().toLowerCase() || "local");
/** Local lyrics are never tagged, so an absent tag means the sidecar is local. */
const isLocal = computed(() => !tag.value || source.value === "local");
const format = computed(() => {
  try {
    return detectFormat(content.value);
  } catch {
    return "";
  }
});

const sidecarPath = computed(() => lyricSidecarPath(props.path));
const platformLink = computed(() => lyricPlatformLink(source.value, tag.value?.sourceId));
const dirty = computed(() => content.value !== savedSnapshot.value);
const disabled = computed(() => !!props.path && profile.isLyricsDisabled(props.path));

/** Suppress or restore lyrics for this track, reflecting playback immediately. */
function toggleDisabled() {
  const path = props.path;
  if (!path) return;
  profile.toggleLyricsDisabled(path);
  const current = player.currentTrack;
  if (current && current.path === path) void lyrics.loadForTrack(current);
}

const providerKey = computed(() =>
  ["qq", "netease", "amll", "local"].includes(source.value)
    ? `settings.lyrics.provider.${source.value}.short`
    : null,
);
const providerLabel = computed(() => (providerKey.value ? t(providerKey.value) : source.value));
const downloadIsAmll = computed(() => downloadProvider.value === "amll");
const inputLabel = computed(() => (downloadIsAmll.value ? t("lyricsEditor.addressLabel") : t("lyricsEditor.idLabel")));
const inputPlaceholder = computed(() => (downloadIsAmll.value ? "https://music.163.com/song?id=186016" : "186016"));
const idHelpKey = computed(() => {
  if (downloadIsAmll.value) return "lyricsEditor.idHelpAmll";
  return downloadProvider.value === "qq" ? "lyricsEditor.idHelpQq" : "lyricsEditor.idHelpNetease";
});
const providerOptions = computed<Array<{ value: "netease" | "qq" | "amll"; label: string }>>(() => [
  { value: "netease", label: t("settings.lyrics.provider.netease.short") },
  { value: "qq", label: t("settings.lyrics.provider.qq.short") },
  { value: "amll", label: t("settings.lyrics.provider.amll.short") },
]);

async function loadLyrics() {
  content.value = "";
  savedSnapshot.value = "";
  loadError.value = "";
  if (!props.path) return;
  loading.value = true;
  let text: string | null = null;
  try {
    text = await invoke<string>("read_track_lyrics", { path: props.path });
  } catch (error) {
    // WebDAV may be unconfigured or the sidecar missing; the cache is the fallback.
    loadError.value = describeError(error);
  }
  if (!text) {
    try {
      const cached = await invoke<string | null>("get_cached_lyrics", { path: props.path });
      if (cached) {
        text = cached;
        loadError.value = "";
      }
    } catch (error) {
      if (!loadError.value) loadError.value = describeError(error);
    }
  }
  content.value = text ?? "";
  savedSnapshot.value = content.value;
  loading.value = false;
}

async function save() {
  if (!props.path || saving.value) return;
  saving.value = true;
  try {
    await invoke("save_track_lyrics", { path: props.path, content: content.value });
    savedSnapshot.value = content.value;
    loadError.value = "";
    pushToast("success", t("lyricsEditor.saved"));
  } catch (error) {
    emit("friendlyError", error);
  } finally {
    saving.value = false;
  }
}

function reset() {
  content.value = savedSnapshot.value;
}

function openSource() {
  if (platformLink.value) void openUrl(platformLink.value).catch((error) => emit("friendlyError", error));
}

/** Fetch lyrics for a manually entered platform id / address and save them. */
async function downloadLyrics() {
  if (!props.path || downloading.value) return;
  const raw = downloadInput.value.trim();
  if (!raw) {
    pushToast("warning", t("lyricsEditor.needId"));
    return;
  }
  downloading.value = true;
  try {
    let hit: LyricFetchResult | null = null;
    if (downloadProvider.value === "netease") {
      const id = parseNeteaseId(raw);
      if (!id) { pushToast("error", t("lyricsEditor.invalidId")); return; }
      const text = await lyricFromNetease(id);
      hit = text ? { content: text, sourceId: `ncm/${id}` } : null;
    } else if (downloadProvider.value === "qq") {
      const ids = parseQqIds(raw);
      if (!ids) { pushToast("error", t("lyricsEditor.invalidId")); return; }
      hit = await lyricFromQqBest(ids);
    } else {
      const ids = parseAmllAddress(raw);
      if (!ids) { pushToast("error", t("lyricsEditor.invalidAddress")); return; }
      hit = await lyricFromAmll(ids, {
        title: track.value?.title ?? "",
        artist: track.value?.artist ?? "",
        album: track.value?.album ?? "",
      });
    }

    if (!hit?.content.trim()) {
      pushToast("error", t("lyricsEditor.noLyricsFound"));
      return;
    }

    // Tag with the origin so the "来源" section and platform link keep working.
    const tagged = tagLyric(hit.content, { source: downloadProvider.value, sourceId: hit.sourceId });
    await invoke("save_track_lyrics", { path: props.path, content: tagged });
    content.value = tagged;
    savedSnapshot.value = tagged;
    loadError.value = "";
    editorOpen.value = true;
    pushToast("success", t("lyricsEditor.downloaded"));
  } catch (error) {
    emit("friendlyError", error);
  } finally {
    downloading.value = false;
  }
}

/** Search the selected provider by title/artist and fill in the best match. */
async function autoSearch() {
  if (!props.path || searching.value) return;
  const query = {
    title: track.value?.title ?? "",
    artist: track.value?.artist ?? "",
    album: track.value?.album ?? "",
  };
  if (!query.title.trim()) {
    pushToast("warning", t("lyricsEditor.searchNoTrack"));
    return;
  }
  searching.value = true;
  try {
    if (downloadProvider.value === "amll") {
      const resolver = new LyricResolver(query);
      await resolver.netease();
      await resolver.qq();
      const ids = resolver.cached;
      const address = ids.netease
        ? `https://music.163.com/song?id=${ids.netease}`
        : ids.qqMid
          ? `https://y.qq.com/n/ryqq/songDetail/${ids.qqMid}`
          : null;
      if (!address) { pushToast("error", t("lyricsEditor.searchEmpty")); return; }
      downloadInput.value = address;
      pushToast("success", t("lyricsEditor.searchFound", { value: address }));
      return;
    }

    const match = downloadProvider.value === "qq" ? await searchQq(query) : await searchNetease(query);
    if (!match) { pushToast("error", t("lyricsEditor.searchEmpty")); return; }
    downloadInput.value = match.id;
    pushToast("success", t("lyricsEditor.searchFound", { value: `${match.title} · ${match.artist}` }));
  } catch (error) {
    emit("friendlyError", error);
  } finally {
    searching.value = false;
  }
}

watch(() => props.path, loadLyrics, { immediate: true });
</script>

<template>
  <div class="mx-auto w-full max-w-4xl px-6 py-6 lg:px-8 lg:py-8">
    <button type="button" class="flex items-center gap-2 text-[12px] font-semibold uppercase tracking-[0.2em] text-dim transition-colors hover:text-fg" @click="emit('back')">
      <ArrowLeft :size="15" />{{ t("lyricsEditor.back") }}
    </button>

    <header class="mt-6 flex items-start gap-5 border-b border-line pb-6">
      <span class="ak-frame grid h-20 w-20 shrink-0 place-items-center overflow-hidden border border-line text-2xl font-black text-white/90" :style="{ backgroundColor: track?.color ?? 'var(--color-accent)' }">
        <img v-if="track?.cover" :src="track.cover" alt="" decoding="async" class="h-full w-full object-cover" />
        <FileText v-else :size="30" />
      </span>
      <div class="min-w-0">
        <p class="flex items-center gap-2 text-[13px] font-semibold uppercase tracking-[0.35em] text-accent"><span class="h-2 w-2 bg-accent"></span>{{ t("lyricsEditor.eyebrow") }}</p>
        <h1 class="mt-3 truncate text-3xl font-black leading-tight tracking-tight sm:text-4xl">{{ track?.title ?? t("lyricsEditor.title") }}</h1>
        <p class="mt-2 truncate text-sm text-muted">{{ track?.artist }}<span v-if="track?.album"> · {{ track.album }}</span></p>
      </div>
    </header>

    <div v-if="loading" class="mt-8 flex items-center gap-2 font-mono text-[12px] uppercase tracking-[0.2em] text-accent"><span class="ak-pulse" style="width: 14px; height: 9px"></span>{{ t("lyricsEditor.loading") }}</div>

    <template v-else>
      <div class="mt-6 grid gap-4 lg:grid-cols-2">
        <section class="ak-frame border border-line bg-surface p-6">
          <h2 class="flex items-center gap-2 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("lyricsEditor.location") }}</h2>
          <p class="mt-3 text-xs text-muted">{{ t("lyricsEditor.locationHint") }}</p>
          <dl class="mt-4 grid gap-3">
            <div>
              <dt class="text-[11px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("lyricsEditor.remotePath") }}</dt>
              <dd class="mt-1 break-all font-mono text-xs">{{ sidecarPath ?? "—" }}</dd>
            </div>
          </dl>
          <p v-if="!webdav.url" class="mt-4 border-l-2 border-accent/60 pl-3 text-xs text-muted">{{ t("lyricsEditor.notConfigured") }}</p>
        </section>

        <section class="ak-frame border border-line bg-surface p-6">
          <h2 class="flex items-center gap-2 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("lyricsEditor.source") }}</h2>
          <p class="mt-3 text-xs text-muted">{{ t("lyricsEditor.sourceHint") }}</p>
          <p class="mt-4 text-2xl font-black tracking-tight">{{ providerLabel }}</p>
          <dl class="mt-4 grid gap-3">
            <div v-if="tag?.sourceId">
              <dt class="text-[11px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("lyricsEditor.sourceId") }}</dt>
              <dd class="mt-1 break-all font-mono text-xs">{{ tag.sourceId }}</dd>
            </div>
            <div v-if="tag?.fetched">
              <dt class="text-[11px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("lyricsEditor.fetched") }}</dt>
              <dd class="mt-1 font-mono text-xs">{{ tag.fetched }}</dd>
            </div>
            <div v-if="format">
              <dt class="text-[11px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("lyricsEditor.format") }}</dt>
              <dd class="mt-1 font-mono text-xs uppercase">{{ format }}</dd>
            </div>
          </dl>
          <div v-if="platformLink" class="mt-4">
            <p class="text-[11px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("lyricsEditor.linkUrl") }}</p>
            <p class="mt-1 break-all font-mono text-xs">{{ platformLink }}</p>
            <button type="button" class="ak-clip-tr mt-3 flex h-10 items-center gap-2 bg-accent px-4 text-[13px] font-bold text-accent-fg transition-transform hover:scale-[1.02] active:scale-95" @click="openSource"><ExternalLink :size="15" />{{ t("lyricsEditor.openSource") }}</button>
          </div>
          <p v-else-if="isLocal" class="mt-4 border-l-2 border-line-strong pl-3 text-xs text-muted">{{ t("lyricsEditor.localNote") }}</p>
          <p v-else class="mt-4 border-l-2 border-line-strong pl-3 text-xs text-muted">{{ t("lyricsEditor.noLink") }}</p>
        </section>
      </div>

      <section class="ak-frame mt-4 border border-line bg-surface p-6">
        <div class="flex flex-wrap items-center justify-between gap-3">
          <div>
            <h2 class="flex items-center gap-2 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("lyricsEditor.content") }}</h2>
            <p class="mt-2 text-xs text-muted">{{ t("lyricsEditor.contentHint") }}</p>
          </div>
          <div class="flex flex-wrap items-center gap-3">
            <button type="button" class="ak-clip-tr flex h-10 items-center gap-2 border border-line px-4 text-[13px] font-semibold transition-colors hover:border-accent hover:text-accent" @click="toggleDisabled"><EyeOff v-if="!disabled" :size="15" /><Eye v-else :size="15" />{{ disabled ? t("lyricsEditor.enableLyrics") : t("lyricsEditor.disableLyrics") }}</button>
            <button type="button" class="ak-clip-tr flex h-10 items-center gap-2 bg-accent px-4 text-[13px] font-bold text-accent-fg transition-transform hover:scale-[1.02] active:scale-95" @click="editorOpen = true"><PenLine :size="15" />{{ t("lyricsEditor.edit") }}</button>
          </div>
        </div>
        <p v-if="disabled" class="mt-4 border-l-2 border-accent/60 pl-3 text-xs text-muted">{{ t("lyricsEditor.disabledNote") }}</p>
      </section>

      <section class="ak-frame mt-4 border border-line bg-surface p-6">
        <h2 class="flex items-center gap-2 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim"><Download :size="15" />{{ t("lyricsEditor.downloadTitle") }}</h2>
        <p class="mt-3 text-xs text-muted">{{ t("lyricsEditor.downloadHint") }}</p>
        <div class="mt-4 grid gap-4 sm:grid-cols-2">
          <div class="block">
            <span class="text-[11px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("lyricsEditor.providerLabel") }}</span>
            <LayeredSelect v-model="downloadProvider" :options="providerOptions" :label="t('lyricsEditor.providerLabel')" class="mt-1" />
          </div>
          <label class="block">
            <span class="text-[11px] font-semibold uppercase tracking-[0.2em] text-dim">{{ inputLabel }}</span>
            <input v-model="downloadInput" type="text" spellcheck="false" class="mt-1 h-10 w-full border border-line bg-bg px-3 font-mono text-xs outline-none focus:border-accent" :placeholder="inputPlaceholder" />
          </label>
        </div>
        <p class="mt-3 border-l-2 border-line-strong pl-3 text-xs text-muted">{{ t(idHelpKey) }}</p>
        <div class="mt-4 flex flex-wrap items-center gap-3">
          <button type="button" class="ak-clip-tr flex h-10 items-center gap-2 border border-line px-4 text-[13px] font-semibold transition-colors hover:border-accent hover:text-accent disabled:opacity-50" :disabled="searching || downloading || !props.path" @click="autoSearch"><Search :size="15" />{{ searching ? t("lyricsEditor.searching") : t("lyricsEditor.search") }}</button>
          <button type="button" class="ak-clip-tr flex h-10 items-center gap-2 bg-accent px-5 text-[13px] font-bold text-accent-fg transition-transform hover:scale-[1.02] active:scale-95 disabled:opacity-50" :disabled="downloading || !props.path" @click="downloadLyrics"><Download :size="15" />{{ downloading ? t("lyricsEditor.downloading") : t("lyricsEditor.download") }}</button>
        </div>
      </section>
    </template>

    <Teleport to="body">
      <div v-if="editorOpen" class="fixed inset-0 z-[80] grid place-items-center bg-black/70 p-4">
        <div class="ak-frame flex max-h-[90vh] w-full max-w-3xl flex-col border border-line bg-surface p-6">
          <div class="flex items-start justify-between gap-3">
            <div>
              <h3 class="text-sm font-bold uppercase tracking-[0.2em]">{{ t("lyricsEditor.content") }}</h3>
              <p class="mt-2 text-xs text-muted">{{ t("lyricsEditor.contentHint") }}</p>
            </div>
            <button type="button" class="grid h-8 w-8 shrink-0 place-items-center text-dim hover:text-fg" :title="t('lyricsEditor.close')" @click="editorOpen = false"><X :size="16" /></button>
          </div>
          <textarea v-model="content" spellcheck="false" class="mt-4 min-h-[45vh] flex-1 resize-none border border-line bg-bg p-4 font-mono text-xs leading-relaxed outline-none focus:border-accent" :placeholder="t('lyricsEditor.placeholder')"></textarea>
          <p v-if="loadError" class="mt-3 text-xs text-muted">{{ t("lyricsEditor.loadFailed", { value: loadError }) }}</p>
          <div class="mt-4 flex flex-wrap items-center gap-3">
            <span v-if="dirty" class="flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-[0.2em] text-accent"><span class="h-1.5 w-1.5 bg-accent"></span>{{ t("lyricsEditor.dirty") }}</span>
            <div class="ml-auto flex items-center gap-3">
              <button type="button" class="ak-clip-tr flex h-10 items-center gap-2 border border-line px-4 text-[13px] font-semibold transition-colors hover:border-accent hover:text-accent disabled:opacity-50" :disabled="!dirty" @click="reset"><RotateCcw :size="15" />{{ t("lyricsEditor.reset") }}</button>
              <button type="button" class="ak-clip-tr flex h-10 items-center gap-2 bg-accent px-5 text-[13px] font-bold text-accent-fg transition-transform hover:scale-[1.02] active:scale-95 disabled:opacity-50" :disabled="saving || !props.path" @click="save"><Save :size="15" />{{ saving ? t("lyricsEditor.saving") : t("lyricsEditor.save") }}</button>
            </div>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>
