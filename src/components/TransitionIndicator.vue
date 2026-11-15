<!--
 Copyright 2026 Atemukesu
 SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { Sparkles } from "@lucide/vue";
import { usePlayerStore } from "../stores/player";

/**
 * Compact live indicator for a smart transition.
 *
 * The backend only emits while a real overlap is being prepared or mixed, so
 * `player.transition` is `null` outside one. The bar shows the incoming track
 * and the envelope progress; `preparing` shows a countdown instead because the
 * envelope has not started yet.
 */
const props = withDefaults(defineProps<{ tone?: "accent" | "light"; compact?: boolean }>(), {
  tone: "accent",
  compact: false,
});

const { t } = useI18n();
const player = usePlayerStore();

const status = computed(() => player.transition);
const preparing = computed(() => status.value?.phase === "preparing");
const label = computed(() =>
  preparing.value ? t("controls.smartTransitionPreparing") : t("controls.smartTransitionMixing"),
);
const nextTitle = computed(() => player.transitionNext?.title ?? "");
const percent = computed(() => {
  const current = status.value;
  if (!current) return "";
  if (preparing.value) return `~${Math.max(1, Math.ceil(current.startsIn))}s`;
  return `${Math.round(current.progress * 100)}%`;
});
const fill = computed(() => (preparing.value ? 0 : (status.value?.progress ?? 0) * 100));
</script>

<template>
  <Transition name="ak-mix">
    <div
      v-if="status"
      class="ak-mix relative flex min-w-0 items-center gap-1.5 overflow-hidden border px-2 text-[11px] font-semibold"
      :class="[
        tone === 'light' ? 'border-white/40 bg-white/10 text-white' : 'border-accent/50 bg-accent/10 text-accent',
        props.compact ? 'h-7' : 'h-8',
      ]"
      role="status"
      aria-live="off"
    >
      <span
        v-if="preparing"
        class="ak-mix-shimmer pointer-events-none absolute inset-y-0 w-1/3"
        :class="tone === 'light' ? 'bg-white/15' : 'bg-accent/15'"
      />
      <Sparkles :size="12" :stroke-width="2" class="shrink-0" :class="preparing ? 'animate-pulse' : ''" />
      <span class="shrink-0 uppercase tracking-[0.14em]">{{ label }}</span>
      <span v-if="nextTitle" class="min-w-0 truncate opacity-70">→ {{ nextTitle }}</span>
      <span class="ml-auto shrink-0 pl-1 font-mono tabular-nums opacity-80">{{ percent }}</span>
      <span class="absolute inset-x-0 bottom-0 h-[2px]" :class="tone === 'light' ? 'bg-white/25' : 'bg-accent/20'">
        <span
          class="block h-full transition-[width] duration-150 ease-linear"
          :class="tone === 'light' ? 'bg-white' : 'bg-accent'"
          :style="{ width: `${fill}%` }"
        />
      </span>
    </div>
  </Transition>
</template>

<style scoped>
.ak-mix-enter-active,
.ak-mix-leave-active {
  transition: opacity 200ms ease, transform 200ms ease;
}

.ak-mix-enter-from,
.ak-mix-leave-to {
  opacity: 0;
  transform: translateY(4px);
}

.ak-mix-shimmer {
  animation: ak-mix-shimmer 1.4s linear infinite;
}

@keyframes ak-mix-shimmer {
  from {
    transform: translateX(-120%);
  }
  to {
    transform: translateX(420%);
  }
}

@media (prefers-reduced-motion: reduce) {
  .ak-mix-enter-active,
  .ak-mix-leave-active {
    transition: none;
  }

  .ak-mix-shimmer {
    display: none;
  }
}
</style>
