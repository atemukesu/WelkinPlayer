/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

import { nextTick, onBeforeUnmount, watch } from "vue";
import type { Track, TransitionStatus } from "../stores/player";
import { trackKey } from "../lib/sources";
import { animateTitleFlight, captureTitleFlight } from "../lib/titleFlight";

export interface TitleFlightTargets {
  /** Inline next-title element beside the artist, before the track swaps. */
  inline: () => HTMLElement | null | undefined;
  /** The real title element that the inline text should land on. */
  title: () => HTMLElement | null | undefined;
}

/** How long the incoming key is remembered after the last transition tick. */
const HANDOVER_WINDOW_MS = 3000;

/**
 * Watches the handover from the outgoing track to the incoming one and flies
 * the inline next-title into the real title. The transition target is
 * remembered independently of `player.transition` because the engine may clear
 * the transition status slightly before or after the track snapshot changes.
 */
export function useTransitionTitleFlight(
  track: () => Track | undefined | null,
  transition: () => TransitionStatus | null | undefined,
  targets: TitleFlightTargets,
) {
  let pendingKey: string | null = null;
  let pendingAt = 0;

  // Refresh the timestamp on *every* status tick, not only when the target key
  // changes: a mix can be prepared 8–20s before it hands over, so a window
  // measured from the first event would always be stale by the handover.
  watch(
    transition,
    (status) => {
      if (!status?.to) return;
      pendingKey = status.to;
      pendingAt = performance.now();
    },
    { flush: "sync" },
  );

  watch(
    track,
    (next) => {
      if (!next) return;
      const key = trackKey(next);
      if (!key || key !== pendingKey || performance.now() - pendingAt > HANDOVER_WINDOW_MS) return;
      // Capture before Vue patches the DOM: the inline element is about to be
      // unmounted and its title is replaced by the incoming one.
      const snapshot = captureTitleFlight(targets.inline());
      pendingKey = null;
      if (!snapshot) return;
      void nextTick(() => {
        animateTitleFlight(snapshot, targets.title());
      });
    },
    { flush: "pre" },
  );

  onBeforeUnmount(() => {
    pendingKey = null;
  });
}
