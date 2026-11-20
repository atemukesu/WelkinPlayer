/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { usePlayerStore } from "../stores/player";
import { trackKey } from "../lib/sources";

/**
 * Live state for the compact smart-transition readout. The indicator stays
 * visible while a mix is being prepared or is running, and hides as soon as the
 * incoming deck becomes the current track (the handover point). Seconds count
 * down instead of a percentage so the readout stays quiet.
 */
export function useTransitionIndicator() {
  const { t } = useI18n();
  const player = usePlayerStore();

  const currentKey = computed(() => trackKey(player.currentTrack));
  const active = computed(() => {
    const status = player.transition;
    return !!status && status.to !== currentKey.value;
  });
  const preparing = computed(() => player.transition?.phase === "preparing");
  const label = computed(() =>
    preparing.value ? t("controls.smartTransitionPreparing") : t("controls.smartTransitionMixing"),
  );
  const nextTitle = computed(() => player.transitionNext?.title ?? "");
  const secondsLeft = computed(() => {
    const status = player.transition;
    if (!status) return 0;
    const remaining = preparing.value
      ? status.startsIn
      : status.blendSecs * (1 - status.progress);
    return Math.max(1, Math.ceil(remaining));
  });

  return { active, label, nextTitle, secondsLeft };
}
