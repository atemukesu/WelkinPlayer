<!--
 Copyright 2026 Atemukesu
 SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { computed, watch } from "vue";
import { useI18n } from "vue-i18n";
import { AudioLines, LoaderCircle, RefreshCw, Sparkles, Trash2, X } from "@lucide/vue";
import { useProfileStore, tracksForPaths } from "../stores/profile";
import { usePlayerStore } from "../stores/player";
import { useSmartTransitionStore, type TrackState } from "../stores/smartTransition";
import { trackKey } from "../lib/sources";
import { pushToast } from "../lib/toast";

const props = defineProps<{ playlistId: string; open: boolean }>();
const emit = defineEmits<{ close: [] }>();
const { t } = useI18n();
const profile = useProfileStore();
const player = usePlayerStore();
const smart = useSmartTransitionStore();

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

/** One bar for the whole run: download while tracks are still arriving, then
 * analysis progress. Avoids showing two competing progress bars. */
const overallPercent = computed(() =>
  activeDownloadCount.value > 0 || downloadedCount.value < trackRows.value.length
    ? downloadPercent.value
    : percent.value,
);

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
  { key: "download", label: t("library.playlists.smart.stepDownload") },
  { key: "analyze", label: t("library.playlists.smart.stepAnalyze") },
  { key: "plan", label: t("library.playlists.smart.stepPlan") },
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

/** A single status dot replaces the row badges: much quieter to scan. */
function dotClass(state: TrackState): string {
  switch (state) {
    case "downloading":
      return "bg-sky-400";
    case "analyzing":
      return "bg-accent animate-pulse";
    case "cached":
      return "bg-accent/50";
    case "done":
      return "bg-emerald-500";
    case "skipped":
      return "bg-amber-500";
    default:
      return "bg-dim/40";
  }
}

const stepDotClass = (state: "done" | "active" | "pending") =>
  state === "done" ? "bg-emerald-500" : state === "active" ? "bg-accent animate-pulse" : "bg-dim/40";

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

function start() {
  void smart.analyze(props.playlistId);
}

