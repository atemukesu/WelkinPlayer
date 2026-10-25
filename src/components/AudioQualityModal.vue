<!--
 Copyright 2026 Atemukesu
 SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { computed, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { X } from "@lucide/vue";
import { usePlayerStore } from "../stores/player";
import * as audioQuality from "../lib/audioQuality";

const { t } = useI18n();
const player = usePlayerStore();

/** `0` = follow the device; otherwise the pinned output rate in Hz. */
const chosen = computed(() => audioQuality.preferredRate.value);

/**
 * Every rate the engine knows how to run at. All of them stay selectable even
 * when the device does not report them — picking one just warns, since the OS
 * will resample. Rates the device does not list are marked as such.
 */
const STANDARD_RATES = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000];

const options = computed(() => {
  const reported = new Set(audioQuality.deviceInfo.value?.supportedRates ?? []);
  return STANDARD_RATES.map((rate) => ({
    value: rate,
    label: audioQuality.formatRate(rate),
    unsupported: reported.size > 0 && !reported.has(rate),
  }));
});

/** The output stream's rate — what the picker actually switches. */
const currentText = computed(() => audioQuality.formatRate(player.outputRate || player.workRate || player.sampleRate));

/** "Device max" line, from the corrected (plugin-bounded) probe. */
const maxText = computed(() => {
  const max = audioQuality.deviceInfo.value?.maxRate ?? 0;
  return max > 0 ? t("audioQuality.deviceMax", { max: audioQuality.formatRate(max) }) : "";
});

const tooHigh = computed(() => chosen.value > 0 && audioQuality.rateTooHigh.value);

function select(rate: number) {
  void audioQuality.setPreferredRate(rate);
}

onMounted(() => {
  void audioQuality.refreshDeviceInfo();
});
</script>

<template>
  <Teleport to="body">
    <Transition name="modal">
      <div v-if="audioQuality.qualityModalOpen.value" class="fixed inset-0 z-[80] grid place-items-center bg-black/70 p-4" role="dialog" aria-modal="true" @pointerdown.self="audioQuality.qualityModalOpen.value = false">
        <div class="ak-frame w-full max-w-md border border-line bg-surface p-5">
          <div class="flex items-center justify-between">
            <h3 class="text-sm font-bold uppercase tracking-[0.2em]">{{ t("audioQuality.title") }}</h3>
            <button type="button" class="grid h-8 w-8 place-items-center text-dim hover:text-fg" :aria-label="t('controls.close')" @click="audioQuality.qualityModalOpen.value = false"><X :size="16" /></button>
          </div>

          <div class="mt-3 flex flex-wrap items-baseline gap-x-2 gap-y-1">
            <span class="text-2xl font-black tabular-nums text-accent">{{ currentText }}</span>
            <span class="text-[11px] text-dim">{{ t("audioQuality.current") }}</span>
          </div>
          <p v-if="maxText" class="mt-1 text-[11px] text-dim">{{ maxText }}</p>

          <div class="mt-4 grid max-h-[50vh] gap-1.5 overflow-y-auto">
            <button
              v-for="option in options"
              :key="option.value"
              type="button"
              class="flex h-10 items-center justify-between border px-3 text-[13px] font-semibold tracking-wide transition-colors"
              :class="chosen === option.value ? 'border-accent text-accent' : 'border-line text-muted hover:border-accent hover:text-fg'"
              :aria-pressed="chosen === option.value"
              @click="select(option.value)"
            >
              <span class="flex items-baseline gap-2">
                {{ option.label }}
                <span v-if="option.unsupported" class="text-[10px] font-normal uppercase tracking-[0.15em] text-amber-500">{{ t("audioQuality.unsupported") }}</span>
              </span>
              <span v-if="chosen === option.value" class="text-[10px] uppercase tracking-[0.2em]">{{ t("audioQuality.active") }}</span>
            </button>
          </div>

          <p v-if="tooHigh" class="mt-4 border border-amber-500/60 bg-amber-500/10 p-3 text-[12px] leading-relaxed text-amber-500">
            {{ t("audioQuality.tooHigh") }}
          </p>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.modal-enter-active,
.modal-leave-active {
  transition: opacity 220ms ease;
}

.modal-enter-from,
.modal-leave-to {
  opacity: 0;
}

.modal-enter-active .ak-frame,
.modal-leave-active .ak-frame {
  transition: transform 260ms cubic-bezier(0.2, 0.8, 0.2, 1), opacity 260ms cubic-bezier(0.2, 0.8, 0.2, 1);
}

.modal-enter-from .ak-frame,
.modal-leave-to .ak-frame {
  opacity: 0;
  transform: translateY(12px) scale(0.97);
}
</style>
