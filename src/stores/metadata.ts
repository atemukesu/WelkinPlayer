/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

import { ref } from "vue";
import { defineStore } from "pinia";
import { invoke } from "../api";
import { Channel } from "@tauri-apps/api/core";
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
          const channel = new Channel<CachedTrack>();
          channel.onmessage = (result) => applyOne(sourceId, result);
          await invoke("read_track_metadata_batch", {
            sourceId,
            paths: batch.map((track) => track.path as string),
            onEvent: channel,
          });
        } catch (error) {
          // Mark the attempt as failed so the spinner stops; the backoff still
          // allows a later retry.
          console.warn("[welkin] metadata batch failed", error);
          for (const track of batch) {
            const trackKeyValue = trackKey(track);
            if (trackKeyValue) attempted.set(trackKeyValue, Date.now());
            if (trackKeyValue) player.updateTrackByKey(trackKeyValue, { metaFailed: true });
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

  /** Merge a single streamed result into the player tracks. */
  function applyOne(sourceId: string, result: CachedTrack) {
    // The batch is per source. Path-only matching can update another source,
    // and numeric ids are unsafe for Rust-created id=-1 placeholders.
    const key = `${sourceId}::${result.path}`;
    const current = player.tracks.find((track) => trackKey(track) === key) ??
      (trackKey(player.currentTrack) === key ? player.currentTrack : null);
    if (key) attempted.set(key, Date.now());
    const meta = result?.metadata;
    if (!meta) {
      if (current) player.updateTrackByKey(key, { metaFailed: true });
      return;
    }
    const patch: Partial<Track> = { metaLoaded: true, metaFailed: false };
    if (meta.title) patch.title = meta.title;
    if (meta.artist) patch.artist = meta.artist;
    if (meta.album) patch.album = meta.album;
    if (meta.durationSecs) patch.duration = formatDuration(meta.durationSecs);
    const url = coverUrl(result?.coverPath, current?.modified);
    if (url) patch.cover = url;
    if (meta.coverHash) patch.coverHash = meta.coverHash;
    player.updateTrackByKey(key, patch);
  }

  // apply removed; replaced by applyOne streaming per-track patches

  return { pending, request, requestMany };
});
