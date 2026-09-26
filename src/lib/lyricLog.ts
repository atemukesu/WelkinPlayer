//! Diagnostics for the lyric pipeline.
//!
//! Every line goes to the webview console **and** to the Rust log via the
//! `log_message` command, so lyric activity shows up on the same stderr stream
//! as the backend while running `tauri dev` (the webview console is easy to
//! miss when the app is windowed).

import { invoke } from "../api";

export type LogLevel = "info" | "warn" | "error";

/** Render whatever was passed as `details` into a compact suffix. */
function describe(details: unknown): string {
  if (details === undefined || details === null) return "";
  if (details instanceof Error) return ` ${details.name}: ${details.message}`;
  if (typeof details === "string") return ` ${details}`;
  try {
    return ` ${JSON.stringify(details)}`;
  } catch {
    return ` ${String(details)}`;
  }
}

/**
 * Emit one lyric-pipeline event.
 *
 * @param level  `info` for normal flow, `warn` for graceful fallbacks,
 *               `error` for failures that change the outcome.
 * @param message short human-readable step description.
 * @param details optional object/string/Error appended to the line.
 */
export function lyricLog(level: LogLevel, message: string, details?: unknown): void {
  const line = `[lyrics] ${message}${describe(details)}`;

  if (level === "error") console.error(line);
  else if (level === "warn") console.warn(line);
  else console.log(line);

  // Mirror to Rust; never let a logging failure break the pipeline.
  void invoke("log_message", { level, message: line }).catch(() => {});
}

/** Milliseconds since the pipeline started, for step timings. */
export function since(start: number): string {
  return `${Math.round(performance.now() - start)}ms`;
}
