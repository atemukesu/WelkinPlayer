<!--
 Copyright 2026 Atemukesu
 SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { ArrowLeft, ImagePlus, RotateCcw, Save, Trash2 } from "@lucide/vue";
import { describeError, invoke } from "../api";
import { pushToast } from "../lib/toast";
import { initial } from "../lib/format";
import { usePlayerStore } from "../stores/player";
import type { Track } from "../stores/player";
import { useSourcesStore } from "../stores/sources";
import { trackKey } from "../lib/sources";
import type { TrackMetadata, TrackTags } from "../lib/remote";
import CoverCropper from "../components/CoverCropper.vue";

const props = defineProps<{ path: string | null; sourceId?: string | null }>();
const emit = defineEmits<{ back: []; friendlyError: [error: unknown] }>();
const { t } = useI18n();
const player = usePlayerStore();
const sources = useSourcesStore();
const key = computed(() => trackKey({ sourceId: props.sourceId ?? undefined, path: props.path ?? undefined }));

interface Form {
  title: string; artist: string; album: string; albumArtist: string; genre: string; comment: string;
  year: string; trackNumber: string; trackTotal: string; discNumber: string; discTotal: string;
}

function emptyForm(): Form {
  return { title: "", artist: "", album: "", albumArtist: "", genre: "", comment: "", year: "", trackNumber: "", trackTotal: "", discNumber: "", discTotal: "" };
}

const form = ref<Form>(emptyForm());
const savedForm = ref<Form>(emptyForm());
const loading = ref(false);
const saving = ref(false);
const loadError = ref("");
const hasCover = ref(false);
const coverPreview = ref<string | null>(null);
const pendingCover = ref<string | null>(null);
const removeCover = ref(false);
const savedRemoveCover = ref(false);
const cropSrc = ref<string | null>(null);

const track = computed<Track | null>(() => player.tracks.find((item) => trackKey(item) === key.value) ?? null);
const previewSrc = computed(() => pendingCover.value ?? (removeCover.value ? null : coverPreview.value));
const dirty = computed(() =>
  JSON.stringify(form.value) !== JSON.stringify(savedForm.value)
  || pendingCover.value !== null
  || removeCover.value !== savedRemoveCover.value,
);

function toForm(tags: TrackTags): Form {
  const text = (value: string | null) => value ?? "";
  const number = (value: number | null) => (value === null ? "" : String(value));
  return {
    title: text(tags.title), artist: text(tags.artist), album: text(tags.album),
    albumArtist: text(tags.albumArtist), genre: text(tags.genre), comment: text(tags.comment),
    year: number(tags.year), trackNumber: number(tags.trackNumber), trackTotal: number(tags.trackTotal),
    discNumber: number(tags.discNumber), discTotal: number(tags.discTotal),
  };
}
function emptyToNull(value: string): string | null {
  const trimmed = value.trim();
  return trimmed ? trimmed : null;
}
function numberOrNull(value: string): number | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  const parsed = Number.parseInt(trimmed, 10);
  return Number.isFinite(parsed) && parsed >= 0 ? parsed : null;
}
function formatSeconds(seconds: number): string {
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

async function load() {
  form.value = emptyForm();
  savedForm.value = emptyForm();
  loadError.value = "";
  hasCover.value = false;
  coverPreview.value = null;
  pendingCover.value = null;
  removeCover.value = false;
  savedRemoveCover.value = false;
  if (!props.path) return;

  loading.value = true;
  try {
    const tags = await invoke<TrackTags>("read_track_tags", { sourceId: props.sourceId, path: props.path });
    form.value = toForm(tags);
    savedForm.value = { ...form.value };
    hasCover.value = tags.hasCover;
    // Prefer the already-hydrated library cover, then fall back to the local cache.
    coverPreview.value = track.value?.cover ?? null;
    if (!coverPreview.value) {
      coverPreview.value = await invoke<string | null>("get_cached_cover", { sourceId: props.sourceId, path: props.path }).catch(() => null);
    }
  } catch (error) {
    loadError.value = describeError(error);
  } finally {
    loading.value = false;
  }
}

function onFile(event: Event) {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0];
  if (!file) return;
  const reader = new FileReader();
  reader.onload = () => { cropSrc.value = String(reader.result); };
  reader.readAsDataURL(file);
  input.value = "";
}
function applyCrop(value: string) { pendingCover.value = value; removeCover.value = false; cropSrc.value = null; }
function markRemoveCover() { pendingCover.value = null; removeCover.value = true; }

