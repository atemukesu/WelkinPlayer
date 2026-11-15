<!--
 Copyright 2026 Atemukesu
 SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { computed, watch } from "vue";
import { useI18n } from "vue-i18n";
import { AudioLines, LoaderCircle, Sparkles, RotateCcw, Trash2, X } from "@lucide/vue";
import { useProfileStore, tracksForPaths } from "../stores/profile";
import { usePlayerStore } from "../stores/player";
import { useSmartTransitionStore, type TrackState } from "../stores/smartTransition";
import { useTrackSort } from "../lib/sort";
import { trackKey } from "../lib/sources";
import { pushToast } from "../lib/toast";

const props = defineProps<{ playlistId: string; open: boolean }>();
const emit = defineEmits<{ close: [] }>();
const { t } = useI18n();
const profile = useProfileStore();
const player = usePlayerStore();
const smart = useSmartTransitionStore();
const playlistSort = useTrackSort("playlist");

const playlist = computed(() => profile.playlists.find((item) => item.id === props.playlistId) ?? null);
const summary = computed(() => smart.summaryFor(props.playlistId));
const progress = computed(() => smart.progressFor(props.playlistId));
const busy = computed(() => smart.isBusy(props.playlistId));
const cancelling = computed(() => smart.isCancelling(props.playlistId));
const incomplete = computed(() => summary.value?.incomplete === true);
const error = computed(() => smart.errorFor(props.playlistId));

/** Summary metadata keyed by playback key. */
const summaryTracks = computed(() => {
  const map = new Map<string, { status: string; bpm: number; keyCamelot: string; title: string }>();
  for (const track of summary.value?.tracks ?? []) {
    map.set(track.key, track);
  }
  return map;
});

/** Every playlist entry with its live analysis state. */
const trackRows = computed(() => {
  const current = playlist.value;
  if (!current) return [];
  const resolved = tracksForPaths(current.tracks, player.tracks);
  const states = smart.trackStatesFor(props.playlistId);
  return current.tracks.map((entry, index) => {
    const track = resolved[index];
    const key = trackKey(track) ?? entry;
    const meta = summaryTracks.value.get(key) ?? summaryTracks.value.get(entry);
    const state: TrackState =
      states[key] ??
      states[entry] ??
      (meta ? (meta.status === "skipped" ? "skipped" : "done") : "pending");
    return {
      key,
      title: track?.title ?? meta?.title ?? key.split("::").pop() ?? key,
      state,
      bpm: meta?.bpm ?? 0,
      keyCamelot: meta?.keyCamelot ?? "",
      // Highlight every active download instead of following the last event.
      current: state === "downloading",
    };
  });
});

/** How many tracks have finished downloading their head/tail windows. */
const downloadedCount = computed(
  () =>
    trackRows.value.filter(
      (row) => row.state !== "pending" && row.state !== "downloading",
    ).length,
);

const downloads = computed(() => smart.downloadsFor(props.playlistId));

const activeDownloads = computed(() =>
  Object.values(downloads.value).filter(
    (item) => item.total === null || item.total <= 0 || item.done < item.total,
  ),
);

const activeDownloadCount = computed(() => activeDownloads.value.length);

const activeDownloadBytes = computed(() =>
  activeDownloads.value.reduce((sum, item) => sum + item.done, 0),
);

const activeDownloadTotal = computed<number | null>(() => {
  if (activeDownloads.value.some((item) => !item.total || item.total <= 0)) return null;
  return activeDownloads.value.reduce((sum, item) => sum + (item.total ?? 0), 0);
});

const activeDownloadFraction = computed(() => {
  let sum = 0;
  for (const item of activeDownloads.value) {
    if (item.total && item.total > 0) {
      sum += Math.min(1, item.done / item.total);
    }
  }
  return sum;
});

