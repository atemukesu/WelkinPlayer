/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

import { computed, ref } from "vue";
import { defineStore } from "pinia";
import { invoke } from "../api";
import { i18n } from "../i18n";
import { sourceProfileId, splitTrackKey, trackKey } from "../lib/sources";
import { pushToast } from "../lib/toast";
import { usePlayerStore } from "./player";
import { useSourcesStore } from "./sources";

/** How often the complete/pinned key sets and download progress are re-read from Rust. */
const STATUS_POLL_MS = 1000;

/** One manual-cache entry mirrored from Rust. */
export interface CacheItem {
  key: string;
  pinned: boolean;
  complete: boolean;
  /** Upstream size in bytes, once known. */
  size: number | null;
  /** Bytes written to disk. */
  written: number;
  /** Whether the worker is downloading this entry right now. */
  active: boolean;
  /** Smoothed transfer rate in bytes per second, computed on the frontend. */
  speed: number;
}

/** Raw shape returned by the Rust `cache_status` command. */
interface CacheStatusPayload {
  cached: string[];
  pinned: string[];
  items: {
    key: string;
    pinned: boolean;
    complete: boolean;
    size: number | null;
    written: number;
    active: boolean;
  }[];
}

/**
 * Whole-track cache state for manually pinned tracks. Rust owns the files;
 * this store mirrors which keys are complete or pinned so lists can badge them,
 * and computes live download progress/speed for the download manager views.
 */
