/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

/**
 * Smart-transition documents.
 *
 * Each analysed playlist owns one small JSON file on WebDAV (and in the local
 * cache). The heavy per-track features stay in that file; the profile only
 * keeps a pointer and a status badge. This module is the typed bridge to the
 * Rust commands that produce and read those documents.
 */

import { invoke } from "../api";

/** Mirrors Rust `TrackInput`. */
export interface SmartTrackInput {
  /** `sourceId::path`. */
  key: string;
  title: string;
}

/** Mirrors Rust `TransitionPreview`. */
export interface TransitionPreview {
  from: string;
  to: string;
  fromTitle: string;
  toTitle: string;
  /** Absolute seconds in the outgoing track where the fade starts. */
  outAt: number;
  /** Entry position in the incoming track. */
  inAt: number;
  /** Fade length in seconds. */
  blendSecs: number;
  mode: "strict" | "relaxed" | "fallback";
  score: number;
}

export interface AnalysisStats {
  analyzed: number;
  skipped: number;
  reused: number;
  lowConfidence: number;
}

/** Mirrors Rust `AnalysisSummary`. */
export interface SmartTransitionSummary {
  playlistId: string;
  file: string;
  revision: number;
  updatedAt: number;
  order: string[];
  previousOrder: string[];
  transitions: TransitionPreview[];
  stats: AnalysisStats;
  /** `remote`, `local` or `none`. */
  source: string;
}

/** Mirrors Rust `TransitionPlan`, used by the playback engine. */
export interface PlaybackTransition {
  from: string;
  to: string;
  outAt: number;
  inAt: number;
  blendSecs: number;
  fadeOutCurve: string;
  fadeInCurve: string;
  mode: string;
  score: number;
}

export interface AnalysisProgress {
  playlistId: string;
  done: number;
  total: number;
  phase: "analyzing" | "planning";
}

/** Analyse (or re-order) a playlist. Cached tracks are not decoded again. */
export function analyzePlaylist(
  playlistId: string,
  playlistName: string,
  tracks: SmartTrackInput[],
  options: { previousOrder?: string[]; fixedFirst?: string } = {},
): Promise<SmartTransitionSummary> {
  return invoke("smart_transition_analyze", {
    playlistId,
    playlistName,
    tracks,
    previousOrder: options.previousOrder,
    fixedFirst: options.fixedFirst,
  });
}

/** Read the stored summary (remote copy when it is newer). */
export function loadSmartTransition(playlistId: string): Promise<SmartTransitionSummary | null> {
  return invoke("smart_transition_load", { playlistId });
}

/** Read only the adjacent transition plans for playback scheduling. */
export function loadTransitionPlans(playlistId: string): Promise<PlaybackTransition[]> {
  return invoke("smart_transition_plans", { playlistId });
}

/** Delete a playlist's analysis file locally and remotely. */
export function deleteSmartTransition(playlistId: string): Promise<void> {
  return invoke("smart_transition_delete", { playlistId });
}

/** Ask a running analysis to stop after the current track. */
export function cancelSmartTransition(playlistId: string): Promise<void> {
  return invoke("smart_transition_cancel", { playlistId });
}
