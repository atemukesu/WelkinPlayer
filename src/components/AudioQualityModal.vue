<!--
 Copyright 2026 Atemukesu
 SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { X } from "@lucide/vue";
import { usePlayerStore } from "../stores/player";
import * as audioQuality from "../lib/audioQuality";

const { t } = useI18n();
const player = usePlayerStore();

/** The pinned output rate in Hz, chosen by the user. */
const chosen = computed(() => audioQuality.preferredRate.value);

/**
 * Every rate the engine knows how to run at. Rates the device does not report
 * are marked as such — picking one just warns, since the OS will resample.
 */
const STANDARD_RATES = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000];

const options = computed(() => {
  const fileRate = player.sampleRate || 0;
  const cap = fileRate > 0 ? fileRate : STANDARD_RATES[STANDARD_RATES.length - 1];
  const supported = audioQuality.deviceInfo.value?.supportedRates ?? [];
  return STANDARD_RATES
    .filter((rate) => rate <= cap)
    .map((rate) => ({
      value: rate,
      label: audioQuality.formatRate(rate),
      unsupported: supported.length > 0 && !supported.includes(rate),
    }));
});

/** The output stream's rate — what the picker actually switches. */
const currentText = computed(() => audioQuality.formatRate(player.outputRate || player.workRate || player.sampleRate));

function select(rate: number) {
  void audioQuality.setPreferredRate(rate);
}
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
