/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

import { ref, watch, type Ref } from "vue";
import type { Track } from "../stores/player";
import { bucketLabel, bucketWeight } from "./reading";
import { reading } from "./readings";

export type TrackSortKey = "default" | "title" | "artist" | "album" | "duration";
export type TrackSortDir = "asc" | "desc";

/** Each page keeps its own sort preference instead of sharing one global choice. */
export type TrackSortContext = "tracks" | "favorites" | "playlist" | "artist" | "album" | "downloaded";

const LEGACY_KEY_STORAGE = "welkin-track-sort-key";
const LEGACY_DIR_STORAGE = "welkin-track-sort-dir";

function keyStorage(context: TrackSortContext) {
  return `welkin-track-sort-${context}-key`;
}
function dirStorage(context: TrackSortContext) {
  return `welkin-track-sort-${context}-dir`;
}

function normalizeKey(value: string | null): TrackSortKey {
  return value === "title" || value === "artist" || value === "album" || value === "duration" ? value : "default";
}

function readKey(context: TrackSortContext): TrackSortKey {
  const stored = localStorage.getItem(keyStorage(context));
  // The "全部歌曲" page inherits the old single global preference once.
  if (stored === null && context === "tracks") return normalizeKey(localStorage.getItem(LEGACY_KEY_STORAGE));
  return normalizeKey(stored);
}

function readDir(context: TrackSortContext): TrackSortDir {
  const stored = localStorage.getItem(dirStorage(context));
  if (stored === null && context === "tracks") return localStorage.getItem(LEGACY_DIR_STORAGE) === "desc" ? "desc" : "asc";
  return stored === "desc" ? "desc" : "asc";
}

export interface TrackSortState {
  key: Ref<TrackSortKey>;
  dir: Ref<TrackSortDir>;
}

function createState(context: TrackSortContext): TrackSortState {
  const key = ref<TrackSortKey>(readKey(context));
  const dir = ref<TrackSortDir>(readDir(context));
  watch(key, (value) => localStorage.setItem(keyStorage(context), value));
  watch(dir, (value) => localStorage.setItem(dirStorage(context), value));
  return { key, dir };
}

// Created eagerly so `useTrackSort` is a pure lookup and safe to call from computeds.
const registry: Record<TrackSortContext, TrackSortState> = {
  tracks: createState("tracks"),
  favorites: createState("favorites"),
  playlist: createState("playlist"),
  artist: createState("artist"),
  album: createState("album"),
  downloaded: createState("downloaded"),
};

/** Returns the persistent, reactive sort state for a page; the same context shares one instance. */
export function useTrackSort(context: TrackSortContext): TrackSortState {
  return registry[context];
}

const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

/** Parses "m:ss" / "h:mm:ss" / plain seconds; returns NaN when the value is not a clock. */
export function parseDuration(value: string): number {
  const text = value.trim();
  if (!/^\d+(:\d+)*$/.test(text)) return Number.NaN;
  return text.split(":").reduce((total, part) => total * 60 + Number(part), 0);
}

function compare(a: Track, b: Track, key: Exclude<TrackSortKey, "default">): number {
  if (key === "duration") {
    const left = parseDuration(a.duration);
    const right = parseDuration(b.duration);
    const leftKnown = !Number.isNaN(left);
    const rightKnown = !Number.isNaN(right);
    if (leftKnown && rightKnown) return left - right;
    if (leftKnown !== rightKnown) return leftKnown ? -1 : 1;
    return 0;
  }
  if (key === "title") return compareReadings(a.title, b.title);
  const field = key === "artist" ? a.artist : a.album;
  const other = key === "artist" ? b.artist : b.album;
  return collator.compare(field, other);
}

/**
 * Title order by romanized reading: hanzi/kana are read (pinyin/romaji) so the
 * list sorts alphabetically, latin titles stay as-is. Titles with no latin
 * reading (digits, symbols, other scripts) weight last so they gather under the
 * trailing `#` in the index rail; other ties fall back to a raw collation so the
 * order stays stable.
 */
function compareReadings(a: string, b: string): number {
  const left = reading(a);
  const right = reading(b);
  const leftWeight = bucketWeight(left);
  const rightWeight = bucketWeight(right);
  if (leftWeight !== rightWeight) return leftWeight - rightWeight;
  return collator.compare(left, right);
}

