<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { ArrowLeft } from "@lucide/vue";
import { describeError, invoke } from "../api";
import { initial } from "../lib/format";
import { usePlayerStore } from "../stores/player";
import type { Track } from "../stores/player";
import { useProfileStore } from "../stores/profile";
import type { TrackTags } from "../lib/remote";

const props = defineProps<{ path: string | null }>();
const emit = defineEmits<{ back: [] }>();
const { t } = useI18n();
const player = usePlayerStore();
const profile = useProfileStore();

interface InfoRow { label: string; value: string; wide?: boolean; wrap?: boolean }

const tags = ref<TrackTags | null>(null);
const loading = ref(false);
const loadError = ref("");
const coverUrl = ref<string | null>(null);

const track = computed<Track | null>(() => player.tracks.find((item) => item.path === props.path) ?? null);
const playCount = computed(() => (props.path ? profile.profile.playCounts[props.path] ?? 0 : 0));
const favorite = computed(() => profile.isFavorite(props.path ?? undefined));
const duration = computed(() => {
  if (track.value && player.currentTrack?.path === track.value.path && player.duration > 0) {
    return formatSeconds(Math.floor(player.duration));
  }
  return track.value?.duration || "—";
});

function formatSeconds(seconds: number): string {
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}
/** First non-empty value wins, so tag rows can fall back to the library-provided track fields. */
function pick(...values: Array<string | null | undefined>): string {
  for (const value of values) {
    if (value && value.trim()) return value;
  }
  return "—";
}
function text(value: string | null | undefined): string {
  return pick(value);
}
function numberText(value: number | null | undefined): string {
  return value === null || value === undefined ? "—" : String(value);
}
function yesNo(value: boolean): string {
  return value ? t("trackInfo.yes") : t("trackInfo.no");
}

/** Show the embedded-cover state as soon as either the tags or a known cover tell us. */
const coverState = computed(() => {
  if (tags.value) return tags.value.hasCover ? t("trackInfo.coverEmbedded") : t("trackInfo.coverNone");
  return coverUrl.value ? t("trackInfo.coverEmbedded") : "—";
});

const tagRows = computed<InfoRow[]>(() => {
  const value = tags.value;
  return [
    { label: t("trackEditor.titleLabel"), value: pick(value?.title, track.value?.title), wide: true },
    { label: t("trackEditor.artistLabel"), value: pick(value?.artist, track.value?.artist) },
    { label: t("trackEditor.albumLabel"), value: pick(value?.album, track.value?.album) },
    { label: t("trackEditor.albumArtistLabel"), value: text(value?.albumArtist) },
    { label: t("trackEditor.genreLabel"), value: text(value?.genre) },
    { label: t("trackEditor.yearLabel"), value: numberText(value?.year) },
    { label: t("trackEditor.trackNumberLabel"), value: numberText(value?.trackNumber) },
    { label: t("trackEditor.trackTotalLabel"), value: numberText(value?.trackTotal) },
    { label: t("trackEditor.discNumberLabel"), value: numberText(value?.discNumber) },
    { label: t("trackEditor.discTotalLabel"), value: numberText(value?.discTotal) },
    { label: t("trackEditor.cover"), value: coverState.value },
    { label: t("trackEditor.commentLabel"), value: text(value?.comment), wide: true, wrap: true },
  ];
});

const fileRows = computed<InfoRow[]>(() => [
  { label: t("trackInfo.path"), value: props.path ?? "—", wide: true, wrap: true },
  { label: t("trackInfo.modified"), value: text(track.value?.modified) },
  { label: t("trackInfo.duration"), value: duration.value },
  { label: t("trackInfo.playCount"), value: numberText(playCount.value) },
  { label: t("trackInfo.favorite"), value: yesNo(favorite.value) },
  { label: t("trackInfo.metaLoaded"), value: yesNo(!!track.value?.metaLoaded) },
  { label: t("trackInfo.cached"), value: yesNo(!!track.value?.assetsHydrated) },
]);

async function load() {
  tags.value = null;
  loadError.value = "";
  coverUrl.value = track.value?.cover ?? null;
  if (!props.path) return;

  loading.value = true;
  try {
    if (!coverUrl.value) {
      coverUrl.value = await invoke<string | null>("get_cached_cover", { path: props.path }).catch(() => null);
    }
    tags.value = await invoke<TrackTags>("read_track_tags", { path: props.path });
  } catch (error) {
    loadError.value = describeError(error);
  } finally {
    loading.value = false;
  }
}

watch(() => props.path, load, { immediate: true });
</script>

