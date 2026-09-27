import { ref, watch, type Ref } from "vue";
import type { Track } from "../stores/player";

export type TrackSortKey = "default" | "title" | "artist" | "album" | "duration";
export type TrackSortDir = "asc" | "desc";

/** Each page keeps its own sort preference instead of sharing one global choice. */
export type TrackSortContext = "tracks" | "favorites" | "playlist" | "artist" | "album";

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
  const field = key === "title" ? a.title : key === "artist" ? a.artist : a.album;
  const other = key === "title" ? b.title : key === "artist" ? b.artist : b.album;
  return collator.compare(field, other);
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
