/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

import { ref } from "vue";
import { defineStore } from "pinia";
import { invoke } from "../api";

/** Resume point persisted independently of the profile. */
interface PlaybackState {
  path: string;
  position: number;
  updatedAt: number;
  queue?: string[];
  queueIndex?: number;
}

const SAVE_DEBOUNCE_MS = 900;

export const usePlaybackStore = defineStore("playback", () => {
  const path = ref<string | null>(null);
  const position = ref(0);
  /** Track keys of the saved play queue, in playback order. */
  const queue = ref<string[]>([]);
  /** Index of the current track within `queue`. */
  const queueIndex = ref(0);
  const ready = ref(false);
  /** Set once the user starts playback, so a late cloud copy cannot override them. */
  const remoteLocked = ref(false);

  let saveTimer = 0;
  let queueTimer = 0;

  function adopt(state: PlaybackState) {
    path.value = state.path;
    position.value = Math.max(0, state.position);
    queue.value = Array.isArray(state.queue) ? state.queue : [];
    queueIndex.value = Number.isFinite(state.queueIndex) ? (state.queueIndex as number) : 0;
  }

  /** Stop accepting the remote resume point (the user has taken over locally). */
  function lockRemote() {
    remoteLocked.value = true;
  }

  /**
   * Load the locally cached resume point instantly, then refresh from the
   * authoritative remote copy in the background. When neither exists, seed from
   * the profile's legacy `lastTrack`/`lastPosition` once and push it out.
   */
  async function hydrate(seed?: { path?: string; position?: number }): Promise<boolean> {
    if (ready.value) return false;
    let seeded = false;
    try {
      const local = await invoke<PlaybackState | null>("load_local_playback");
      if (local && local.path) {
        adopt(local);
      } else if (seed?.path) {
        path.value = seed.path;
        position.value = Math.max(0, seed.position ?? 0);
        seeded = true;
        void persist(true);
      }
    } catch (error) {
      console.warn("[welkin] failed to load local playback state", error);
    } finally {
      ready.value = true;
    }
    void syncRemote();
    return seeded;
  }

  /** Replace the cached resume point when the remote copy has one. */
  async function syncRemote(): Promise<void> {
    try {
      const remote = await invoke<PlaybackState | null>("load_remote_playback");
      // A slow reply must never clobber a track the user already started.
      if (remote && remote.path && !remoteLocked.value) adopt(remote);
    } catch (error) {
      console.warn("[welkin] failed to load remote playback state", error);
    }
  }

  async function persist(remote: boolean, includeQueue = false): Promise<void> {
    const current = path.value;
    if (!current) return;
    try {
      const args: Record<string, unknown> = { path: current, position: position.value, remote };
      if (includeQueue) {
        args.queue = queue.value;
        args.queueIndex = queueIndex.value;
      }
      await invoke("save_playback", args);
    } catch (error) {
      console.warn("[welkin] failed to save playback state", error);
    }
  }

  /** Update the resume point and schedule a cheap local write. */
  function record(nextPath: string | undefined, nextPosition: number) {
    if (!nextPath) return;
    path.value = nextPath;
    position.value = Math.max(0, Math.round(nextPosition));
    window.clearTimeout(saveTimer);
    saveTimer = window.setTimeout(() => void persist(false), SAVE_DEBOUNCE_MS);
  }

  /** Replace the saved play queue and schedule a local write. */
  function recordQueue(keys: string[], index: number) {
    const same =
      index === queueIndex.value &&
      keys.length === queue.value.length &&
      keys.every((key, position) => key === queue.value[position]);
    if (same) return;
    queue.value = keys;
    queueIndex.value = index;
    window.clearTimeout(queueTimer);
    queueTimer = window.setTimeout(() => void persist(false, true), SAVE_DEBOUNCE_MS);
  }

  /** Write immediately, including the remote copy (pause / seek / exit). */
  function flush(): Promise<void> {
    window.clearTimeout(saveTimer);
    window.clearTimeout(queueTimer);
    return persist(true, true);
  }

  return { path, position, queue, queueIndex, ready, hydrate, record, recordQueue, flush, lockRemote };
});