<template>
  <div class="fixed inset-0 z-40 flex flex-col overflow-hidden bg-bg text-fg sm:items-center sm:justify-center sm:bg-black/60 sm:p-6 sm:backdrop-blur-sm">
    <div class="ak-info-panel flex h-full min-h-0 w-full flex-col overflow-hidden bg-bg sm:ak-frame sm:h-auto sm:max-h-[88vh] sm:max-w-4xl sm:border sm:border-line sm:shadow-2xl">
      <header class="flex h-16 shrink-0 items-center gap-4 border-b border-line px-5">
        <button type="button" class="flex items-center gap-2 text-[12px] font-semibold uppercase tracking-[0.2em] text-dim transition-colors hover:text-fg" @click="emit('back')">
          <ArrowLeft :size="16" />{{ t("trackInfo.back") }}
        </button>
        <span class="ml-auto hidden font-mono text-[11px] uppercase tracking-[0.25em] text-dim sm:block">{{ t("trackInfo.eyebrow") }}</span>
      </header>

      <div class="min-h-0 flex-1 overflow-y-auto">
        <div class="mx-auto w-full max-w-4xl px-6 py-8 lg:px-8">
          <header class="flex flex-col gap-5 border-b border-line pb-6 sm:flex-row sm:items-start sm:gap-6">
            <span class="ak-frame grid h-28 w-28 shrink-0 place-items-center overflow-hidden border border-line text-4xl font-black text-white/90" :style="{ backgroundColor: track?.color ?? 'var(--color-accent)' }">
              <img v-if="coverUrl" :src="coverUrl" alt="" decoding="async" class="h-full w-full object-cover" />
              <template v-else>{{ track ? initial(track) : "?" }}</template>
            </span>
            <div class="min-w-0 flex-1">
              <h1 class="truncate text-3xl font-black leading-tight tracking-tight sm:text-4xl">{{ track?.title ?? t("trackInfo.title") }}</h1>
              <p class="mt-2 truncate text-sm text-muted">{{ track?.artist }}<span v-if="track?.album"> · {{ track.album }}</span></p>
              <div class="mt-4 flex flex-wrap items-center gap-2">
                <span class="border border-line px-2 py-1 font-mono text-[10px] uppercase tracking-[0.2em] text-dim">{{ duration }}</span>
                <span v-if="favorite" class="border border-accent px-2 py-1 font-mono text-[10px] uppercase tracking-[0.2em] text-accent">{{ t("trackInfo.favorite") }}</span>
                <span v-if="playCount > 0" class="border border-line px-2 py-1 font-mono text-[10px] uppercase tracking-[0.2em] text-dim">{{ t("trackInfo.playCountBadge", { count: playCount }) }}</span>
              </div>
            </div>
          </header>

          <p v-if="loadError" class="mt-6 border-l-2 border-accent/60 pl-3 text-xs text-muted">{{ t("trackInfo.loadFailed", { value: loadError }) }}</p>

          <section class="ak-frame mt-6 border border-line bg-surface p-6">
            <h2 class="flex flex-wrap items-center gap-3 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">
              {{ t("trackInfo.tags") }}
              <span v-if="loading" class="flex items-center gap-2 font-mono text-[10px] tracking-[0.2em] text-accent"><span class="ak-pulse" style="width: 12px; height: 12px"></span>{{ t("trackInfo.loading") }}</span>
            </h2>
            <dl class="mt-5 grid gap-x-8 gap-y-4 sm:grid-cols-2">
              <div v-for="row in tagRows" :key="row.label" class="grid gap-1" :class="row.wide ? 'sm:col-span-2' : ''">
                <dt class="text-[10px] font-semibold uppercase tracking-[0.2em] text-dim">{{ row.label }}</dt>
                <dd class="text-sm text-fg" :class="row.wrap ? 'whitespace-pre-wrap break-words' : 'truncate'" :title="row.value">{{ row.value }}</dd>
              </div>
            </dl>
          </section>

          <section class="ak-frame mt-4 border border-line bg-surface p-6">
            <h2 class="text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("trackInfo.file") }}</h2>
            <dl class="mt-5 grid gap-x-8 gap-y-4 sm:grid-cols-2">
              <div v-for="row in fileRows" :key="row.label" class="grid gap-1" :class="row.wide ? 'sm:col-span-2' : ''">
                <dt class="text-[10px] font-semibold uppercase tracking-[0.2em] text-dim">{{ row.label }}</dt>
                <dd class="text-sm text-fg" :class="row.wrap ? 'whitespace-pre-wrap break-all' : 'truncate'" :title="row.value">{{ row.value }}</dd>
              </div>
            </dl>
          </section>
        </div>
      </div>
    </div>
  </div>
</template>
