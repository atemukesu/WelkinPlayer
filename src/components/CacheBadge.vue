<!--
 Copyright 2026 Atemukesu
 SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { computed } from "vue";
import { CircleCheck } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { useCacheStore } from "../stores/cache";

const props = withDefaults(defineProps<{ cacheKey?: string | null; size?: number }>(), { cacheKey: null, size: 14 });
const { t } = useI18n();
const cache = useCacheStore();

const cached = computed(() => cache.isCached(props.cacheKey ?? undefined));
const item = computed(() => cache.itemFor(props.cacheKey ?? undefined));
const downloading = computed(() => !cached.value && !!item.value?.pinned && !item.value.complete);
const progress = computed(() => cache.progressFor(props.cacheKey ?? undefined));
/** Unknown size (or freshly queued) means there is no honest percentage to show yet. */
const indeterminate = computed(() => progress.value === null || progress.value <= 0);
const radius = 8;
const circumference = 2 * Math.PI * radius;
const dashOffset = computed(() => (progress.value === null ? circumference : circumference * (1 - progress.value)));
const label = computed(() => {
  if (cached.value) return t("library.cached");
  if (!downloading.value) return "";
  return progress.value === null
    ? t("downloads.waiting")
    : t("downloads.progressLabel", { percent: Math.round(progress.value * 100) });
});
</script>

<template>
  <CircleCheck v-if="cached" :size="size" class="shrink-0 text-accent" :aria-label="label" />
  <span v-else-if="downloading" class="inline-grid shrink-0 place-items-center text-accent" :title="label" :aria-label="label" role="img">
    <!-- Queued / size still unknown: spin the whole SVG so the arc keeps a stable centre. -->
    <svg v-if="indeterminate" :width="size" :height="size" viewBox="0 0 20 20" fill="none" class="animate-spin" aria-hidden="true">
      <circle cx="10" cy="10" :r="radius" stroke="currentColor" stroke-width="2" opacity="0.22" />
      <circle cx="10" cy="10" :r="radius" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-dasharray="16 34" />
    </svg>
    <!-- Real progress: a determinate ring that fills smoothly as bytes arrive. -->
    <svg v-else :width="size" :height="size" viewBox="0 0 20 20" fill="none" aria-hidden="true">
      <circle cx="10" cy="10" :r="radius" stroke="currentColor" stroke-width="2" opacity="0.22" />
      <circle
        cx="10"
        cy="10"
        :r="radius"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        :stroke-dasharray="circumference"
        :stroke-dashoffset="dashOffset"
        transform="rotate(-90 10 10)"
        class="transition-[stroke-dashoffset] duration-500 ease-out"
      />
    </svg>
  </span>
</template>
