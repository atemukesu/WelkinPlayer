//! Stamp fetched lyrics with their origin.
//!
//! The tag is written using each format's native metadata syntax, so it is
//! carried into the local cache and the WebDAV sidecar without disturbing
//! playback: `lyric-kit` treats unknown `[key:value]` lines (LRC/QRC) and
//! unknown `<metadata>` children (TTML) as inert metadata that never becomes a
//! displayed lyric line.

import { detectFormat } from "lyric-kit";

/** Where a lyric came from. */
export interface LyricSourceTag {
  /** Provider id: `amll` | `qq` | `netease`. */
  source: string;
  /** Platform id used for the lookup, e.g. `ncm/186016` or `qq/97773`. */
  sourceId?: string;
  /** Fetch date (`YYYY-MM-DD`); defaults to today. */
  fetched?: string;
}

/** Metadata keys owned by this app. */
const SOURCE_KEY = "source";
const SOURCE_ID_KEY = "sourceId";
const FETCHED_KEY = "fetched";
const OWNED_KEYS = [SOURCE_KEY, SOURCE_ID_KEY, FETCHED_KEY];

function today(): string {
  const now = new Date();
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

/** Ordered metadata entries for one fetch. */
function entries(tag: LyricSourceTag): Array<[string, string]> {
  const list: Array<[string, string]> = [[SOURCE_KEY, tag.source]];
  if (tag.sourceId) list.push([SOURCE_ID_KEY, tag.sourceId]);
  list.push([FETCHED_KEY, tag.fetched ?? today()]);
  return list;
}

function escapeXml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

/** Drop any tag this module previously added, so re-tagging stays idempotent. */
function stripLineTags(lines: string[]): string[] {
  return lines.filter(
    (line) => !new RegExp(`^\\[(?:${OWNED_KEYS.join("|")}):`, "i").test(line.trim()),
  );
}

function tagLineBased(content: string, tag: LyricSourceTag): string {
  const header = entries(tag)
    .map(([key, value]) => `[${key}:${value}]`)
    .join("\n");
  const body = stripLineTags(content.split("\n")).join("\n");
  return `${header}\n${body}`;
}

function tagTtml(content: string, tag: LyricSourceTag): string {
  const stripped = content.replace(/<amll:meta\s+key="(?:source|sourceId|fetched)"[^>]*\/>/g, "");
  const block = entries(tag)
    .map(([key, value]) => `<amll:meta key="${key}" value="${escapeXml(value)}"/>`)
    .join("");

  // AMLL's own payloads already carry a <head><metadata> block: extend it.
  const metadataOpen = /<metadata\b[^>]*>/.exec(stripped);
  if (metadataOpen) {
    const at = metadataOpen.index + metadataOpen[0].length;
    return stripped.slice(0, at) + block + stripped.slice(at);
  }

  // TTML produced by `lyric-kit`'s `toTTML` has a <head> but no <metadata>.
  const headOpen = /<head\b[^>]*>/.exec(stripped);
  if (headOpen) {
    const at = headOpen.index + headOpen[0].length;
    return `${stripped.slice(0, at)}<metadata xmlns="">${block}</metadata>${stripped.slice(at)}`;
  }

  // …or neither, in which case the metadata block has to be created wholesale.
  const ttOpen = /<tt\b[^>]*>/.exec(stripped);
  if (ttOpen) {
    const at = ttOpen.index + ttOpen[0].length;
    return `${stripped.slice(0, at)}<head><metadata xmlns="">${block}</metadata></head>${stripped.slice(at)}`;
  }

  // Unrecognised shape: leave the payload untouched rather than risk it.
  return content;
}

/**
 * Attach origin metadata to a fetched lyric.
 *
 * Only LRC/QRC (bracket tags) and TTML (`<amll:meta>`) are supported; any other
 * format is returned unchanged.
 */
export function tagLyric(content: string, tag: LyricSourceTag): string {
  const format = detectFormat(content);
  if (format === "lrc" || format === "qrc") return tagLineBased(content, tag);
  if (format === "ttml") return tagTtml(content, tag);
  return content;
}

/** Read back the origin metadata added by {@link tagLyric}, if present. */
export function readLyricSource(content: string): LyricSourceTag | null {
  const bracket = /^\[source:([^\]]*)\]/im.exec(content);
  if (bracket) {
    return {
      source: bracket[1].trim(),
      sourceId: /^\[sourceId:([^\]]*)\]/im.exec(content)?.[1]?.trim(),
      fetched: /^\[fetched:([^\]]*)\]/im.exec(content)?.[1]?.trim(),
    };
  }
  const meta = /<amll:meta\s+key="source"\s+value="([^"]*)"/i.exec(content);
  if (!meta) return null;
  return {
    source: meta[1].trim(),
    sourceId: /<amll:meta\s+key="sourceId"\s+value="([^"]*)"/i.exec(content)?.[1]?.trim(),
    fetched: /<amll:meta\s+key="fetched"\s+value="([^"]*)"/i.exec(content)?.[1]?.trim(),
  };
}
