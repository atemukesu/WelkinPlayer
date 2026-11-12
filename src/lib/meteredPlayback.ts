/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

import { ref } from "vue";

/** The user's answer to the metered-playback prompt. */
export type MeteredPlaybackChoice = "all" | "cached" | "cancel";

/** Counts shown by the prompt. `offline` tracks are cached or local. */
export interface MeteredPlaybackRequest {
  total: number;
  offline: number;
}

/**
 * Pending prompt, or `null` when nothing is being asked. The modal subscribes
 * to this ref; `player.playInQueue` opens it before starting a cloud queue on a
 * metered connection and waits for the answer.
 */
export const meteredPlaybackRequest = ref<MeteredPlaybackRequest | null>(null);

let resolvePending: ((choice: MeteredPlaybackChoice) => void) | null = null;

/**
 * Ask the user how to proceed with a cloud queue on mobile data. A second call
 * while a prompt is open cancels the previous one, so a double tap cannot leave
 * a dangling promise.
 */
export function requestMeteredPlayback(request: MeteredPlaybackRequest): Promise<MeteredPlaybackChoice> {
  resolvePending?.("cancel");
  meteredPlaybackRequest.value = request;
  return new Promise((resolve) => {
    resolvePending = (choice) => {
      resolvePending = null;
      meteredPlaybackRequest.value = null;
      resolve(choice);
    };
  });
}

/** Settle the open prompt; called by the modal's buttons. */
export function resolveMeteredPlayback(choice: MeteredPlaybackChoice) {
  resolvePending?.(choice);
}
