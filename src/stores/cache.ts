import { ref, watch } from "vue";
import { defineStore } from "pinia";
import { invoke } from "../api";
import { i18n } from "../i18n";
import { splitTrackKey, trackKey } from "../lib/sources";
import { pushToast } from "../lib/toast";
import { useProfileStore } from "./profile";
import { useSourcesStore } from "./sources";
import { usePlayerStore } from "./player";

/** Only tracks played at least this many times are offered for whole caching. */
const MIN_PLAYS = 5;
/** Upper bound on ranked candidates sent to Rust (the "top" of the ranking). */
const MAX_CANDIDATES = 300;
/** How often the complete/pinned key sets are re-read from Rust. */
const STATUS_POLL_MS = 5000;

/**
 * Whole-track ("smart") cache state. Rust owns the files; this store mirrors
 * which keys are complete or pinned so lists can badge them, and pushes the
 * ranked play counts that drive automatic caching.
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

  /** Push the ranked, thresholded play counts to Rust. */
  async function syncCandidates() {
    const profile = useProfileStore();
    // Only sources this device knows and can stream from. Play counts are
    // synced, so keys may reference another device's source id.
    const knownIds = new Set(useSourcesStore().sources.filter((source) => source.kind !== "local").map((source) => source.id));
    const candidates = Object.entries(profile.profile.playCounts)
      .filter(([, count]) => count >= MIN_PLAYS)
      .sort((a, b) => b[1] - a[1])
      .slice(0, MAX_CANDIDATES)
      .map(([key, count]) => ({ ...splitTrackKey(key), freq: count }))
      .filter((item) => item.path && knownIds.has(item.sourceId));
    try {
      await invoke("sync_smart_cache", { candidates });
    } catch {
      // Ignore: caching is best-effort.
    }
  }

  // Pinned tracks are explicit user choices: downloaded first, never evicted.
  async function pin(key: string) {
    const { sourceId, path } = splitTrackKey(key);
    const local = useSourcesStore().sources.some((source) => source.id === sourceId && source.kind === "local");
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
    const { sourceId, path } = splitTrackKey(key);
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
    void syncCandidates();
    const profile = useProfileStore();
    watch(
      () => JSON.stringify(profile.profile.playCounts),
      () => void syncCandidates(),
    );
    // Re-sync once sources finish loading (or change), so keys with unknown
    // source ids are not offered before we can resolve them.
    const sources = useSourcesStore();
    watch(
      () => sources.sources.map((source) => source.id).join(","),
      () => void syncCandidates(),
    );
    if (!timer) timer = window.setInterval(() => void refresh(), STATUS_POLL_MS);
  }

  return { cached, pinned, isCached, isPinned, refresh, syncCandidates, pin, unpin, togglePin, start };
});
