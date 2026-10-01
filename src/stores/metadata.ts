import { ref } from "vue";
import { defineStore } from "pinia";
import { invoke } from "../api";
import { coverUrl, formatDuration } from "../lib/remote";
import type { CachedTrack } from "../lib/remote";
import { trackKey } from "../lib/sources";
import { usePlayerStore } from "./player";
import type { Track } from "./player";

/** Paths coalesced into one backend call. */
const BATCH_SIZE = 24;
/** Coalescing window before a queued request is sent. */
const FLUSH_DELAY_MS = 120;
/** How long a failed/empty attempt is remembered before it may be retried. */
const RETRY_AFTER_MS = 10 * 60 * 1000;

/**
 * Lazy track metadata.
 *
 * Rendering never triggers a full-library fetch. Instead, views ask for the
 * tracks they are about to show (rows/cards report visibility) and playback
 * asks for the current track; requests are coalesced per source and sent in
 * small batches. Results land in the same player-track patch path the cache
 * hydration uses, so the UI updates in place.
 */
export const useMetadataStore = defineStore("metadata", () => {
  const player = usePlayerStore();
  /** Keys waiting to be sent, in insertion order. */
  const queued = new Map<string, Track>();
  /** Keys with an in-flight request. */
  const inflight = new Set<string>();
  /** Key -> time of the last attempt, for failure/empty backoff. */
  const attempted = new Map<string, number>();
  /** Number of tracks waiting to be fetched (for optional progress UI). */
  const pending = ref(0);

  let timer = 0;
  let draining = false;

  function request(track: Track | null | undefined) {
    if (!track?.path || !track.sourceId || track.metaLoaded) return;
    const key = trackKey(track);
    if (!key || inflight.has(key) || queued.has(key)) return;
    const lastAttempt = attempted.get(key);
    if (lastAttempt && Date.now() - lastAttempt < RETRY_AFTER_MS) return;
    queued.set(key, track);
    pending.value = queued.size;
    schedule();
  }

  function requestMany(tracks: Track[]) {
    for (const track of tracks) request(track);
  }

  function schedule() {
    if (timer) return;
    timer = window.setTimeout(() => {
      timer = 0;
      void drain();
    }, FLUSH_DELAY_MS);
  }

  /** Send queued requests in per-source batches until the queue is empty. */
  async function drain() {
    if (draining) return;
    draining = true;
    try {
      while (queued.size > 0) {
        const first = queued.values().next().value;
        if (!first?.sourceId) {
          queued.clear();
          pending.value = 0;
          break;
        }
        const sourceId = first.sourceId;
        const batch: Track[] = [];
        for (const [key, track] of queued) {
          if (track.sourceId !== sourceId) continue;
          batch.push(track);
          queued.delete(key);
          inflight.add(key);
          if (batch.length >= BATCH_SIZE) break;
        }
        pending.value = queued.size;

        try {
          const results = await invoke<CachedTrack[]>("read_track_metadata_batch", {
            sourceId,
            paths: batch.map((track) => track.path as string),
          });
          apply(batch, results);
        } catch (error) {
          // Mark the attempt as failed so the spinner stops; the backoff still
          // allows a later retry.
          console.warn("[welkin] metadata batch failed", error);
          for (const track of batch) {
            const key = trackKey(track);
            if (key) attempted.set(key, Date.now());
            player.updateTrack(track.id, { metaFailed: true });
          }
        } finally {
          for (const track of batch) {
            const key = trackKey(track);
            if (key) inflight.delete(key);
          }
        }
      }
    } finally {
      draining = false;
    }
  }

  /** Merge batch results into the player tracks. */
  function apply(batch: Track[], results: CachedTrack[]) {
    const byPath = new Map(results.map((result) => [result.path, result]));
    // Resolve the *current* track by key: a library reload may have replaced
    // the objects (and their ids) while the batch was in flight.
    const byKey = new Map(
      player.tracks
        .filter((track) => track.path)
        .map((track) => [trackKey(track) as string, track]),
    );
    const patches: Array<{ id: number; patch: Partial<Track> }> = [];
    for (const track of batch) {
      const key = trackKey(track);
      if (key) attempted.set(key, Date.now());
      const result = track.path ? byPath.get(track.path) : undefined;
      const meta = result?.metadata;
      const current = key ? byKey.get(key) : undefined;
      const id = current?.id ?? track.id;
      // A `null` metadata means the read failed: stop the spinner but leave the
      // track retryable once the backoff elapses.
      if (!meta) {
        patches.push({ id, patch: { metaFailed: true } });
        continue;
      }
      const patch: Partial<Track> = { metaLoaded: true, metaFailed: false };
      if (meta.title) patch.title = meta.title;
      if (meta.artist) patch.artist = meta.artist;
      if (meta.album) patch.album = meta.album;
      if (meta.durationSecs) patch.duration = formatDuration(meta.durationSecs);
      const url = coverUrl(result?.coverPath, current?.modified ?? track.modified);
      if (url) patch.cover = url;
      if (meta.coverHash) patch.coverHash = meta.coverHash;
      patches.push({ id, patch });
    }
    player.updateTracks(patches);
  }

  return { pending, request, requestMany };
});