function reset() {
  form.value = { ...savedForm.value };
  pendingCover.value = null;
  removeCover.value = savedRemoveCover.value;
}

async function save() {
  if (!props.path || saving.value) return;
  saving.value = true;
  try {
    const tags: TrackTags = {
      title: emptyToNull(form.value.title),
      artist: emptyToNull(form.value.artist),
      album: emptyToNull(form.value.album),
      albumArtist: emptyToNull(form.value.albumArtist),
      genre: emptyToNull(form.value.genre),
      comment: emptyToNull(form.value.comment),
      year: numberOrNull(form.value.year),
      trackNumber: numberOrNull(form.value.trackNumber),
      trackTotal: numberOrNull(form.value.trackTotal),
      discNumber: numberOrNull(form.value.discNumber),
      discTotal: numberOrNull(form.value.discTotal),
      hasCover: hasCover.value,
    };
    const metadata = await invoke<TrackMetadata>("edit_track_metadata", {
      sourceId: props.sourceId,
      path: props.path,
      tags,
      coverDataUrl: pendingCover.value,
      removeCover: removeCover.value,
    });

    // Reflect the saved tags immediately so the library updates without a re-list.
    const target = track.value;
    if (target) {
      const patch: Partial<Track> = { metaLoaded: true, assetsHydrated: true };
      patch.title = metadata.title ?? target.title;
      patch.artist = metadata.artist ?? target.artist;
      patch.album = metadata.album ?? target.album;
      if (metadata.durationSecs) patch.duration = formatSeconds(metadata.durationSecs);
      if (pendingCover.value) patch.cover = pendingCover.value;
      else if (removeCover.value) patch.cover = undefined;
      player.updateTrack(target.id, patch);
    }

    if (pendingCover.value) { coverPreview.value = pendingCover.value; hasCover.value = true; }
    if (removeCover.value) { coverPreview.value = null; hasCover.value = false; }
    savedForm.value = { ...form.value };
    savedRemoveCover.value = removeCover.value;
    pendingCover.value = null;
    loadError.value = "";
    pushToast("success", t("trackEditor.saved"));
  } catch (error) {
    emit("friendlyError", error);
  } finally {
    saving.value = false;
  }
}

watch(() => props.path, load, { immediate: true });
</script>

