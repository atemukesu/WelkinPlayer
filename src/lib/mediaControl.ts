/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

import { effectScope, watch } from "vue";
import type { EffectScope } from "vue";
import { invoke } from "../api";
import { currentTime } from "./audio";
import { trackKey } from "./sources";
import { usePlayerStore } from "../stores/player";

/**
 * Rust owns transport commands and periodic reporting. The webview still sends
 * a full snapshot when lazy metadata arrives: the Rust event thread can only
 * read metadata already present in the disk cache, while this result is the
 * first moment at which the title/cover is known to the UI.
 */
let running = false;
let scope: EffectScope | null = null;

function pushFull() {
  const player = usePlayerStore();
  const track = player.currentTrack;
  if (!track) {
    void invoke("media_control_clear").catch(() => {});
    return;
  }
  void invoke("media_control_update", {
    payload: {
      title: track.title ?? "",
      artist: track.artist ?? "",
      album: track.album ?? "",
      durationMs: Math.round((player.duration > 0 ? player.duration : 0) * 1000),
      positionMs: Math.round(currentTime() * 1000),
      playing: player.isPlaying,
      volume: player.volume,
      coverHash: track.coverHash ?? null,
    },
  }).catch(() => {});
}

/** Start mirroring playback to the OS media controls. */
export function startMediaControl() {
  if (running) return;
  running = true;
  scope = effectScope();
  scope.run(() => {
    watch(
      () => {
        const player = usePlayerStore();
        const track = player.currentTrack;
        return [
          trackKey(track),
          track?.title ?? "",
          track?.artist ?? "",
          track?.album ?? "",
          track?.coverHash ?? "",
          player.duration,
          player.isPlaying,
        ].join("\u001f");
      },
      pushFull,
      { immediate: true },
    );
  });
}

/** Stop mirroring playback to the OS media controls. */
export function stopMediaControl() {
  if (!running) return;
  running = false;
  scope?.stop();
  scope = null;
  void invoke("media_control_clear").catch(() => {});
}
