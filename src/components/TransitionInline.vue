<!--
  Copyright 2026 Atemukesu
  SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { ref } from "vue";
import { Sparkles } from "@lucide/vue";
import { useTransitionIndicator } from "../composables/useTransitionIndicator";

/**
 * The only on-screen trace of a smart transition: a tiny sparkle, the incoming
 * title (truncated to whatever room is left) and the seconds remaining. It sits
 * inline beside the artist and deliberately has no frame, background, shimmer
 * or progress line.
 */
const { active, label, nextTitle, secondsLeft } = useTransitionIndicator();
const titleEl = ref<HTMLElement | null>(null);

/** Let the player views capture the title's position for the handover flight. */
function getTitleElement() {
  return titleEl.value;
}

defineExpose({ getTitleElement });
</script>

<template>
  <Transition name="transition-inline">
    <span
      v-if="active"
      class="transition-inline inline-flex min-w-0 items-center gap-1.5"
      role="status"
      aria-live="off"
      :aria-label="label"
      :title="label"
    >
      <Sparkles :size="12" :stroke-width="2" class="shrink-0 opacity-60" aria-hidden="true" />
      <span ref="titleEl" class="min-w-0 truncate">{{ nextTitle }}</span>
      <span class="shrink-0 font-mono tabular-nums opacity-60">{{ secondsLeft }}s</span>
    </span>
  </Transition>
</template>

<style scoped>
.transition-inline-enter-active {
  transition: opacity 160ms ease;
}

/* The handover flight takes over from the same spot, so the inline copy must
 * not linger while the clone is already moving. */
.transition-inline-leave-active {
  transition: none;
}

.transition-inline-enter-from,
.transition-inline-leave-to {
  opacity: 0;
}

@media (prefers-reduced-motion: reduce) {
  .transition-inline-enter-active {
    transition: none;
  }
}
</style>
