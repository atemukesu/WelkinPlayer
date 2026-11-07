/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

import { ref } from "vue";
import { defineStore } from "pinia";
import { invoke } from "../api";
import { i18n } from "../i18n";
import { sourceProfileId, splitTrackKey, trackKey } from "../lib/sources";
import { pushToast } from "../lib/toast";
import { usePlayerStore } from "./player";
import { useSourcesStore } from "./sources";

/** How often the complete/pinned key sets are re-read from Rust. */
const STATUS_POLL_MS = 5000;

/**
 * Whole-track cache state for manually pinned tracks. Rust owns the files;
 * this store mirrors which keys are complete or pinned so lists can badge them.
 */
export const useCacheStore = defineStore("cache", () => {
  const cached = ref<Set<string>>(new Set());
  const pinned = ref<Set<string>>(new Set());
  let timer = 0;
  /** Suppresses completion toasts for tracks already cached at startup. */
  let initialized = false;

  function isCached(key: string | undefined): boolean {
    return !!key && cached.value.has(key);
  }

  function isPinned(key: string | undefined): boolean {
    return !!key && pinned.value.has(key);
  }

  /** A readable track name for a cache key, for the completion toast. */
  function trackLabel(key: string): string {
    const track = usePlayerStore().tracks.find((item) => trackKey(item) === key);
    if (track) return track.title;
    const { path } = splitTrackKey(key);
    return (path.split("/").pop() ?? path).replace(/\.[^./]+$/, "");
  }

  async function refresh() {
    try {
      const status = await invoke<{ cached: string[]; pinned: string[] }>("cache_status");
      const next = new Set(status.cached);
      // Toast when a manually cached track finishes downloading.
      if (initialized) {
        for (const key of next) {
          if (!cached.value.has(key) && pinned.value.has(key)) {
            pushToast("success", i18n.global.t("settings.cache.trackCached", { title: trackLabel(key) }));
          }
        }
      }
      cached.value = next;
      pinned.value = new Set(status.pinned);
      initialized = true;
    } catch {
      // Ignore: the badge simply stays as it was.
    }
  }

  // Pinned tracks are explicit user choices: downloaded first, never evicted.
  async function pin(key: string) {
    const { sourceId: profileId, path } = splitTrackKey(key);
    const source = useSourcesStore().sources.find((item) => sourceProfileId(item) === profileId || item.id === profileId);
    const sourceId = source?.id ?? profileId;
    const local = source?.kind === "local";
    if (local) return;
    pinned.value = new Set(pinned.value).add(key);
    const alreadyCached = cached.value.has(key);
    pushToast("info", i18n.global.t(alreadyCached ? "settings.cache.trackAlreadyCached" : "settings.cache.trackCaching", { title: trackLabel(key) }));
    try {
      await invoke("pin_track", { sourceId, path });
    } catch {
      pinned.value = new Set([...pinned.value].filter((item) => item !== key));
    }
  }

  async function unpin(key: string) {
    pinned.value = new Set([...pinned.value].filter((item) => item !== key));
    const { sourceId: profileId, path } = splitTrackKey(key);
    const sourceId = useSourcesStore().sources.find((item) => sourceProfileId(item) === profileId || item.id === profileId)?.id ?? profileId;
    try {
      await invoke("unpin_track", { sourceId, path });
    } catch {
      // Ignore.
    }
  }

  function togglePin(key: string) {
    if (isPinned(key)) void unpin(key);
    else void pin(key);
  }

  function start() {
    void refresh();
    if (!timer) timer = window.setInterval(() => void refresh(), STATUS_POLL_MS);
  }

  return { cached, pinned, isCached, isPinned, refresh, pin, unpin, togglePin, start };
});
