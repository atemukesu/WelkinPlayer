/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

//! Read and write the lyric timing offset.
//!
//! The offset is kept as format-native metadata — an `[offset:<ms>]` header line
//! for LRC-family payloads and an `<amll:meta key="offset">` entry for TTML — so
//! the individual timestamps stay untouched and the value remains a single
//! editable number.
//!
//! Sign convention matches `lyric-kit`: a positive offset is added to every
//! timestamp, which delays the lyrics; a negative one advances them.

import { detectFormat } from "lyric-kit";
import type { LyricLine } from "lyric-kit";

const OFFSET_KEY = "offset";
/** `[offset:120]` / `[offset: -120]` header line. */
const LINE_TAG_RE = /^[ \t]*\[offset:\s*(-?\d+)\s*\][ \t]*$/i;
/** `<amll:meta key="offset" value="120"/>`. */
const TTML_TAG_RE = /<amll:meta\s+key="offset"\s+value="([^"]*)"/i;
const LINE_FORMATS: ReadonlySet<string> = new Set(["lrc", "qrc", "yrc", "krc", "lys"]);

function formatOf(content: string): string {
  try {
    return detectFormat(content);
  } catch {
    return "";
  }
}

/** Whether the detected format can carry an offset tag. */
export function supportsLyricOffset(format: string): boolean {
  return format === "ttml" || LINE_FORMATS.has(format);
}

function toOffsetMs(value: string): number {
  const parsed = Number.parseInt(value, 10);
  return Number.isFinite(parsed) ? parsed : 0;
}

/** Read the offset in milliseconds; `0` when none is set or unsupported. */
export function readLyricOffset(content: string): number {
  const format = formatOf(content);
  if (!supportsLyricOffset(format)) return 0;
  if (format === "ttml") {
    const match = TTML_TAG_RE.exec(content);
    return match ? toOffsetMs(match[1]) : 0;
  }
  for (const line of content.split(/\r?\n/)) {
    const match = LINE_TAG_RE.exec(line);
    if (match) return toOffsetMs(match[1]);
  }
  return 0;
}

function writeLineBased(content: string, offsetMs: number): string {
  const eol = content.includes("\r\n") ? "\r\n" : "\n";
  const lines = content.split(/\r?\n/).filter((line) => !LINE_TAG_RE.test(line));
  if (offsetMs !== 0) lines.unshift(`[offset:${offsetMs}]`);
  return lines.join(eol);
}

function writeTtml(content: string, offsetMs: number): string {
  const stripped = content.replace(/<amll:meta\s+key="offset"[^>]*\/?>/gi, "");
  if (offsetMs === 0) return stripped;

  const tag = `<amll:meta key="${OFFSET_KEY}" value="${offsetMs}"/>`;
  const metadataOpen = /<metadata\b[^>]*>/.exec(stripped);
  if (metadataOpen) {
    const at = metadataOpen.index + metadataOpen[0].length;
    return stripped.slice(0, at) + tag + stripped.slice(at);
  }
  const headOpen = /<head\b[^>]*>/.exec(stripped);
  if (headOpen) {
    const at = headOpen.index + headOpen[0].length;
    return `${stripped.slice(0, at)}<metadata xmlns="">${tag}</metadata>${stripped.slice(at)}`;
  }
  const ttOpen = /<tt\b[^>]*>/.exec(stripped);
  if (ttOpen) {
    const at = ttOpen.index + ttOpen[0].length;
    return `${stripped.slice(0, at)}<head><metadata xmlns="">${tag}</metadata></head>${stripped.slice(at)}`;
  }

  // Unrecognised shape: leave the payload untouched rather than risk it.
  return content;
}

/**
 * Write (or clear) the offset, returning a new payload. Unsupported formats and
 * unrecognised payloads are returned unchanged.
 */
export function writeLyricOffset(content: string, offsetMs: number): string {
  const ms = Number.isFinite(offsetMs) ? Math.trunc(offsetMs) : 0;
  const format = formatOf(content);
  if (!content.trim() || !supportsLyricOffset(format)) return content;
  if (format === "ttml") return writeTtml(content, ms);
  return writeLineBased(content, ms);
}

/** Shift every line, word and ruby timestamp by `offsetMs` (in place). */
export function applyLyricOffset(lines: LyricLine[], offsetMs: number): void {
  if (!offsetMs) return;
  const shift = (span: { startTime: number; endTime: number }) => {
    span.startTime = Math.max(0, span.startTime + offsetMs);
    span.endTime = Math.max(0, span.endTime + offsetMs);
  };
  for (const line of lines) {
    shift(line);
    for (const word of line.words) {
      shift(word);
      if (word.ruby) for (const ruby of word.ruby) shift(ruby);
    }
  }
}