function cancelRun() {
  void smart.cancel(props.playlistId);
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
    <Transition name="modal">
      <div
        v-if="open"
        class="fixed inset-0 z-[80] grid place-items-center bg-black/70 p-4 backdrop-blur-sm"
        role="dialog"
        aria-modal="true"
        @click.self="close"
      >
        <div class="ak-frame flex max-h-[88vh] w-full max-w-xl flex-col overflow-hidden border border-line bg-bg shadow-2xl">
          <header class="flex items-center gap-3 border-b border-line px-5 py-4">
            <span class="grid h-10 w-10 shrink-0 place-items-center bg-accent/15 text-accent">
              <AudioLines :size="18" />
            </span>
            <div class="min-w-0 flex-1">
              <h2 class="truncate text-base font-black tracking-wide">
                {{ t("library.playlists.smart.title") }}<span class="ml-1 align-baseline text-[11px] font-normal tracking-normal text-dim">{{ t("library.playlists.smart.experimental") }}</span>
              </h2>
              <p class="mt-0.5 line-clamp-2 text-xs text-dim">{{ t("library.playlists.smart.desc") }}</p>
            </div>
            <button
              type="button"
              class="grid h-8 w-8 shrink-0 place-items-center text-dim transition-colors hover:text-fg"
              :title="t('library.playlists.smart.close')"
              :aria-label="t('library.playlists.smart.close')"
              @click="close"
            >
              <X :size="16" />
            </button>
          </header>

          <div class="min-h-0 flex-1 overflow-y-auto px-5 py-4">
            <div v-if="error && !busy" class="border border-red-500/40 bg-red-500/10 px-3 py-2 text-sm text-red-400">
              {{ error === "no-playable-tracks" ? t("library.playlists.smart.noTracks") : error }}
            </div>

            <!-- Running: one progress card, a quiet stepper and the live list. -->
            <section v-if="busy" class="space-y-4">
              <div class="border border-line bg-surface px-4 py-3">
                <div class="flex items-center gap-2">
                  <LoaderCircle :size="15" class="shrink-0 animate-spin text-accent" />
                  <span class="min-w-0 flex-1 truncate text-sm font-semibold">{{ currentStageLabel }}</span>
                  <span class="shrink-0 font-mono text-[11px] text-dim">
                    <template v-if="activeDownloadCount > 0">{{ downloadedCount }}/{{ trackRows.length }} · {{ downloadPercent }}%</template>
                    <template v-else>{{ analyzeDone }}/{{ analyzeTotal }} · {{ percent }}%</template>
                  </span>
                </div>
                <div class="mt-3 h-1.5 overflow-hidden bg-bg">
                  <div class="h-full bg-accent transition-[width] duration-300" :style="{ width: `${overallPercent}%` }" />
                </div>
                <div class="mt-2 flex items-center justify-between gap-3 text-[11px] text-dim">
                  <span class="flex min-w-0 items-center gap-1.5 overflow-hidden">
                    <template v-for="(step, index) in stepDefs" :key="step.key">
                      <span class="flex shrink-0 items-center gap-1.5" :class="stepStates[index] === 'active' ? 'text-accent' : stepStates[index] === 'done' ? 'text-emerald-400' : ''">
                        <span class="h-1.5 w-1.5 rounded-full" :class="stepDotClass(stepStates[index])" />
                        {{ step.label }}
                      </span>
                      <span v-if="index < stepDefs.length - 1" class="h-px w-3 shrink-0 bg-line" />
                    </template>
                  </span>
                  <span v-if="activeDownloadCount > 0" class="shrink-0 font-mono">
                    {{ formatBytes(activeDownloadBytes) }}<template v-if="activeDownloadTotal"> / {{ formatBytes(activeDownloadTotal) }}</template>
                  </span>
                </div>
              </div>

              <div>
                <div class="flex items-center justify-between gap-2">
                  <h3 class="text-[11px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("library.playlists.smart.allTracksTitle") }}</h3>
                  <span class="font-mono text-[11px] text-dim">{{ downloadedCount }}/{{ trackRows.length }} · {{ analyzeDone }}/{{ analyzeTotal }}</span>
                </div>
                <ul class="mt-2 max-h-56 divide-y divide-line overflow-y-auto border border-line">
                  <li
                    v-for="(row, index) in trackRows"
                    :key="`${row.key}-${index}`"
                    class="flex items-center gap-2 px-3 py-2 text-sm"
                    :class="row.current ? 'bg-accent/5' : ''"
                    :title="stateLabel(row.state)"
                  >
                    <span class="h-1.5 w-1.5 shrink-0 rounded-full" :class="dotClass(row.state)" />
                    <span class="w-6 shrink-0 text-right font-mono text-[11px] text-dim">{{ index + 1 }}</span>
                    <span class="min-w-0 flex-1 truncate" :class="row.state === 'skipped' ? 'text-amber-400/90' : row.state === 'pending' ? 'text-dim' : ''">{{ row.title }}</span>
                    <span v-if="row.bpm > 0" class="hidden shrink-0 font-mono text-[10px] text-dim sm:inline">{{ row.bpm.toFixed(0) }} BPM<span v-if="row.keyCamelot"> · {{ row.keyCamelot }}</span></span>
                    <span v-if="row.state === 'downloading' && downloadPercentFor(row.key) > 0" class="shrink-0 font-mono text-[10px] text-sky-400">{{ downloadPercentFor(row.key) }}%</span>
                    <span v-else-if="row.state === 'skipped'" class="shrink-0 font-mono text-[10px] text-amber-400">{{ stateLabel(row.state) }}</span>
                  </li>
                </ul>
              </div>

              <p class="text-[11px] leading-relaxed text-dim">{{ t("library.playlists.smart.backgroundHint") }}</p>
            </section>

            <!-- Result: just a compact status; the order itself is applied in
                 the playlist header and needs no preview here. -->
            <template v-if="summary && !busy">
              <div v-if="incomplete" class="mb-3 border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-400">
                {{ t("library.playlists.smart.partial", { analyzed: summary.stats.analyzed, total: progress?.total ?? (summary.previousOrder.length || summary.order.length) }) }}
              </div>

              <div class="flex items-center gap-3 border border-line bg-surface px-4 py-4">
                <span class="grid h-10 w-10 shrink-0 place-items-center bg-accent/10 text-accent">
                  <Sparkles :size="18" />
                </span>
                <div class="min-w-0 flex-1">
                  <p class="truncate text-sm font-semibold">{{ incomplete ? t("library.playlists.smart.resume") : t("library.playlists.smart.badgeReady") }}</p>
                  <p class="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-dim">
                    <span>{{ t("library.playlists.smart.stats", { analyzed: summary.stats.analyzed }) }}</span>
                    <span v-if="summary.stats.reused">{{ t("library.playlists.smart.reused", { count: summary.stats.reused }) }}</span>
                    <span v-if="summary.stats.skipped">{{ t("library.playlists.smart.skipped", { count: summary.stats.skipped }) }}</span>
                    <span v-if="summary.source === 'local'" class="text-amber-400">{{ t("library.playlists.smart.localOnly") }}</span>
                  </p>
                </div>
              </div>
            </template>

            <div v-if="!busy && !error && !summary" class="grid place-items-center gap-3 border border-dashed border-line px-4 py-12 text-center">
              <span class="grid h-11 w-11 place-items-center bg-accent/10 text-accent">
                <Sparkles :size="20" />
              </span>
              <p class="max-w-sm text-sm text-muted">{{ t("library.playlists.smart.emptyHint") }}</p>
            </div>
          </div>

          <footer class="flex items-center gap-2 border-t border-line px-5 py-3">
            <button
              v-if="busy"
              type="button"
              :disabled="cancelling"
              class="ak-clip-tr flex h-9 items-center gap-2 border border-line px-3 text-xs font-semibold transition-colors hover:border-accent hover:text-accent disabled:cursor-wait disabled:opacity-60"
              @click="cancelRun"
            >
              <LoaderCircle v-if="cancelling" :size="13" class="animate-spin" />
              {{ cancelling ? t("library.playlists.smart.cancelling") : t("library.playlists.smart.cancel") }}
            </button>
            <template v-else>
              <button
                v-if="!summary || incomplete"
                type="button"
                class="ak-clip-tr flex h-9 items-center gap-2 bg-accent px-4 text-xs font-bold text-accent-fg transition-transform hover:scale-[1.02] active:scale-95"
                @click="start"
              >
                <Sparkles :size="14" />
                {{ incomplete ? t("library.playlists.smart.resume") : t("library.playlists.smart.start") }}
              </button>
              <button
                v-else
                type="button"
                class="ak-clip-tr flex h-9 items-center gap-2 bg-accent px-4 text-xs font-bold text-accent-fg transition-transform hover:scale-[1.02] active:scale-95"
                @click="close"
              >
                {{ t("library.playlists.smart.close") }}
              </button>
              <button
                v-if="summary"
                type="button"
                class="grid h-9 w-9 place-items-center border border-line text-dim transition-colors hover:border-accent hover:text-accent"
                :title="t('library.playlists.smart.reanalyze')"
                :aria-label="t('library.playlists.smart.reanalyze')"
                @click="start"
              >
                <RefreshCw :size="14" />
              </button>
              <button
                v-if="summary"
                type="button"
                class="ml-auto grid h-9 w-9 place-items-center border border-line text-dim transition-colors hover:border-red-500/60 hover:text-red-400"
                :title="t('library.playlists.smart.remove')"
                :aria-label="t('library.playlists.smart.remove')"
                @click="remove"
              >
                <Trash2 :size="14" />
              </button>
            </template>
          </footer>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.modal-enter-active,
.modal-leave-active {
  transition: opacity 220ms ease;
}

.modal-enter-from,
.modal-leave-to {
  opacity: 0;
}

.modal-enter-active .ak-frame,
.modal-leave-active .ak-frame {
  transition: transform 260ms cubic-bezier(0.2, 0.8, 0.2, 1), opacity 260ms cubic-bezier(0.2, 0.8, 0.2, 1);
}

.modal-enter-from .ak-frame,
.modal-leave-to .ak-frame {
  opacity: 0;
  transform: translateY(12px) scale(0.97);
}
</style>
