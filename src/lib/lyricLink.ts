//! Derive the WebDAV sidecar location and the platform page for a lyric.
//!
//! The lyric sidecar lives next to the audio file as a same-named `.lrc`
//! (see the Rust `lyric_path` mirror in `src-tauri/src/commands/media.rs`).
//! Provenance written by [`tagLyric`](../lib/lyricTag) lets us turn a
//! QQ / NetEase fetch back into the song's public page.

/** Replace the audio extension with `.lrc`, mirroring the Rust `lyric_path`. */
export function lyricSidecarPath(trackPath: string | null | undefined): string | null {
  if (!trackPath) return null;
  const slash = trackPath.lastIndexOf("/");
  if (slash < 0) return null;
  const fileName = trackPath.slice(slash + 1);
  const dot = fileName.lastIndexOf(".");
  if (dot < 0) return null;
  return `${trackPath.slice(0, slash + 1)}${fileName.slice(0, dot)}.lrc`;
}

/** Build the public song page for a QQ / NetEase lyric, if one can be derived. */
export function lyricPlatformLink(source: string | null | undefined, sourceId: string | null | undefined): string | null {
  const id = sourceId?.trim();
  if (!id) return null;
  const kind = source?.trim().toLowerCase() ?? "";

  if (kind === "netease" || kind === "ncm") {
    const songId = id.replace(/^(?:ncm|netease)\//i, "");
    return /^\d+$/.test(songId) ? `https://music.163.com/song?id=${songId}` : null;
  }

  if (kind === "qq") {
    // QQ song pages are keyed by the MID (alphanumeric), but some entries only
    // carry the numeric id — both work with the `songDetail` route.
    const mid = id.replace(/^qq\//i, "");
    return mid ? `https://y.qq.com/n/ryqq/songDetail/${encodeURIComponent(mid)}` : null;
  }

  return null;
}

/** Platform ids parsed from a user-supplied id or song URL. */
export interface ParsedLyricIds {
  netease?: string;
  qqMid?: string;
  qqId?: string;
}

function neteaseIdFrom(value: string): string | null {
  if (/^\d+$/.test(value)) return value;
  // Query strings and SPA hashes, e.g. `song?id=186016` or `#/song?id=186016`.
  const embedded = /[?&#]id=(\d+)/.exec(value);
  if (embedded) return embedded[1];
  try {
    const id = new URL(value).searchParams.get("id");
    if (id && /^\d+$/.test(id)) return id;
  } catch {
    // Not a URL.
  }
  return null;
}

/** Parse a NetEase song URL or bare numeric id. */
export function parseNeteaseId(input: string): string | null {
  const value = input.trim();
  return value ? neteaseIdFrom(value) : null;
}

/** Parse a QQ song URL, bare MID or bare numeric id. */
export function parseQqIds(input: string): ParsedLyricIds | null {
  const value = input.trim();
  if (!value) return null;
  if (!value.includes(":") && !value.includes("/")) {
    return /^\d+$/.test(value) ? { qqId: value } : { qqMid: value };
  }
  const path = /songDetail\/([0-9A-Za-z]+)/.exec(value);
  if (path) return { qqMid: path[1] };
  try {
    const url = new URL(value);
    const mid = url.searchParams.get("songmid");
    if (mid) return { qqMid: mid };
    const id = url.searchParams.get("songid") ?? url.searchParams.get("id");
    if (id && /^\d+$/.test(id)) return { qqId: id };
  } catch {
    // Not a URL.
  }
  return null;
}

/**
 * Parse the NetEase / QQ address required when downloading from AMLL, whose
 * database is matched by platform id rather than an id of its own.
 */
export function parseAmllAddress(input: string): ParsedLyricIds | null {
  const value = input.trim();
  if (!value) return null;
  const netease = neteaseIdFrom(value);
  if (netease) return { netease };
  return parseQqIds(value);
}