const downloadPercent = computed(() => {
  const total = trackRows.value.length;
  if (total === 0) return 0;
  // Finished tracks plus every active download's fraction, so the bar moves
  // smoothly without following whichever track emitted an event last.
  const remaining = Math.max(0, total - downloadedCount.value);
  const partial = Math.min(activeDownloadFraction.value, remaining);
  return Math.min(100, Math.round(((downloadedCount.value + partial) / total) * 100));
});

/**
 * Counts are derived from the per-track states, never from the last progress
 * event: concurrent workers each report their own track index, so `done` on a
 * stage event is not a global completed count.
 */
const analyzeDone = computed(
  () =>
    trackRows.value.filter(
      (row) => row.state === "done" || row.state === "skipped" || row.state === "cached",
    ).length,
);
const analyzeTotal = computed(() => trackRows.value.length || progress.value?.total || 0);

const percent = computed(() => {
  if (analyzeTotal.value === 0) return 0;
  return Math.round((analyzeDone.value / analyzeTotal.value) * 100);
});

/** The three pipeline steps and which one is active right now. */
const stepStates = computed<Array<"done" | "active" | "pending">>(() => {
  if (!busy.value) {
    if (summary.value && !summary.value.incomplete) return ["done", "done", "done"];
    return ["pending", "pending", "pending"];
  }
  if (progress.value?.stage === "planning") return ["done", "done", "active"];

  const total = trackRows.value.length;
  const downloaded = downloadedCount.value;
  const analyzing = trackRows.value.some((row) => row.state === "analyzing");
  const processed = analyzeDone.value;

  const downloadStep: "done" | "active" = downloaded >= total ? "done" : "active";
  let analyzeStep: "done" | "active" | "pending" = "pending";
  if (analyzing || downloaded >= total) {
    analyzeStep = processed >= total ? "done" : "active";
  }
  return [downloadStep, analyzeStep, "pending"];
});

const stepDefs = computed(() => [
  { key: "download", label: t("library.playlists.smart.stepDownload"), hint: t("library.playlists.smart.stepDownloadHint") },
  { key: "analyze", label: t("library.playlists.smart.stepAnalyze"), hint: t("library.playlists.smart.stepAnalyzeHint") },
  { key: "plan", label: t("library.playlists.smart.stepPlan"), hint: t("library.playlists.smart.stepPlanHint") },
]);

/** Stable summary label; never follows the last track that emitted an event. */
const currentStageLabel = computed(() => {
  if (activeDownloadCount.value > 0) return t("library.playlists.smart.statusDownloading");
  if (trackRows.value.some((row) => row.state === "analyzing")) {
    return t("library.playlists.smart.statusAnalyzing");
  }
  if (trackRows.value.some((row) => row.state === "cached")) {
    return t("library.playlists.smart.statusCached");
  }
  if (progress.value?.stage === "planning") return t("library.playlists.smart.stepPlan");
  if (busy.value) return t("library.playlists.smart.statusPending");
  return "";
});

/** Percentage for one row's own download, so rows do not need the latest event. */
function downloadPercentFor(key: string): number {
  const state = downloads.value[key];
  if (!state || !state.total || state.total <= 0) return 0;
  return Math.min(100, Math.round((state.done / state.total) * 100));
}

function stateLabel(state: TrackState): string {
  switch (state) {
    case "downloading":
      return t("library.playlists.smart.statusDownloading");
    case "analyzing":
      return t("library.playlists.smart.statusAnalyzing");
    case "cached":
      return t("library.playlists.smart.statusCached");
    case "done":
      return t("library.playlists.smart.statusDone");
    case "skipped":
      return t("library.playlists.smart.statusSkipped");
    default:
      return t("library.playlists.smart.statusPending");
  }
}

function stateClass(state: TrackState): string {
  switch (state) {
    case "downloading":
      return "border-sky-500/40 text-sky-400";
    case "analyzing":
      return "border-accent/50 text-accent";
    case "cached":
      return "border-accent/30 text-accent";
    case "done":
      return "border-emerald-500/40 text-emerald-400";
    case "skipped":
      return "border-amber-500/40 text-amber-400";
    default:
      return "border-line text-dim";
  }
}

