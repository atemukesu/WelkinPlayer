<!--
 Copyright 2026 Atemukesu
 SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { computed, onBeforeUnmount, watch } from "vue";
import { useI18n } from "vue-i18n";
import { RotateCcw, X } from "@lucide/vue";
import LayeredSelect from "./LayeredSelect.vue";
import VerticalFader from "./VerticalFader.vue";
import * as eq from "../lib/equalizer";

const { t } = useI18n();

const presetOptions = computed(() => [
  ...eq.PRESETS.map((preset) => ({
    value: preset.id,
    label: t(`soundSettings.presets.${preset.id}`),
  })),
  { value: "custom", label: t("soundSettings.custom") },
]);

const preset = computed({
  get: () => eq.activePreset.value,
  set: (value: string) => eq.applyPreset(value),
});

/** Fill percentage for the pre-amp rail, drawn from the left edge. */
function preampFill(value: number): string {
  const span = eq.MAX_PREAMP - eq.MIN_PREAMP;
  return `${((value - eq.MIN_PREAMP) / span) * 100}%`;
}

/** Toggle the EQ and apply it at once (single action, no drag to wait for). */
function onToggle(event: Event) {
  eq.eqEnabled.value = (event.target as HTMLInputElement).checked;
  eq.commitEqualizer();
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === "Escape" && eq.eqWindowOpen.value) eq.eqWindowOpen.value = false;
}

watch(
  () => eq.eqWindowOpen.value,
  (open) => {
    if (open) window.addEventListener("keydown", onKeydown);
    else window.removeEventListener("keydown", onKeydown);
  },
  { immediate: true },
);

onBeforeUnmount(() => window.removeEventListener("keydown", onKeydown));
</script>

<template>
  <Teleport to="body">
    <Transition name="modal">
      <div
        v-if="eq.eqWindowOpen.value"
        class="fixed inset-0 z-[80] grid place-items-center bg-black/70 p-3 sm:p-4"
        role="dialog"
        aria-modal="true"
        :aria-label="t('soundSettings.title')"
        @pointerdown.self="eq.eqWindowOpen.value = false"
      >
        <div
          class="ak-frame flex max-h-[90vh] w-full max-w-3xl flex-col overflow-hidden border border-line bg-surface"
        >
          <div class="flex shrink-0 items-start justify-between gap-4 border-b border-line px-5 py-4">
            <h3 class="text-sm font-bold uppercase tracking-[0.2em]">{{ t("soundSettings.title") }}</h3>
            <button
              type="button"
              class="grid h-8 w-8 place-items-center text-dim transition-colors hover:text-fg"
              :aria-label="t('controls.close')"
              @click="eq.eqWindowOpen.value = false"
            >
              <X :size="16" :stroke-width="1.8" />
            </button>
          </div>

          <div class="modal-body min-h-0 flex-1 overflow-y-auto overscroll-contain px-5 py-4">
            <div class="flex items-center justify-between gap-3">
              <h4 class="text-[12px] font-bold uppercase tracking-[0.2em] text-muted">
                {{ t("soundSettings.equalizer") }}
              </h4>
              <input
                :checked="eq.eqEnabled.value"
                class="ak-switch shrink-0"
                type="checkbox"
                :aria-label="t('soundSettings.enable')"
                @change="onToggle"
              />
            </div>

            <div class="mt-3 flex items-center gap-2">
              <LayeredSelect
                v-model="preset"
                :options="presetOptions"
                :label="t('soundSettings.preset')"
                class="min-w-0 flex-1"
              />
              <button
                type="button"
                class="flex h-10 shrink-0 items-center gap-1.5 border border-line px-3 text-[12px] font-semibold text-muted transition-colors hover:border-accent hover:text-fg"
                @click="eq.resetEqualizer()"
              >
                <RotateCcw :size="14" :stroke-width="1.8" />
                {{ t("soundSettings.reset") }}
              </button>
            </div>

            <div
              class="mt-4 grid grid-cols-6 gap-x-1 gap-y-6 border border-line bg-bg/40 px-3 py-5 sm:grid-cols-12"
              :class="eq.eqEnabled.value ? '' : 'opacity-45'"
            >
              <div
                v-for="(gain, index) in eq.eqGains.value"
                :key="eq.BANDS[index]"
                class="flex flex-col items-center gap-2"
              >
                <span
                  class="font-mono text-[10px] tabular-nums"
                  :class="gain === 0 ? 'text-dim' : 'text-accent'"
                >
                  {{ eq.formatGain(gain) }}
                </span>
                <VerticalFader
                  :model-value="gain"
                  :min="eq.MIN_GAIN"
                  :max="eq.MAX_GAIN"
                  :step="1"
                  :label="`${eq.formatBand(eq.BANDS[index])} Hz`"
                  @update:model-value="eq.eqGains.value[index] = $event"
                  @commit="eq.commitEqualizer()"
                />
                <span class="font-mono text-[9px] uppercase tracking-wider text-dim">
                  {{ eq.formatBand(eq.BANDS[index]) }}
                </span>
              </div>
            </div>

            <p class="mt-2 text-right font-mono text-[10px] uppercase tracking-[0.15em] text-dim">
              {{ t("soundSettings.rangeHint", { min: eq.MIN_GAIN, max: `+${eq.MAX_GAIN}` }) }}
            </p>

            <div class="mt-5 grid gap-2">
              <div class="flex items-baseline justify-between gap-3">
                <h4 class="text-[12px] font-bold uppercase tracking-[0.2em] text-muted">
                  {{ t("soundSettings.preamp") }}
                </h4>
                <span class="font-mono text-[11px] tabular-nums text-accent">
                  {{ eq.formatGain(eq.eqPreamp.value) }} dB
                </span>
              </div>
              <div class="flex h-4 items-center">
                <input
                  v-model.number="eq.eqPreamp.value"
                  :style="{ '--fill': preampFill(eq.eqPreamp.value) }"
                  class="ak-slider w-full"
                  type="range"
                  :min="eq.MIN_PREAMP"
                  :max="eq.MAX_PREAMP"
                  step="1"
                  :aria-label="t('soundSettings.preamp')"
                  @change="eq.commitEqualizer()"
                />
              </div>
            </div>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
/* Reserve the scrollbar gutter so showing/hiding it never shifts the panel or
   makes the thumb pop in and out while a fader is dragged. */
.modal-body {
  scrollbar-gutter: stable;
}

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
