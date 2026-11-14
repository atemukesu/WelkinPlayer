/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

/**
 * Smart-transition store.
 *
 * Holds the analysis summaries/progress for the playlists the user has opened
 * and bridges them to two systems:
 *
 * * the profile — a small pointer (`smartTransition`) plus the reordered track
 *   list, so the order and badge survive a restart and sync to other devices;
 * * the Rust playback context — the adjacent transition plans, installed just
 *   before a queue starts so the engine can fade at the analysed points.
 */

import { computed, ref } from "vue";
import { defineStore } from "pinia";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "../api";
import { useProfileStore, tracksForPaths } from "./profile";
import { usePlayerStore } from "./player";
import { trackKey } from "../lib/sources";
import {
  analyzePlaylist,
  cancelSmartTransition,
  deleteSmartTransition,
  loadSmartTransition,
  loadTransitionPlans,
  type AnalysisProgress,
  type SmartTransitionSummary,
  type SmartTrackInput,
} from "../lib/smartTransition";
import type { Playlist } from "../lib/profile";

interface ProgressState {
  done: number;
  total: number;
  phase: AnalysisProgress["phase"];
}

export const useSmartTransitionStore = defineStore("smartTransition", () => {
  const profile = useProfileStore();
  const player = usePlayerStore();

  const summaries = ref<Record<string, SmartTransitionSummary>>({});
  const progress = ref<Record<string, ProgressState>>({});
  const running = ref<Record<string, boolean>>({});
  const cancelling = ref<Record<string, boolean>>({});
  const loading = ref<Record<string, boolean>>({});
  const errors = ref<Record<string, string | null>>({});

  let unlisten: UnlistenFn[] = [];
  let listening: Promise<void> | null = null;

  /** Subscribe once to the Rust progress and partial-result events. */
  function ensureListener(): Promise<void> {
    if (listening) return listening;
    listening = Promise.all([
      listen<AnalysisProgress>("smart-transition:progress", (event) => {
        const payload = event.payload;
        if (!payload?.playlistId) return;
        progress.value = {
          ...progress.value,
          [payload.playlistId]: {
            done: payload.done,
            total: payload.total,
            phase: payload.phase,
          },
        };
      }),
      listen<SmartTransitionSummary>("smart-transition:partial", (event) => {
        const payload = event.payload;
        if (!payload?.playlistId) return;
        // Guard against a late snapshot overwriting the finished document of
        // the same run (both share a revision).
        const current = summaries.value[payload.playlistId];
        if (
          payload.incomplete &&
          current &&
          !current.incomplete &&
          current.revision >= payload.revision
        ) {
          return;
        }
        // Live snapshots are shown as they arrive; they are marked incomplete
        // and never update the profile pointer, so they cannot be applied or
        // used for playback until the analysis finishes.
        summaries.value = { ...summaries.value, [payload.playlistId]: payload };
      }),
    ])
      .then((stops) => {
        unlisten = stops;
      })
      .catch((error) => {
        console.warn("[welkin] smart-transition listener failed", error);
        listening = null;
      });
    return listening;
  }

  function isBusy(playlistId: string): boolean {
    return running.value[playlistId] === true;
  }

  function isCancelling(playlistId: string): boolean {
    return cancelling.value[playlistId] === true;
  }

  function summaryFor(playlistId: string): SmartTransitionSummary | null {
    return summaries.value[playlistId] ?? null;
  }

  function progressFor(playlistId: string): ProgressState | null {
    return progress.value[playlistId] ?? null;
  }

  function errorFor(playlistId: string): string | null {
    return errors.value[playlistId] ?? null;
  }

  function clearError(playlistId: string) {
    if (!errors.value[playlistId]) return;
    errors.value = { ...errors.value, [playlistId]: null };
  }

  /** Resolve a playlist's entries to playable tracks and `sourceId::path` keys. */
  function resolvedTracks(playlist: Playlist) {
    const tracks = tracksForPaths(playlist.tracks, player.tracks);
    const entries = new Map<string, string>();
    const inputs: SmartTrackInput[] = [];
    const orderKeys: string[] = [];
    playlist.tracks.forEach((entry, index) => {
      const track = tracks[index];
      const key = trackKey(track);
      if (!track?.path || !key) {
        orderKeys.push(entry);
        return;
      }
      inputs.push({ key, title: track.title });
      orderKeys.push(key);
      if (!entries.has(key)) entries.set(key, entry);
      // Older saved orders may still contain the profile key; accept it too.
      if (!entries.has(entry)) entries.set(entry, entry);
    });
    return { inputs, entries, orderKeys };
  }

  /** Map a list of playback keys back to the profile's playlist entry order. */
  function toPlaylistEntries(playlist: Playlist, keys: string[]): string[] {
    const { entries } = resolvedTracks(playlist);
    const ordered: string[] = [];
    for (const key of keys) {
      const entry = entries.get(key);
      if (entry && !ordered.includes(entry)) ordered.push(entry);
    }
    // Keep entries that could not be resolved (unavailable on this device) at
    // the end so applying an order never loses them.
    for (const entry of playlist.tracks) {
      if (!ordered.includes(entry)) ordered.push(entry);
    }
    return ordered;
  }

  /** Load the stored summary, updating the profile pointer when it is ready. */
  async function load(playlistId: string): Promise<SmartTransitionSummary | null> {
    if (loading.value[playlistId]) return summaryFor(playlistId);
    loading.value = { ...loading.value, [playlistId]: true };
    try {
      const summary = await loadSmartTransition(playlistId);
      if (summary) {
        summaries.value = { ...summaries.value, [playlistId]: summary };
        // A partial snapshot is only resumable context; it must not mark the
        // playlist as analysed or flip an existing badge to ready.
        if (summary.incomplete) return summary;
        const playlist = profile.playlists.find((item) => item.id === playlistId);
        if (playlist) {
          const previous = playlist.smartTransition;
          profile.updatePlaylist(playlistId, {
            smartTransition: {
              enabled: previous?.enabled ?? false,
              file: summary.file,
              algorithmVersion: 2,
              updatedAt: summary.updatedAt,
              // Opening the dialog must not clear a stale marker: the stored
              // document still describes the pre-edit track list.
              status: previous?.status === "stale" ? "stale" : "ready",
            },
          });
        }
      }
      return summary;
    } catch (error) {
      console.warn("[welkin] failed to load smart transition", error);
      return null;
    } finally {
      loading.value = { ...loading.value, [playlistId]: false };
    }
  }

  /** Run (or re-run) the analysis. Cached tracks are not decoded again. */
  async function analyze(playlistId: string): Promise<SmartTransitionSummary | null> {
    const playlist = profile.playlists.find((item) => item.id === playlistId);
    if (!playlist || isBusy(playlistId)) return null;
    await ensureListener();
    const { inputs, orderKeys } = resolvedTracks(playlist);
    if (inputs.length === 0) {
      errors.value = { ...errors.value, [playlistId]: "no-playable-tracks" };
      return null;
    }
    const previous = summaries.value[playlistId]?.previousOrder;
    running.value = { ...running.value, [playlistId]: true };
    cancelling.value = { ...cancelling.value, [playlistId]: false };
    clearError(playlistId);
    try {
      const summary = await analyzePlaylist(playlistId, playlist.name, inputs, {
        previousOrder: previous && previous.length > 0 ? previous : orderKeys,
      });
      summaries.value = { ...summaries.value, [playlistId]: summary };
      profile.updatePlaylist(playlistId, {
        smartTransition: {
          enabled: playlist.smartTransition?.enabled ?? false,
          file: summary.file,
          algorithmVersion: 2,
          updatedAt: summary.updatedAt,
          status: "ready",
        },
      });
      return summary;
    } catch (error) {
      // A user cancellation is not an error: the partial snapshot emitted so
      // far stays visible and can be resumed later.
      if (cancelling.value[playlistId]) return null;
      const message = error instanceof Error ? error.message : String(error);
      errors.value = { ...errors.value, [playlistId]: message };
      return null;
    } finally {
      running.value = { ...running.value, [playlistId]: false };
      cancelling.value = { ...cancelling.value, [playlistId]: false };
    }
  }

  async function cancel(playlistId: string): Promise<void> {
    if (!running.value[playlistId] || cancelling.value[playlistId]) return;
    cancelling.value = { ...cancelling.value, [playlistId]: true };
    try {
      await cancelSmartTransition(playlistId);
    } catch (error) {
      cancelling.value = { ...cancelling.value, [playlistId]: false };
      console.warn("[welkin] failed to cancel smart transition", error);
    }
  }

  /** Apply the analysed order and enable transitions. */
  function applyOrder(playlistId: string): boolean {
    const playlist = profile.playlists.find((item) => item.id === playlistId);
    const summary = summaries.value[playlistId];
    if (!playlist || !summary || summary.incomplete) return false;
    const tracks = toPlaylistEntries(playlist, summary.order);
    profile.updatePlaylist(playlistId, {
      tracks,
      smartTransition: {
        enabled: true,
        file: summary.file,
        algorithmVersion: 2,
        updatedAt: summary.updatedAt,
        status: "ready",
      },
    });
    return true;
  }

  /** Enable transitions without changing the playlist's current order. */
  function enableOnly(playlistId: string): boolean {
    const playlist = profile.playlists.find((item) => item.id === playlistId);
    const summary = summaries.value[playlistId];
    if (!playlist || !summary || summary.incomplete) return false;
    profile.updatePlaylist(playlistId, {
      smartTransition: {
        enabled: true,
        file: summary.file,
        algorithmVersion: 2,
        updatedAt: summary.updatedAt,
        status: "ready",
      },
    });
    return true;
  }

  /** Restore the order that existed before the first analysis. */
  function undoOrder(playlistId: string): boolean {
    const playlist = profile.playlists.find((item) => item.id === playlistId);
    const summary = summaries.value[playlistId];
    if (!playlist || !summary || summary.previousOrder.length === 0) return false;
    const tracks = toPlaylistEntries(playlist, summary.previousOrder);
    profile.updatePlaylist(playlistId, {
      tracks,
      smartTransition: playlist.smartTransition
        ? { ...playlist.smartTransition, enabled: false, status: "stale" }
        : undefined,
    });
    return true;
  }

  /** Disable transitions without touching the order. */
  function disable(playlistId: string): void {
    const playlist = profile.playlists.find((item) => item.id === playlistId);
    if (!playlist?.smartTransition) return;
    profile.updatePlaylist(playlistId, {
      smartTransition: { ...playlist.smartTransition, enabled: false },
    });
  }

  /** Delete the stored document and its profile pointer. */
  async function remove(playlistId: string): Promise<void> {
    try {
      await deleteSmartTransition(playlistId);
    } catch (error) {
      console.warn("[welkin] failed to delete smart transition", error);
    }
    const next = { ...summaries.value };
    delete next[playlistId];
    summaries.value = next;
    const playlist = profile.playlists.find((item) => item.id === playlistId);
    if (playlist?.smartTransition) {
      profile.updatePlaylist(playlistId, { smartTransition: undefined });
    }
  }

  /** Remove any installed plans (e.g. when the visible order no longer matches). */
  async function clearPlans(): Promise<void> {
    await invoke("playback_clear_transition_plans").catch(() => {});
  }

  /**
   * Install plans into the Rust playback context just before a queue starts.
   * A stale/disabled analysis or shuffle leaves the queue on a normal hard
   * transition, which is always safe.
   */
  async function armPlayback(playlistId: string): Promise<void> {
    const playlist = profile.playlists.find((item) => item.id === playlistId);
    if (!playlist?.smartTransition?.enabled || playlist.smartTransition.status !== "ready" || player.shuffle) {
      await invoke("playback_clear_transition_plans").catch(() => {});
      return;
    }    try {
      let plans = await loadTransitionPlans(playlistId);
      if (plans.length === 0) {
        // First play on this device: pull the document once, then use the
        // local copy from then on.
        await load(playlistId);
        plans = await loadTransitionPlans(playlistId);
      }
      if (plans.length === 0) {
        await invoke("playback_clear_transition_plans");
        return;
      }
      await invoke("playback_set_transition_plans", { plans });
    } catch (error) {
      console.warn("[welkin] failed to install transition plans", error);
      await invoke("playback_clear_transition_plans").catch(() => {});
    }
  }

  const enabledCount = computed(
    () => profile.playlists.filter((item) => item.smartTransition?.enabled).length,
  );

  function dispose() {
    unlisten.forEach((stop) => stop());
    unlisten = [];
    listening = null;
  }

  return {
    summaries,
    progress,
    running,
    cancelling,
    loading,
    enabledCount,
    ensureListener,
    isBusy,
    isCancelling,
    summaryFor,
    progressFor,
    errorFor,
    clearError,
    load,
    analyze,
    cancel,
    applyOrder,
    enableOnly,
    undoOrder,
    disable,
    remove,
    armPlayback,
    clearPlans,
    dispose,
  };
});