/** Titles for the preview order, resolved from the current library. */
const orderedRows = computed(() => {
  const value = summary.value;
  const current = playlist.value;
  if (!value || !current) return [];
  const resolved = tracksForPaths(current.tracks, player.tracks);
  const byKey = new Map<string, string>();
  for (const track of resolved) {
    const key = trackKey(track);
    if (key) byKey.set(key, track.title);
  }
  return value.order.map((key, index) => ({
    index: index + 1,
    key,
    title: byKey.get(key) ?? key.split("::").pop() ?? key,
  }));
});

const transitionRows = computed(() => summary.value?.transitions.slice(0, 10) ?? []);

/** Format a byte count for the live download readout. */
function formatBytes(value: number | null | undefined): string {
  if (!value || value <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  let size = value;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }
  return `${size.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}

/** Format an absolute second as `m:ss.d` for the cue-point readout. */
function cuePoint(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return "0:00";
  const whole = Math.floor(seconds);
  const tenth = Math.floor((seconds - whole) * 10);
  return `${Math.floor(whole / 60)}:${String(whole % 60).padStart(2, "0")}.${tenth}`;
}

function start() {
  void smart.analyze(props.playlistId);
}

function cancelRun() {
  void smart.cancel(props.playlistId);
}

function apply() {
  if (!smart.applyOrder(props.playlistId)) return;
  // The smart order is now the playlist order; show it instead of whatever
  // manual sort the user had selected.
  playlistSort.key.value = "default";
  pushToast("success", t("library.playlists.smart.applied"));
  emit("close");
}

/** Whether the live queue is exactly this playlist's tracks. */
function isCurrentQueue(): boolean {
  const current = playlist.value;
  if (!current) return false;
  const playlistKeys = new Set(
    tracksForPaths(current.tracks, player.tracks).map(trackKey).filter((key): key is string => !!key),
  );
  const playing = trackKey(player.currentTrack);
  if (!playing || !playlistKeys.has(playing)) return false;
  const queueKeys = player.queue.map(trackKey).filter((key): key is string => !!key);
  return queueKeys.length > 0 && queueKeys.every((key) => playlistKeys.has(key));
}

function undo() {
  if (!smart.undoOrder(props.playlistId)) return;
  // Restoring the old order invalidates the plans/order lock of this queue.
  if (isCurrentQueue()) void smart.clearPlans();
  pushToast("info", t("library.playlists.smart.undone"));
}

function remove() {
  void smart.remove(props.playlistId).then(() => {
    if (isCurrentQueue()) void smart.clearPlans();
    pushToast("info", t("library.playlists.smart.removed"));
    emit("close");
  });
}

function close() {
  emit("close");
}

watch(
  () => props.open,
  (open) => {
    if (!open) return;
    smart.clearError(props.playlistId);
    void smart.ensureListener();
    void smart.load(props.playlistId);
  },
  { immediate: true },
);
</script>

<template>
  <Teleport to="body">
    <div v-if="open" class="fixed inset-0 z-[80] grid place-items-center bg-black/70 p-4 backdrop-blur-sm" @click.self="close">
      <div class="ak-frame flex max-h-[88vh] w-full max-w-2xl flex-col overflow-hidden border border-line bg-bg shadow-2xl">
        <header class="flex items-start gap-3 border-b border-line px-5 py-4">
          <span class="mt-0.5 grid h-9 w-9 shrink-0 place-items-center bg-accent text-accent-fg"><AudioLines :size="18" /></span>
          <div class="min-w-0 flex-1">
            <p class="text-[11px] font-semibold uppercase tracking-[0.3em] text-accent">{{ t("library.playlists.smart.eyebrow") }}</p>
            <h2 class="mt-1 truncate text-lg font-black">{{ t("library.playlists.smart.title") }}</h2>
            <p class="mt-1 text-xs text-dim">{{ t("library.playlists.smart.desc") }}</p>
          </div>
          <button type="button" class="grid h-8 w-8 shrink-0 place-items-center border border-line text-dim transition-colors hover:border-accent hover:text-accent" :title="t('library.playlists.smart.close')" @click="close"><X :size="16" /></button>
        </header>

        <div class="min-h-0 flex-1 overflow-y-auto px-5 py-4">
          <div v-if="busy" class="space-y-4">
            <div class="grid grid-cols-1 gap-2 sm:grid-cols-3">
              <div
                v-for="(step, index) in stepDefs"
                :key="step.key"
                class="border px-3 py-2 transition-colors"
                :class="stepStates[index] === 'active' ? 'border-accent bg-accent/10' : stepStates[index] === 'done' ? 'border-emerald-500/40 bg-emerald-500/5' : 'border-line'"
              >
                <div class="flex items-center gap-2">
                  <span
                    class="grid h-5 w-5 shrink-0 place-items-center rounded-full border font-mono text-[10px]"
                    :class="stepStates[index] === 'active' ? 'border-accent text-accent' : stepStates[index] === 'done' ? 'border-emerald-500/60 text-emerald-400' : 'border-line text-dim'"
                  >{{ stepStates[index] === "done" ? "✓" : index + 1 }}</span>
                  <span class="text-xs font-semibold" :class="stepStates[index] === 'active' ? 'text-accent' : stepStates[index] === 'done' ? 'text-emerald-400' : 'text-muted'">{{ step.label }}</span>
                </div>
                <p class="mt-1 text-[11px] leading-snug text-dim">{{ step.hint }}</p>
              </div>
            </div>

            <div class="space-y-1.5">
              <div class="flex items-center justify-between text-xs">
                <span class="text-muted">{{ t("library.playlists.smart.downloadLabel") }}</span>
                <span class="font-mono text-dim">{{ downloadedCount }}/{{ trackRows.length }} · {{ downloadPercent }}%</span>
              </div>
              <div class="h-1.5 w-full overflow-hidden bg-surface">
                <div class="h-full bg-sky-500 transition-[width] duration-300" :style="{ width: `${downloadPercent}%` }" />
              </div>
            </div>

            <div class="space-y-1.5">
              <div class="flex items-center justify-between text-xs">
                <span class="text-muted">{{ t("library.playlists.smart.analyzeLabel") }}</span>
                <span class="font-mono text-dim">{{ analyzeDone }}/{{ analyzeTotal }} · {{ percent }}%</span>
              </div>
              <div class="h-1.5 w-full overflow-hidden bg-surface">
                <div class="h-full bg-accent transition-[width] duration-300" :style="{ width: `${percent}%` }" />
              </div>
            </div>

            <div class="flex items-center gap-2 border border-accent/30 bg-accent/5 px-3 py-2 text-xs">
              <LoaderCircle :size="14" class="shrink-0 animate-spin text-accent" />
              <span class="min-w-0 flex-1 truncate">
                <span class="font-semibold text-accent">{{ currentStageLabel }}</span>
                <span v-if="activeDownloadCount > 0" class="text-muted"> · {{ t("library.playlists.count", { count: activeDownloadCount }) }}</span>
              </span>
              <span v-if="activeDownloadCount > 0" class="shrink-0 font-mono text-dim">
                {{ formatBytes(activeDownloadBytes) }}<span v-if="activeDownloadTotal"> / {{ formatBytes(activeDownloadTotal) }}</span>
              </span>
              <span v-else class="shrink-0 font-mono text-dim">{{ analyzeDone }}/{{ analyzeTotal }}</span>
            </div>
            <p class="text-xs text-dim">{{ t("library.playlists.smart.analyzingHint") }}</p>
            <p class="border border-accent/30 bg-accent/5 px-3 py-2 text-xs text-accent">{{ t("library.playlists.smart.backgroundHint") }}</p>
          </div>

          <div v-if="error && !busy" class="border border-red-500/40 bg-red-500/10 px-3 py-2 text-sm text-red-400">
            {{ error === "no-playable-tracks" ? t("library.playlists.smart.noTracks") : error }}
          </div>

          <template v-if="summary">
            <div v-if="incomplete && !busy" class="mb-3 border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-400">
              {{ t("library.playlists.smart.partial", { analyzed: summary.stats.analyzed, total: progress?.total ?? (summary.previousOrder.length || summary.order.length) }) }}
            </div>
            <div class="flex flex-wrap items-center gap-2 text-xs text-dim">
              <span class="border border-line px-2 py-1 font-mono uppercase tracking-wider">{{ t("library.playlists.smart.stats", { analyzed: summary.stats.analyzed, transitions: summary.transitions.length }) }}</span>
              <span v-if="summary.stats.reused" class="border border-line px-2 py-1 font-mono uppercase tracking-wider">{{ t("library.playlists.smart.reused", { count: summary.stats.reused }) }}</span>
              <span v-if="summary.stats.skipped" class="border border-line px-2 py-1 font-mono uppercase tracking-wider">{{ t("library.playlists.smart.skipped", { count: summary.stats.skipped }) }}</span>
              <span v-if="summary.source === 'local'" class="border border-amber-500/40 px-2 py-1 font-mono uppercase tracking-wider text-amber-400">{{ t("library.playlists.smart.localOnly") }}</span>
            </div>
          </template>

          <section v-if="trackRows.length && (busy || summary)" class="mt-4">
            <div class="flex items-center justify-between gap-2">
              <h3 class="text-xs font-semibold uppercase tracking-[0.25em] text-dim">{{ t("library.playlists.smart.allTracksTitle") }}</h3>
              <span class="font-mono text-[11px] text-dim">{{ downloadedCount }}/{{ trackRows.length }} · {{ analyzeDone }}/{{ analyzeTotal }}</span>
            </div>
            <ul class="mt-2 max-h-64 divide-y divide-line overflow-y-auto border border-line">
              <li
                v-for="(row, index) in trackRows"
                :key="`${row.key}-${index}`"
                class="flex items-center gap-2 px-3 py-1.5 text-sm"
                :class="row.current ? 'bg-accent/10' : ''"
              >
                <span class="w-6 shrink-0 text-right font-mono text-xs text-dim">{{ index + 1 }}</span>
                <span class="min-w-0 flex-1 truncate" :class="row.state === 'skipped' ? 'text-amber-400/90' : row.state === 'pending' ? 'text-dim' : ''">{{ row.title }}</span>
                <span v-if="row.bpm > 0" class="hidden shrink-0 font-mono text-[10px] text-dim sm:inline">{{ row.bpm.toFixed(0) }} BPM<span v-if="row.keyCamelot"> · {{ row.keyCamelot }}</span></span>
                <span v-if="row.state === 'downloading' && downloadPercentFor(row.key) > 0" class="shrink-0 font-mono text-[10px] text-sky-400">{{ downloadPercentFor(row.key) }}%</span>
                <span class="shrink-0 border px-1.5 py-0.5 font-mono text-[10px] uppercase" :class="stateClass(row.state)">{{ stateLabel(row.state) }}</span>
              </li>
            </ul>
          </section>

          <template v-if="summary && !busy">
            <section class="mt-4">
              <h3 class="text-xs font-semibold uppercase tracking-[0.25em] text-dim">{{ t("library.playlists.smart.orderTitle") }}</h3>
              <ol class="mt-2 max-h-64 divide-y divide-line overflow-y-auto border border-line">
                <li v-for="row in orderedRows" :key="`${row.key}-${row.index}`" class="flex items-center gap-3 px-3 py-1.5 text-sm">
                  <span class="w-6 shrink-0 text-right font-mono text-xs text-dim">{{ row.index }}</span>
                  <span class="min-w-0 flex-1 truncate">{{ row.title }}</span>
                </li>
              </ol>
            </section>

            <section v-if="transitionRows.length" class="mt-4">
              <h3 class="text-xs font-semibold uppercase tracking-[0.25em] text-dim">{{ t("library.playlists.smart.transitionsTitle") }}</h3>
              <ul class="mt-2 space-y-1.5">
                <li v-for="(plan, index) in transitionRows" :key="`${plan.from}-${plan.to}-${index}`" class="flex items-center gap-2 border border-line px-3 py-2 text-xs">
                  <span class="min-w-0 flex-1 truncate">{{ plan.fromTitle }} <span class="text-dim">→</span> {{ plan.toTitle }}</span>
                  <span class="shrink-0 font-mono text-[11px] text-dim">{{ cuePoint(plan.outAt) }} → {{ cuePoint(plan.inAt) }}</span>
                  <span class="shrink-0 font-mono text-[11px] text-dim">{{ plan.blendSecs.toFixed(1) }}s</span>
                  <span class="shrink-0 border px-1.5 py-0.5 font-mono text-[10px] uppercase" :class="plan.mode === 'fallback' ? 'border-amber-500/40 text-amber-400' : 'border-accent/40 text-accent'">{{ plan.mode }}</span>
                </li>
              </ul>
            </section>
          </template>

          <div v-if="!busy && !error && !summary" class="grid place-items-center gap-3 border border-dashed border-line px-4 py-10 text-center">
            <Sparkles :size="22" class="text-accent" />
            <p class="max-w-sm text-sm text-muted">{{ t("library.playlists.smart.emptyHint") }}</p>
          </div>
        </div>

        <footer class="flex flex-wrap items-center gap-2 border-t border-line px-5 py-3">
          <button v-if="busy" type="button" :disabled="cancelling" class="ak-clip-tr flex h-9 items-center gap-2 border border-line px-3 text-xs font-semibold transition-colors hover:border-accent hover:text-accent disabled:cursor-wait disabled:opacity-60" @click="cancelRun">
            <LoaderCircle v-if="cancelling" :size="13" class="animate-spin" />
            {{ cancelling ? t("library.playlists.smart.cancelling") : t("library.playlists.smart.cancel") }}
          </button>
          <template v-else>
            <button type="button" class="ak-clip-tr flex h-9 items-center gap-2 bg-accent px-4 text-xs font-bold text-accent-fg transition-transform hover:scale-[1.02] active:scale-95" @click="start">
              <Sparkles :size="14" />{{ incomplete ? t("library.playlists.smart.resume") : summary ? t("library.playlists.smart.reanalyze") : t("library.playlists.smart.start") }}
            </button>
            <button v-if="summary && !incomplete" type="button" class="ak-clip-tr flex h-9 items-center gap-2 border border-line px-3 text-xs font-semibold transition-colors hover:border-accent hover:text-accent" @click="apply">{{ t("library.playlists.smart.apply") }}</button>
            <button v-if="summary && !incomplete && summary.previousOrder.length" type="button" class="ak-clip-tr flex h-9 items-center gap-2 border border-line px-3 text-xs font-semibold transition-colors hover:border-accent hover:text-accent" @click="undo"><RotateCcw :size="14" />{{ t("library.playlists.smart.undo") }}</button>
            <button v-if="summary" type="button" class="ml-auto grid h-9 w-9 place-items-center border border-line text-dim transition-colors hover:border-red-500/60 hover:text-red-400" :title="t('library.playlists.smart.remove')" @click="remove"><Trash2 :size="14" /></button>
          </template>
        </footer>
      </div>
    </div>
  </Teleport>
</template>