<template>
  <div class="mx-auto w-full max-w-4xl px-6 py-6 lg:px-8 lg:py-8">
    <button type="button" class="flex items-center gap-2 text-[12px] font-semibold uppercase tracking-[0.2em] text-dim transition-colors hover:text-fg" @click="emit('back')">
      <ArrowLeft :size="15" />{{ t("trackEditor.back") }}
    </button>

    <header class="mt-6 flex items-start gap-5 border-b border-line pb-6">
      <span class="ak-frame grid h-20 w-20 shrink-0 place-items-center overflow-hidden border border-line text-2xl font-black text-white/90" :style="{ backgroundColor: track?.color ?? 'var(--color-accent)' }">
        <img v-if="previewSrc" :src="previewSrc" alt="" decoding="async" class="h-full w-full object-cover" />
        <template v-else>{{ track ? initial(track) : "?" }}</template>
      </span>
      <div class="min-w-0">
        <p class="flex items-center gap-2 text-[13px] font-semibold uppercase tracking-[0.35em] text-accent"><span class="h-2 w-2 bg-accent"></span>{{ t("trackEditor.eyebrow") }}</p>
        <h1 class="mt-3 truncate text-3xl font-black leading-tight tracking-tight sm:text-4xl">{{ track?.title ?? t("trackEditor.title") }}</h1>
        <p class="mt-2 truncate text-sm text-muted">{{ track?.artist }}<span v-if="track?.album"> · {{ track.album }}</span></p>
      </div>
    </header>

    <div v-if="loading" class="mt-8 flex items-center gap-2 font-mono text-[12px] uppercase tracking-[0.2em] text-accent"><span class="ak-pulse" style="width: 12px; height: 12px"></span>{{ t("trackEditor.loading") }}</div>

    <template v-else>
      <p v-if="loadError" class="mt-6 border-l-2 border-accent/60 pl-3 text-xs text-muted">{{ t("trackEditor.loadFailed", { value: loadError }) }}</p>
      <p v-if="!sources.hasSources" class="mt-6 border-l-2 border-accent/60 pl-3 text-xs text-muted">{{ t("trackEditor.notConfigured") }}</p>

      <div class="mt-6 grid gap-4 lg:grid-cols-[minmax(0,1fr)_260px]">
        <section class="ak-frame border border-line bg-surface p-6">
          <h2 class="flex items-center gap-2 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("trackEditor.fields") }}</h2>
          <p class="mt-3 text-xs text-muted">{{ t("trackEditor.fieldsHint") }}</p>
          <div class="mt-5 grid gap-4 sm:grid-cols-2">
            <label class="grid gap-2 text-[13px] font-semibold text-dim sm:col-span-2">{{ t("trackEditor.titleLabel") }}<input v-model="form.title" class="h-10 border border-line bg-bg px-3 text-sm font-normal text-fg outline-none focus:border-accent" /></label>
            <label class="grid gap-2 text-[13px] font-semibold text-dim">{{ t("trackEditor.artistLabel") }}<input v-model="form.artist" class="h-10 border border-line bg-bg px-3 text-sm font-normal text-fg outline-none focus:border-accent" /></label>
            <label class="grid gap-2 text-[13px] font-semibold text-dim">{{ t("trackEditor.albumLabel") }}<input v-model="form.album" class="h-10 border border-line bg-bg px-3 text-sm font-normal text-fg outline-none focus:border-accent" /></label>
            <label class="grid gap-2 text-[13px] font-semibold text-dim">{{ t("trackEditor.albumArtistLabel") }}<input v-model="form.albumArtist" class="h-10 border border-line bg-bg px-3 text-sm font-normal text-fg outline-none focus:border-accent" /></label>
            <label class="grid gap-2 text-[13px] font-semibold text-dim">{{ t("trackEditor.genreLabel") }}<input v-model="form.genre" class="h-10 border border-line bg-bg px-3 text-sm font-normal text-fg outline-none focus:border-accent" /></label>
            <label class="grid gap-2 text-[13px] font-semibold text-dim">{{ t("trackEditor.yearLabel") }}<input v-model="form.year" type="number" min="0" inputmode="numeric" class="h-10 border border-line bg-bg px-3 text-sm font-normal text-fg outline-none focus:border-accent" /></label>
            <label class="grid gap-2 text-[13px] font-semibold text-dim">{{ t("trackEditor.trackNumberLabel") }}<input v-model="form.trackNumber" type="number" min="0" inputmode="numeric" class="h-10 border border-line bg-bg px-3 text-sm font-normal text-fg outline-none focus:border-accent" /></label>
            <label class="grid gap-2 text-[13px] font-semibold text-dim">{{ t("trackEditor.trackTotalLabel") }}<input v-model="form.trackTotal" type="number" min="0" inputmode="numeric" class="h-10 border border-line bg-bg px-3 text-sm font-normal text-fg outline-none focus:border-accent" /></label>
            <label class="grid gap-2 text-[13px] font-semibold text-dim">{{ t("trackEditor.discNumberLabel") }}<input v-model="form.discNumber" type="number" min="0" inputmode="numeric" class="h-10 border border-line bg-bg px-3 text-sm font-normal text-fg outline-none focus:border-accent" /></label>
            <label class="grid gap-2 text-[13px] font-semibold text-dim">{{ t("trackEditor.discTotalLabel") }}<input v-model="form.discTotal" type="number" min="0" inputmode="numeric" class="h-10 border border-line bg-bg px-3 text-sm font-normal text-fg outline-none focus:border-accent" /></label>
            <label class="grid gap-2 text-[13px] font-semibold text-dim sm:col-span-2">{{ t("trackEditor.commentLabel") }}<textarea v-model="form.comment" rows="3" class="resize-none border border-line bg-bg p-3 text-sm font-normal text-fg outline-none focus:border-accent"></textarea></label>
          </div>
        </section>

        <section class="ak-frame border border-line bg-surface p-6">
          <h2 class="flex items-center gap-2 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("trackEditor.cover") }}</h2>
          <div class="mt-4 grid place-items-center">
            <span class="ak-frame grid h-40 w-40 place-items-center overflow-hidden border border-line text-4xl font-black text-white/90" :style="{ backgroundColor: track?.color ?? 'var(--color-accent)' }">
              <img v-if="previewSrc" :src="previewSrc" alt="" decoding="async" class="h-full w-full object-cover" />
              <template v-else>{{ track ? initial(track) : "?" }}</template>
            </span>
          </div>
          <p class="mt-3 text-xs text-muted">{{ t("trackEditor.coverHint") }}</p>
          <label class="ak-clip-tr mt-4 flex h-10 cursor-pointer items-center justify-center gap-2 border border-line px-4 text-[13px] font-semibold transition-colors hover:border-accent hover:text-accent"><ImagePlus :size="15" />{{ t("trackEditor.chooseCover") }}<input type="file" accept="image/*" class="hidden" @change="onFile" /></label>
          <button v-if="previewSrc && !removeCover" type="button" class="mt-3 flex w-full items-center justify-center gap-2 text-[12px] font-semibold text-dim transition-colors hover:text-red-500" @click="markRemoveCover"><Trash2 :size="14" />{{ t("trackEditor.removeCover") }}</button>
        </section>
      </div>

      <section class="ak-frame mt-4 border border-line bg-surface p-6">
        <h2 class="flex items-center gap-2 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("trackEditor.remote") }}</h2>
        <p class="mt-3 text-xs text-muted">{{ t("trackEditor.remoteHint") }}</p>
        <p class="mt-3 break-all font-mono text-xs">{{ props.path ?? "—" }}</p>
      </section>

      <footer class="mt-8 flex flex-wrap items-center gap-3 border-t border-line pt-6">
        <span v-if="dirty" class="flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-[0.2em] text-accent"><span class="h-1.5 w-1.5 bg-accent"></span>{{ t("trackEditor.dirty") }}</span>
        <div class="ml-auto flex items-center gap-3">
          <button type="button" class="ak-clip-tr flex h-11 items-center gap-2 border border-line px-4 text-[13px] font-semibold transition-colors hover:border-accent hover:text-accent disabled:opacity-50" :disabled="!dirty || saving" @click="reset"><RotateCcw :size="15" />{{ t("trackEditor.reset") }}</button>
          <button type="button" class="ak-clip-tr flex h-11 items-center gap-2 bg-accent px-6 text-[13px] font-bold text-accent-fg transition-transform hover:scale-[1.02] active:scale-95 disabled:opacity-50" :disabled="saving || !props.path || !!loadError || !sources.hasSources" @click="save"><Save :size="15" />{{ saving ? t("trackEditor.saving") : t("trackEditor.save") }}</button>
        </div>
      </footer>
    </template>

    <CoverCropper v-if="cropSrc" :src="cropSrc" @confirm="applyCrop" @cancel="cropSrc = null" />
  </div>
</template>