export const useCacheStore = defineStore("cache", () => {
  const cached = ref<Set<string>>(new Set());
  const pinned = ref<Set<string>>(new Set());
  const items = ref<CacheItem[]>([]);
  let timer = 0;
  /** Suppresses completion toasts for tracks already cached at startup. */
  let initialized = false;
  /** Previous byte count and timestamp per key, for speed calculation. */
  const samples = new Map<string, { written: number; at: number }>();

  const itemMap = computed(() => new Map(items.value.map((item) => [item.key, item])));
  /** Pinned entries that are still downloading or waiting in the queue. */
  const pendingItems = computed(() => items.value.filter((item) => item.pinned && !item.complete));
  /** Completed manual-cache entries. */
  const completedItems = computed(() => items.value.filter((item) => item.complete));
  /** Combined transfer rate of all active downloads. */
  const totalSpeed = computed(() =>
    pendingItems.value.reduce((sum, item) => sum + (item.active ? item.speed : 0), 0),
  );
  const activeCount = computed(() => pendingItems.value.filter((item) => item.active).length);
  const queuedCount = computed(() => pendingItems.value.filter((item) => !item.active).length);
  /** Aggregate progress (0–1) of the pending queue, or null while sizes are unknown. */
  const overallProgress = computed<number | null>(() => {
    const pending = pendingItems.value.filter((item) => item.size && item.size > 0);
    if (pending.length === 0) return null;
    const total = pending.reduce((sum, item) => sum + (item.size ?? 0), 0);
    const written = pending.reduce((sum, item) => sum + Math.min(item.written, item.size ?? item.written), 0);
    return total > 0 ? Math.min(1, written / total) : null;
  });
  const pendingWritten = computed(() =>
    pendingItems.value.reduce((sum, item) => sum + item.written, 0),
  );
  const pendingTotal = computed(() => {
    const known = pendingItems.value.filter((item) => item.size && item.size > 0);
    return known.length > 0 ? known.reduce((sum, item) => sum + (item.size ?? 0), 0) : 0;
  });
  /** Bytes occupied by completed manual-cache entries. */
  const completedBytes = computed(() =>
    completedItems.value.reduce((sum, item) => sum + (item.written || item.size || 0), 0),
  );

  function isCached(key: string | undefined): boolean {
    return !!key && cached.value.has(key);
  }

  function isPinned(key: string | undefined): boolean {
    return !!key && pinned.value.has(key);
  }

  function itemFor(key: string | undefined): CacheItem | undefined {
    return key ? itemMap.value.get(key) : undefined;
  }

  /** Progress (0–1) of a pinned, incomplete track, or null when not downloading. */
  function progressFor(key: string | undefined): number | null {
    const item = itemFor(key);
    if (!item || item.complete || !item.pinned) return null;
    if (item.size && item.size > 0) return Math.max(0, Math.min(1, item.written / item.size));
    return null;
  }

  /** A readable track name for a cache key, for the completion toast. */
  function trackLabel(key: string): string {
    const track = usePlayerStore().tracks.find((item) => trackKey(item) === key);
    if (track) return track.title;
    const { path } = splitTrackKey(key);
    return (path.split("/").pop() ?? path).replace(/\.[^./]+$/, "");
  }

  function trackFor(key: string) {
    return usePlayerStore().tracks.find((item) => trackKey(item) === key);
  }

  async function refresh() {
    try {
      const status = await invoke<CacheStatusPayload>("cache_status");
      const next = new Set(status.cached);
      // Toast when a manually cached track finishes downloading.
      if (initialized) {
        for (const key of next) {
          if (!cached.value.has(key) && pinned.value.has(key)) {
            pushToast("success", i18n.global.t("settings.cache.trackCached", { title: trackLabel(key) }));
          }
        }
      }
      const now = Date.now();
      const previous = new Map(items.value.map((item) => [item.key, item]));
      const nextItems: CacheItem[] = [];
      for (const raw of status.items ?? []) {
        const old = previous.get(raw.key);
        const last = samples.get(raw.key);
        let speed = 0;
        if (raw.active && !raw.complete) {
          if (last && now > last.at && raw.written >= last.written) {
            const instant = ((raw.written - last.written) * 1000) / (now - last.at);
            // Exponential smoothing keeps the displayed rate from flickering.
            speed = old ? old.speed * 0.55 + instant * 0.45 : instant;
          } else if (old) {
            speed = old.speed;
          }
        }
        samples.set(raw.key, { written: raw.written, at: now });
        nextItems.push({
          key: raw.key,
          pinned: raw.pinned,
          complete: raw.complete,
          size: raw.size,
          written: raw.written,
          active: raw.active,
          speed,
        });
      }
      // Keep optimistic entries that Rust has not reported yet, so a just-pinned
      // track immediately shows its progress ring instead of flickering away.
      for (const [key, item] of previous) {
        if (!nextItems.some((entry) => entry.key === key) && pinned.value.has(key) && !next.has(key)) {
          nextItems.push(item);
        }
      }
      cached.value = next;
      pinned.value = new Set(status.pinned);
      items.value = nextItems;
      // Drop stale speed samples for keys no longer present.
      for (const key of [...samples.keys()]) {
        if (!nextItems.some((item) => item.key === key)) samples.delete(key);
      }
      initialized = true;
    } catch {
      // Ignore: the badge simply stays as it was.
    }
  }

  /** Resolve the actual source id used by `pin_track`, skipping local files. */
  function pinTarget(key: string): { sourceId: string; path: string } | null {
    const { sourceId: profileId, path } = splitTrackKey(key);
    if (!path) return null;
    const source = useSourcesStore().sources.find((item) => sourceProfileId(item) === profileId || item.id === profileId);
    if (source?.kind === "local") return null;
    return { sourceId: source?.id ?? profileId, path };
  }

  function optimisticItem(key: string): CacheItem {
    const existing = itemMap.value.get(key);
    if (existing) return existing;
    return { key, pinned: true, complete: false, size: null, written: 0, active: false, speed: 0 };
  }

  // Pinned tracks are explicit user choices: downloaded first, never evicted.
  async function pin(key: string) {
    const target = pinTarget(key);
    if (!target) return;
    pinned.value = new Set(pinned.value).add(key);
    items.value = [...items.value.filter((item) => item.key !== key), optimisticItem(key)];
    const alreadyCached = cached.value.has(key);
    pushToast("info", i18n.global.t(alreadyCached ? "settings.cache.trackAlreadyCached" : "settings.cache.trackCaching", { title: trackLabel(key) }));
    try {
      await invoke("pin_track", { sourceId: target.sourceId, path: target.path });
    } catch {
      pinned.value = new Set([...pinned.value].filter((item) => item !== key));
      items.value = items.value.filter((item) => item.key !== key);
    }
  }

  /** Queue several tracks for manual caching at once. */
  async function pinMany(keys: string[]) {
    const targets: { key: string; sourceId: string; path: string }[] = [];
    for (const key of keys) {
      if (isPinned(key) || isCached(key)) continue;
      const target = pinTarget(key);
      if (target) targets.push({ key, ...target });
    }
    if (targets.length === 0) return;
    pinned.value = new Set([...pinned.value, ...targets.map((item) => item.key)]);
    items.value = [
      ...items.value.filter((item) => !targets.some((target) => target.key === item.key)),
      ...targets.map((target) => optimisticItem(target.key)),
    ];
    pushToast("info", i18n.global.t("downloads.queuedToast", { count: targets.length }));
    const results = await Promise.allSettled(
      targets.map((target) => invoke("pin_track", { sourceId: target.sourceId, path: target.path })),
    );
    const failed = targets.filter((_, index) => results[index]?.status === "rejected").map((target) => target.key);
    if (failed.length > 0) {
      pinned.value = new Set([...pinned.value].filter((key) => !failed.includes(key)));
      items.value = items.value.filter((item) => !failed.includes(item.key));
    }
    void refresh();
  }

  async function unpin(key: string) {
    // Drop both badges right away: the cached check must disappear the moment
    // the user cancels, not on the next status poll.
    pinned.value = new Set([...pinned.value].filter((item) => item !== key));
    cached.value = new Set([...cached.value].filter((item) => item !== key));
    items.value = items.value.filter((item) => item.key !== key);
    samples.delete(key);
    const { sourceId: profileId, path } = splitTrackKey(key);
    const sourceId = useSourcesStore().sources.find((item) => sourceProfileId(item) === profileId || item.id === profileId)?.id ?? profileId;
    try {
      await invoke("unpin_track", { sourceId, path });
    } catch {
      // Ignore; the refresh below re-syncs if the backend kept the entry.
    }
    // Reconcile with Rust so a failed removal restores the badges.
    void refresh();
  }

  /** Remove several cached tracks at once. */
  async function unpinMany(keys: string[]) {
    const targets = keys.filter((key) => isPinned(key) || isCached(key));
    if (targets.length === 0) return;
    pinned.value = new Set([...pinned.value].filter((key) => !targets.includes(key)));
    cached.value = new Set([...cached.value].filter((key) => !targets.includes(key)));
    items.value = items.value.filter((item) => !targets.includes(item.key));
    for (const key of targets) samples.delete(key);
    const sources = useSourcesStore();
    await Promise.allSettled(targets.map((key) => {
      const { sourceId: profileId, path } = splitTrackKey(key);
      const sourceId = sources.sources.find((item) => sourceProfileId(item) === profileId || item.id === profileId)?.id ?? profileId;
      return invoke("unpin_track", { sourceId, path });
    }));
    void refresh();
  }

  function togglePin(key: string) {
    if (isPinned(key)) void unpin(key);
    else void pin(key);
  }

  function start() {
    void refresh();
    if (!timer) timer = window.setInterval(() => void refresh(), STATUS_POLL_MS);
  }

  return {
    cached,
    pinned,
    items,
    pendingItems,
    completedItems,
    totalSpeed,
    activeCount,
    queuedCount,
    overallProgress,
    pendingWritten,
    pendingTotal,
    completedBytes,
    isCached,
    isPinned,
    itemFor,
    progressFor,
    trackLabel,
    trackFor,
    refresh,
    pin,
    pinMany,
    unpin,
    unpinMany,
    togglePin,
    start,
  };
});