/** Returns a new array sorted by the given key/direction; "default" returns the source untouched. */
export function sortTracks(tracks: Track[], key: TrackSortKey, dir: TrackSortDir): Track[] {
  if (key === "default") return tracks;
  const factor = dir === "desc" ? -1 : 1;
  return tracks
    .map((track, index) => ({ track, index }))
    // Fall back to the original position so equal rows keep their order.
    .sort((a, b) => compare(a.track, b.track, key) * factor || a.index - b.index)
    .map((entry) => entry.track);
}

/** One entry of the alphabetical index rail. */
export interface LetterIndexEntry {
  /** `A`–`Z`, or `#` for titles with no latin reading. */
  label: string;
  /** Index of that bucket's first track in the given (title-sorted) list. */
  anchor: number;
  /** Whether the list actually has tracks in this bucket. */
  present: boolean;
}

/**
 * Build the alphabetical index rail for a title-sorted list. Every track is
 * bucketed by its romanized reading; `anchor` points at the first track of each
 * bucket *in the given (title-sorted) order*, so a caller can scroll to it.
 * All `A`–`Z` labels plus `#` are always returned; `present` tells the rail
 * which ones to show.
 */
export function titleLetterIndex(tracks: Track[]): LetterIndexEntry[] {
  if (tracks.length === 0) return [];
  const anchors = new Map<string, number>();
  const counts = new Map<string, number>();
  tracks.forEach((track, index) => {
    const label = bucketLabel(reading(track.title));
    if (!anchors.has(label)) anchors.set(label, index);
    counts.set(label, (counts.get(label) ?? 0) + 1);
  });

  const labels = [...Array.from({ length: 26 }, (_, i) => String.fromCharCode(65 + i)), "#"];
  return labels.map((label) => ({
    label,
    // Anchor at the end so an absent bucket still jumps somewhere sensible.
    anchor: anchors.get(label) ?? tracks.length,
    present: (counts.get(label) ?? 0) > 0,
  }));
}

// --- Grouped library (歌手 / 专辑 grid) sorting ---

/** Cards on the artist and album grid pages sort by display name or track count only. */
export type GroupSortKey = "default" | "name" | "count";
export type GroupSortContext = "artists" | "albums";

function groupKeyStorage(context: GroupSortContext) {
  return `welkin-group-sort-${context}-key`;
}
function groupDirStorage(context: GroupSortContext) {
  return `welkin-group-sort-${context}-dir`;
}

function normalizeGroupKey(value: string | null): GroupSortKey {
  return value === "name" || value === "count" ? value : "default";
}

export interface GroupSortState {
  key: Ref<GroupSortKey>;
  dir: Ref<TrackSortDir>;
}

function createGroupState(context: GroupSortContext): GroupSortState {
  const key = ref<GroupSortKey>(normalizeGroupKey(localStorage.getItem(groupKeyStorage(context))));
  const dir = ref<TrackSortDir>(localStorage.getItem(groupDirStorage(context)) === "desc" ? "desc" : "asc");
  watch(key, (value) => localStorage.setItem(groupKeyStorage(context), value));
  watch(dir, (value) => localStorage.setItem(groupDirStorage(context), value));
  return { key, dir };
}

// Created eagerly so `useGroupSort` is a pure lookup and safe to call from computeds.
const groupRegistry: Record<GroupSortContext, GroupSortState> = {
  artists: createGroupState("artists"),
  albums: createGroupState("albums"),
};

/** Returns the persistent, reactive sort state for a grid page; the same context shares one instance. */
export function useGroupSort(context: GroupSortContext): GroupSortState {
  return groupRegistry[context];
}

/** The minimal shape a grid card needs to be sortable by name or track count. */
export interface SortableGroup {
  name: string;
  count: number;
}

/** Returns a new array sorted by display name or track count; "default" returns the source untouched. */
export function sortGroups<T extends SortableGroup>(groups: T[], key: GroupSortKey, dir: TrackSortDir): T[] {
  if (key === "default") return groups;
  const factor = dir === "desc" ? -1 : 1;
  return groups
    .map((group, index) => ({ group, index }))
    // Fall back to the original position so equal rows keep their order.
    .sort((a, b) => {
      const result = key === "count" ? a.group.count - b.group.count : collator.compare(a.group.name, b.group.name);
      return result * factor || a.index - b.index;
    })
    .map((entry) => entry.group);
}
