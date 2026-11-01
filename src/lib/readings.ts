/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

/**
 * Disk-backed cache of romanized title readings.
 *
 * Computing a reading (pinyin / romaji) is deterministic but not free, and a
 * library can hold thousands of titles that would otherwise be re-read on every
 * launch. The frontend keeps an in-memory map and flushes it to the backend,
 * which stores it under the configured cache directory (`readings.json`). Once
 * a title is known its reading is never recomputed.
 *
 * The map is deliberately non-reactive: sorting and filtering already recompute
 * whenever the library changes, and a computed must not mutate state while it
 * runs.
 */

import { invoke } from "../api";
import { sortReading } from "./reading";

/** title -> normalized reading, for titles whose reading has been computed. */
const cache = new Map<string, string>();
/** Titles whose reading was computed this session but not yet flushed. */
const dirty = new Set<string>();
let loaded = false;
let flushTimer = 0;

/** Merge backend-provided readings into the in-memory map (never overwrites). */
function merge(entries: Record<string, string>): void {
  if (!entries) return;
  for (const [title, reading] of Object.entries(entries)) {
    if (!cache.has(title)) cache.set(title, reading);
  }
}

/** Load the persisted map once; later calls are no-ops. */
export async function loadReadings(): Promise<void> {
  if (loaded) return;
  loaded = true;
  try {
    merge(await invoke<Record<string, string>>("load_sort_readings"));
  } catch {
    // Missing/unreadable cache simply means nothing is remembered yet.
  }
}

/** Send every unflushed reading to the backend. Failures are retried later. */
async function flush(): Promise<void> {
  if (dirty.size === 0) return;
  const batch: Record<string, string> = {};
  for (const title of dirty) {
    const reading = cache.get(title);
    if (reading !== undefined) batch[title] = reading;
  }
  dirty.clear();
  try {
    await invoke("save_sort_readings", { readings: batch });
  } catch {
    // Keep them pending so the next flush retries.
    for (const title of Object.keys(batch)) dirty.add(title);
  }
}

function scheduleFlush(): void {
  if (flushTimer) return;
  // Coalesce the burst of readings a sort produces into one write.
  flushTimer = window.setTimeout(() => {
    flushTimer = 0;
    void flush();
  }, 1000);
}

/**
 * The normalized reading (sort key) for a title, computed and memoized on first
 * use. Reads are synchronous; new values are persisted in the background.
 */
export function reading(title: string): string {
  const known = cache.get(title);
  if (known !== undefined) return known;
  const value = sortReading(title);
  cache.set(title, value);
  dirty.add(title);
  scheduleFlush();
  return value;
}

/** Flush any pending readings immediately (e.g. on unload). */
export function flushReadings(): Promise<void> {
  return flush();
}
