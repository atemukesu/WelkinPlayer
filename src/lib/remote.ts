/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

import { convertFileSrc } from "@tauri-apps/api/core";
import type { Track } from "../stores/player";
import type { SourceKind } from "./sources";

/** Shape of the `RemoteEntry` struct returned by the Rust `list_source_audio` command. */
export interface RemoteEntry {
  name: string;
  /** Remote href as returned by WebDAV (percent-encoded, URL semantics). */
  path: string;
  /** Normalized, base-stripped, decoded path using "/" separators. */
  localPath: string;
  isDir: boolean;
  size: number | null;
  modified: string | null;
  contentType: string | null;
}

const COLORS = ["#d86f4d", "#6e9b8a", "#4c72a6", "#a884b7", "#cc9b57"];

function colorFor(seed: string): string {
  let hash = 0;
  for (let index = 0; index < seed.length; index += 1) {
    hash = (hash * 31 + seed.charCodeAt(index)) >>> 0;
  }
  return COLORS[hash % COLORS.length];
}

function stripExtension(name: string): string {
  return name.replace(/\.[^./\\]+$/, "");
}

/** Minimal source descriptor attached to library tracks. */
export interface TrackSourceRef {
  id: string;
  name: string;
  kind: SourceKind;
}

/** Convert a source entry into the library's track shape. */
export function trackFromEntry(entry: RemoteEntry, index: number, source?: TrackSourceRef): Track {
  const segments = entry.localPath.split("/").filter(Boolean);
  const fallback = source?.kind === "local" ? (source.name || "本地") : "WebDAV";
  const artist = segments.length > 1 ? segments[segments.length - 2] : fallback;
  return {
    id: index + 1,
    title: stripExtension(entry.name) || entry.name,
    artist,
    album: entry.contentType ?? fallback,
    duration: "--:--",
    color: colorFor(`${source?.id ?? ""}:${entry.path || entry.name}`),
    path: entry.path,
    modified: entry.modified,
    sourceId: source?.id,
    sourceName: source?.name,
  };
}

/** Mirrors the Rust `TrackMetadata` struct. */
export interface TrackMetadata {
  title: string | null;
  artist: string | null;
  album: string | null;
  durationSecs: number | null;
  coverHash: string | null;
}

/** Mirrors the Rust `TrackTags` struct used by the metadata editor. */
export interface TrackTags {
  title: string | null;
  artist: string | null;
  album: string | null;
  albumArtist: string | null;
  genre: string | null;
  comment: string | null;
  year: number | null;
  trackNumber: number | null;
  trackTotal: number | null;
  discNumber: number | null;
  discTotal: number | null;
  hasCover: boolean;
}

/** Mirrors the Rust `CachedTrack` struct returned by `load_cached_tracks`. */
export interface CachedTrack {
  path: string;
  metadata: TrackMetadata | null;
  coverPath: string | null;
}

/**
 * URL for a local file the webview may load as a subresource.
 *
 * Production uses Tauri's `asset:` protocol. In dev the page is served from
 * `http://localhost:1420`, and WebKitGTK 2.54 refuses to load `asset://`
 * images from an http origin ("Domains, protocols and ports must match"), so
 * the same file is routed through Vite's `/@fs` handler instead. This is the
 * upstream workaround for tauri-apps/tauri#16201.
 */
function localFileUrl(path: string): string {
  if (!import.meta.env.DEV) return convertFileSrc(path);
  const normalized = path.replace(/\\/g, "/");
  const encoded = encodeURI(normalized).replace(/[?#]/g, (character) => encodeURIComponent(character));
  return `/@fs${encoded.startsWith("/") ? "" : "/"}${encoded}`;
}

/**
 * Turn a cached cover thumbnail path into a URL the webview can load directly.
 * A version (the remote mtime) busts the browser cache when the underlying file
 * changes.
 */
export function coverUrl(coverPath: string | null | undefined, version?: string | null): string | undefined {
  if (!coverPath) return undefined;
  const url = localFileUrl(coverPath);
  return version ? `${url}?v=${encodeURIComponent(version)}` : url;
}

/** Format whole seconds as `m:ss`. */
export function formatDuration(seconds: number): string {
  const minutes = Math.floor(seconds / 60);
  return `${minutes}:${String(seconds % 60).padStart(2, "0")}`;
}
