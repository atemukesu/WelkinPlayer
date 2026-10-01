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
 * Turn a cached cover thumbnail path into an asset-protocol URL the webview can
 * load directly. A version (the remote mtime) busts the browser cache when the
 * underlying file changes.
 */
export function coverUrl(coverPath: string | null | undefined, version?: string | null): string | undefined {
  if (!coverPath) return undefined;
  const url = convertFileSrc(coverPath);
  return version ? `${url}?v=${encodeURIComponent(version)}` : url;
}

/** Format whole seconds as `m:ss`. */
export function formatDuration(seconds: number): string {
  const minutes = Math.floor(seconds / 60);
  return `${minutes}:${String(seconds % 60).padStart(2, "0")}`;
}

export interface StreamEndpoint {
  url: string;
  token: string;
}

let streamEndpoint: StreamEndpoint | null = null;

/** Cache the loopback proxy address returned by the `stream_endpoint` command. */
export function setStreamEndpoint(endpoint: StreamEndpoint) {
  streamEndpoint = endpoint;
}

/**
 * URL the media element plays: the Rust loopback proxy streams the WebDAV
 * response (with Range support) so credentials stay in Rust and the browser
 * handles progressive playback natively.
 */
export function trackStreamUrl(remotePath: string, sourceId = ""): string {
  if (!streamEndpoint) return "";
  const query = `token=${encodeURIComponent(streamEndpoint.token)}&source=${encodeURIComponent(sourceId)}&path=${encodeURIComponent(remotePath)}`;
  return `${streamEndpoint.url}?${query}`;
}
