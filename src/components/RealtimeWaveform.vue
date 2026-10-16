<!--
  Copyright 2026 Atemukesu
  SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { spectrumData } from "../lib/audio";
import { showWaveform } from "../lib/waveform";

const props = withDefaults(defineProps<{ barCount?: number; fit?: boolean }>(), {
  barCount: 48,
  fit: false,
});

/** Approximate width (px) of one bar plus its gap when auto-fitting a width. */
const FIT_UNIT = 7;

const root = ref<HTMLElement | null>(null);
const fittedCount = ref(props.barCount);
let observer: ResizeObserver | null = null;

/** Pick a bar count so the bars keep a steady width at any container size. */
function measure() {
  const element = root.value;
  if (!element) return;
  const width = element.clientWidth;
  if (width <= 0) return;
  fittedCount.value = Math.min(512, Math.max(8, Math.round(width / FIT_UNIT)));
}

onMounted(() => {
  if (!props.fit) return;
  measure();
  if (typeof ResizeObserver === "undefined") return;
  observer = new ResizeObserver(measure);
  if (root.value) observer.observe(root.value);
});

onBeforeUnmount(() => {
  observer?.disconnect();
  observer = null;
});

const count = computed(() => (props.fit ? fittedCount.value : Math.max(1, Math.round(props.barCount))));

/** Resample non-DC FFT bins to stable logarithmic frequency bands. */
const bars = computed(() => {
  const barTotal = count.value;
  const source = spectrumData.value?.magnitudes ?? [];
  if (source.length === 0) return Array.from({ length: barTotal }, () => 0.06);
  const firstBin = 1;
  const lastBin = source.length - 1;
  const logRatio = Math.pow(lastBin / firstBin, 1 / barTotal);
  return Array.from({ length: barTotal }, (_, index) => {
    // Bin zero is DC and is removed by the native FFT preprocessing. Logarithmic
    // spacing gives bass frequencies more horizontal room instead of making
    // the spectrum look like all energy is packed into the first bars.
    const start = Math.max(firstBin, Math.floor(firstBin * Math.pow(logRatio, index)));
    const end = Math.min(lastBin + 1, Math.max(start + 1, Math.ceil(firstBin * Math.pow(logRatio, index + 1))));
    const window = source.slice(start, end);
    const linear = Math.max(...window, 0);
    const db = 20 * Math.log10(Math.max(linear, 0.00001));
    return Math.max(0.06, Math.min(1, (db + 60) / 60));
  });
});
</script>

<template>
  <div
    v-if="showWaveform"
    ref="root"
    class="realtime-waveform flex h-8 w-full items-center justify-center gap-[2px]"
    :class="{ 'is-fit': fit }"
    aria-label="Realtime frequency spectrum"
    role="img"
  >
    <i v-for="(bar, index) in bars" :key="index" class="realtime-waveform__bar" :style="{ transform: `scaleY(${bar})` }"></i>
  </div>
</template>

<style scoped>
.realtime-waveform__bar {
  width: 3px;
  height: 100%;
  min-height: 2px;
  border-radius: 999px;
  background: currentColor;
  opacity: 0.72;
  transform-origin: center;
  transition: transform 90ms linear;
}

/* Auto-fitted: the bars share the container width evenly, keeping a small gap.
   The gap is a per-bar trailing margin rather than `gap`, so it survives even
   where flexbox `gap` renders collapsed. */
.realtime-waveform.is-fit {
  gap: 0;
}

.realtime-waveform.is-fit .realtime-waveform__bar {
  flex: 1 1 0;
  width: auto;
  min-width: 1px;
  margin-inline-end: 4px;
}

.realtime-waveform.is-fit .realtime-waveform__bar:last-child {
  margin-inline-end: 0;
}
</style>
