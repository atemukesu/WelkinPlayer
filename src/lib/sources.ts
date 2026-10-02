/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

/**
 * Song sources.
 *
 * A source is somewhere audio is read from: a WebDAV collection or a local
 * directory. The list is persisted in `settings.json` (owned by the sources
 * store); the backend reads the same configuration. WebDAV passwords live in
 * the OS keychain, keyed by the source id.
 */

export type SourceKind = "webdav" | "local";

export interface SongSource {
  /** Stable identifier; forms the profile key namespace. */
  id: string;
  kind: SourceKind;
  /** Human-readable label. */
  name: string;
  /** WebDAV collection URL. */
  url?: string;
  /** WebDAV username. */
  username?: string;
  /** Whether plaintext HTTP to a public host is allowed for this source. */
  allowInsecure?: boolean;
  /** Absolute directory for a local source. */
  rootPath?: string;
}

/**
 * Stable identity for data that travels between devices. `id` is intentionally
 * machine-local because it also namespaces the OS keychain and native cache.
 */
export function sourceProfileId(source: Pick<SongSource, "kind" | "url" | "username" | "rootPath" | "id">): string {
  if (source.kind !== "webdav") return source.id;
  let endpoint = (source.url ?? "").trim();
  try {
    const parsed = new URL(endpoint);
    parsed.hostname = parsed.hostname.toLowerCase();
    if ((parsed.protocol === "https:" && parsed.port === "443") || (parsed.protocol === "http:" && parsed.port === "80")) {
      parsed.port = "";
    }
    parsed.pathname = parsed.pathname.replace(/\/+$/, "") || "/";
    parsed.hash = "";
    endpoint = parsed.toString().replace(/\/$/, "");
  } catch {
    endpoint = endpoint.replace(/\/+$/, "");
  }
  const value = `${endpoint}\u0000${(source.username ?? "").trim()}`;
  let hash = 2166136261;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 16777619) >>> 0;
  }
  return `dav-${hash.toString(36)}`;
}

/** Key used by the synchronized profile, independent of a device's source id. */
export function profileTrackKey(track: { sourceId?: string; path?: string; profileSourceId?: string } | null | undefined): string | undefined {
  if (!track?.path) return undefined;
  return `${track.profileSourceId ?? track.sourceId ?? ""}${KEY_SEPARATOR}${track.path}`;
}

/**
 * Selectable source kinds, in the order shown when adding a source. Adding a
 * new kind later means appending here plus a form branch in the editor.
 */
export const SOURCE_KINDS: { kind: SourceKind; labelKey: string }[] = [
  { kind: "webdav", labelKey: "sources.kindWebdav" },
  { kind: "local", labelKey: "sources.kindLocal" },
];

/** Separator between a source id and a track path inside a profile key. */
const KEY_SEPARATOR = "::";

/** The unique key used to reference a track in favorites/playlists/play-counts. */
export function trackKey(track: { sourceId?: string; path?: string } | null | undefined): string | undefined {
  if (!track?.path) return undefined;
  return `${track.sourceId ?? ""}${KEY_SEPARATOR}${track.path}`;
}

/** Split a profile key back into its source id and raw path. */
export function splitTrackKey(key: string): { sourceId: string; path: string } {
  const index = key.indexOf(KEY_SEPARATOR);
  if (index < 0) return { sourceId: "", path: key };
  return { sourceId: key.slice(0, index), path: key.slice(index + KEY_SEPARATOR.length) };
}

/** A short, non-empty label for a source. */
export function sourceLabel(source: SongSource): string {
  const name = source.name?.trim();
  if (name) return name;
  if (source.kind === "local") return source.rootPath?.split(/[\\/]/).filter(Boolean).pop() ?? "Local";
  return source.url ?? "WebDAV";
}

/** Whether a source can be used as the cloud-sync target. */
export function isCloudSource(source: SongSource): boolean {
  return source.kind === "webdav";
}
