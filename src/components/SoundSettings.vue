<!--
 Copyright 2026 Atemukesu
 SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { computed, onBeforeUnmount, reactive, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { ChevronDown, RotateCcw, SlidersHorizontal, Waves, X } from "@lucide/vue";
import Knob from "./Knob.vue";
import LayeredSelect from "./LayeredSelect.vue";
import VerticalFader from "./VerticalFader.vue";
import * as eq from "../lib/equalizer";
import { DEFAULT_EFFECTS, commitEffects, fx, FX_LIMITS, resetEffects } from "../lib/effects";
import type { FxKey } from "../lib/effects";

const { t } = useI18n();

/** Category tabs inside the modal, mirroring the settings page's tab bar.
 * Only the equalizer is implemented; the rest track the playback roadmap. */
const tabItems = [
  { key: "equalizer", labelKey: "soundSettings.tabs.equalizer", icon: SlidersHorizontal },
  { key: "effects", labelKey: "soundSettings.tabs.effects", icon: Waves },
] as const;
type SoundTab = (typeof tabItems)[number]["key"];
const activeTab = ref<SoundTab>("equalizer");

/** One knob on an effect card: UI range, factory value and live binding. */
interface FxParam {
  labelKey: string;
  min: number;
  max: number;
  step: number;
  unit: string;
  /** Factory value, drawn as a tick on the knob and restored on double-click. */
  origin: number;
  get: () => number;
  set: (value: number) => void;
}

interface FxRow {
  key: FxKey;
  params: FxParam[];
}

type FxGroupKey = "time" | "modulation" | "space";

interface FxGroup {
  key: FxGroupKey;
  rows: FxRow[];
}

/** A millisecond knob in the shared 20–1000 ms range. */
function timeMs(origin: number, get: () => number, set: (value: number) => void): FxParam {
  return { labelKey: "soundSettings.effects.time", ...FX_LIMITS.timeMs, origin, get, set };
}

/** A percentage knob; `limit` names the field and its 0–100 (or 0–85) range. */
function percent(
  limit: "mix" | "feedback" | "depth" | "size" | "damping",
  origin: number,
  get: () => number,
  set: (value: number) => void,
): FxParam {
  return { labelKey: `soundSettings.effects.${limit}`, ...FX_LIMITS[limit], origin, get, set };
}

/** Effects grouped by what they do to the signal, in chain order. */
const fxGroups: FxGroup[] = [
  {
    key: "time",
    rows: [
      {
        key: "delay",
        params: [
          timeMs(DEFAULT_EFFECTS.delay.timeMs, () => fx.delay.timeMs, (value) => (fx.delay.timeMs = value)),
          percent("mix", DEFAULT_EFFECTS.delay.mix, () => fx.delay.mix, (value) => (fx.delay.mix = value)),
        ],
      },
      {
        key: "echo",
        params: [
          timeMs(DEFAULT_EFFECTS.echo.timeMs, () => fx.echo.timeMs, (value) => (fx.echo.timeMs = value)),
          percent("feedback", DEFAULT_EFFECTS.echo.feedback, () => fx.echo.feedback, (value) => (fx.echo.feedback = value)),
          percent("mix", DEFAULT_EFFECTS.echo.mix, () => fx.echo.mix, (value) => (fx.echo.mix = value)),
        ],
      },
    ],
  },
  {
    key: "modulation",
    rows: [
      {
        key: "chorus",
        params: [
          percent("depth", DEFAULT_EFFECTS.chorus.depth, () => fx.chorus.depth, (value) => (fx.chorus.depth = value)),
          percent("mix", DEFAULT_EFFECTS.chorus.mix, () => fx.chorus.mix, (value) => (fx.chorus.mix = value)),
        ],
      },
      {
        key: "flanger",
        params: [
          percent("depth", DEFAULT_EFFECTS.flanger.depth, () => fx.flanger.depth, (value) => (fx.flanger.depth = value)),
          percent("feedback", DEFAULT_EFFECTS.flanger.feedback, () => fx.flanger.feedback, (value) => (fx.flanger.feedback = value)),
          percent("mix", DEFAULT_EFFECTS.flanger.mix, () => fx.flanger.mix, (value) => (fx.flanger.mix = value)),
        ],
      },
    ],
  },
  {
    key: "space",
    rows: [
      {
        key: "reverb",
        params: [
          percent("size", DEFAULT_EFFECTS.reverb.size, () => fx.reverb.size, (value) => (fx.reverb.size = value)),
          percent("damping", DEFAULT_EFFECTS.reverb.damping, () => fx.reverb.damping, (value) => (fx.reverb.damping = value)),
          percent("mix", DEFAULT_EFFECTS.reverb.mix, () => fx.reverb.mix, (value) => (fx.reverb.mix = value)),
        ],
      },
    ],
  },
];

/** Every group starts expanded; the headers only clear space when wanted. */
const fxOpen = reactive<Record<FxGroupKey, boolean>>({
  time: true,
  modulation: true,
  space: true,
});

const allFxKeys = fxGroups.flatMap((group) => group.rows.map((row) => row.key));

function toggleFxGroup(key: FxGroupKey) {
  fxOpen[key] = !fxOpen[key];
}

function fxGroupName(key: FxGroupKey): string {
  return t(`soundSettings.effects.groups.${key}`);
}

function resetFxGroup(group: FxGroup) {
  resetEffects(group.rows.map((row) => row.key));
}

/** Enable/disable one effect; a switch is a single action, so commit at once. */
function toggleFx(key: FxKey, event: Event) {
  fx[key].enabled = (event.target as HTMLInputElement).checked;
  commitEffects();
}

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

/** Roving-tabindex keyboard support for the category bar. */
function onTabKeydown(event: KeyboardEvent, index: number) {
  const last = tabItems.length - 1;
  let next = index;
  if (event.key === "ArrowRight" || event.key === "ArrowDown") next = index === last ? 0 : index + 1;
  else if (event.key === "ArrowLeft" || event.key === "ArrowUp") next = index === 0 ? last : index - 1;
  else if (event.key === "Home") next = 0;
  else if (event.key === "End") next = last;
  else return;
  event.preventDefault();
  const tab = tabItems[next];
  activeTab.value = tab.key;
  requestAnimationFrame(() => document.getElementById(`sound-tab-${tab.key}`)?.focus());
}

const body = ref<HTMLElement | null>(null);
/** Start every tab at the top instead of inheriting the previous scroll. */
watch(activeTab, () => body.value?.scrollTo({ top: 0 }));

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
          class="ak-frame flex h-[min(88vh,720px)] w-full max-w-3xl flex-col overflow-hidden border border-line bg-surface"
        >
          <div class="flex shrink-0 items-start justify-between gap-4 px-5 py-4">
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

          <div ref="body" class="modal-body flex min-h-0 flex-1 flex-col overflow-y-auto overscroll-contain px-5 pb-4">
            <nav
              class="sticky top-0 z-10 -mx-5 shrink-0 border-b border-line bg-surface/95 px-5 py-2 backdrop-blur"
              role="tablist"
              :aria-label="t('soundSettings.tabs.label')"
            >
              <div class="grid grid-cols-2 gap-1">
                <button
                  v-for="(item, index) in tabItems"
                  :id="`sound-tab-${item.key}`"
                  :key="item.key"
                  type="button"
                  role="tab"
                  class="relative flex h-10 min-w-0 items-center justify-center gap-1.5 border border-transparent px-1 text-[11px] font-bold uppercase tracking-[0.1em] sm:gap-2 sm:px-3 sm:text-[12px] sm:tracking-[0.16em]"
                  :class="activeTab === item.key ? 'ak-select' : 'border-line text-muted hover:bg-fg/5 hover:text-fg'"
                  :aria-selected="activeTab === item.key"
                  :aria-controls="`sound-panel-${item.key}`"
                  :tabindex="activeTab === item.key ? 0 : -1"
                  @click="activeTab = item.key"
                  @keydown="onTabKeydown($event, index)"
                >
                  <component :is="item.icon" class="hidden shrink-0 sm:block" :size="14" :stroke-width="2" />
                  <span class="truncate">{{ t(item.labelKey) }}</span>
                </button>
              </div>
            </nav>

            <div class="flex flex-1 items-center pt-4">
              <div
                v-show="activeTab === 'equalizer'"
                id="sound-panel-equalizer"
                role="tabpanel"
                aria-labelledby="sound-tab-equalizer"
                class="w-full"
              >
                <div class="flex items-center justify-between gap-3">
                  <span class="text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">
                    {{ t("soundSettings.enable") }}
                  </span>
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

              <div
                v-show="activeTab === 'effects'"
                id="sound-panel-effects"
                role="tabpanel"
                aria-labelledby="sound-tab-effects"
                class="w-full self-start"
              >
                <div class="grid gap-4">
                  <div class="flex items-center justify-end gap-3">
                    <button
                      type="button"
                      class="flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-[0.15em] text-muted transition-colors hover:text-fg"
                      @click="resetEffects(allFxKeys)"
                    >
                      <RotateCcw :size="12" :stroke-width="1.8" />
                      {{ t("soundSettings.effects.resetAll") }}
                    </button>
                  </div>

                  <section v-for="group in fxGroups" :key="group.key" class="grid gap-2">
                    <header class="flex items-center justify-between gap-2">
                      <button
                        type="button"
                        class="flex min-w-0 items-center gap-1.5 text-[11px] font-bold uppercase tracking-[0.2em] text-muted transition-colors hover:text-fg"
                        :aria-expanded="fxOpen[group.key]"
                        @click="toggleFxGroup(group.key)"
                      >
                        <ChevronDown
                          :size="13"
                          :stroke-width="2"
                          class="shrink-0 text-dim transition-transform"
                          :class="fxOpen[group.key] ? 'rotate-180' : ''"
                        />
                        <span class="truncate">{{ fxGroupName(group.key) }}</span>
                      </button>
                      <button
                        type="button"
                        class="grid h-6 w-6 place-items-center text-dim transition-colors hover:text-fg"
                        :aria-label="`${t('soundSettings.reset')} ${fxGroupName(group.key)}`"
                        @click="resetFxGroup(group)"
                      >
                        <RotateCcw :size="12" :stroke-width="1.8" />
                      </button>
                    </header>

                    <div v-show="fxOpen[group.key]" class="grid gap-2 sm:grid-cols-2">
                      <article
                        v-for="row in group.rows"
                        :key="row.key"
                        class="border border-line px-3.5 py-3"
                        :class="group.rows.length === 1 ? 'sm:col-span-2' : ''"
                      >
                        <header class="flex items-center justify-between gap-2">
                          <span
                            class="truncate text-[12px] font-semibold uppercase tracking-[0.16em] transition-colors"
                            :class="fx[row.key].enabled ? 'text-fg' : 'text-muted'"
                          >
                            {{ t(`soundSettings.effects.list.${row.key}`) }}
                          </span>
                          <div class="flex shrink-0 items-center gap-1.5">
                            <button
                              type="button"
                              class="grid h-6 w-6 place-items-center text-dim transition-colors hover:text-fg"
                              :aria-label="`${t('soundSettings.reset')} ${t(`soundSettings.effects.list.${row.key}`)}`"
                              @click="resetEffects([row.key])"
                            >
                              <RotateCcw :size="11" :stroke-width="1.8" />
                            </button>
                            <input
                              :checked="fx[row.key].enabled"
                              class="ak-switch shrink-0"
                              type="checkbox"
                              :aria-label="t(`soundSettings.effects.list.${row.key}`)"
                              @change="toggleFx(row.key, $event)"
                            />
                          </div>
                        </header>

                        <div
                          class="mt-3 flex flex-wrap items-start gap-x-4 gap-y-2"
                          :class="fx[row.key].enabled ? '' : 'opacity-45'"
                        >
                          <Knob
                            v-for="param in row.params"
                            :key="param.labelKey"
                            :model-value="param.get()"
                            :min="param.min"
                            :max="param.max"
                            :step="param.step"
                            :origin="param.origin"
                            :unit="param.unit"
                            :label="t(param.labelKey)"
                            :hint="t('soundSettings.effects.homeHint')"
                            :disabled="!fx[row.key].enabled"
                            @update:model-value="param.set($event)"
                            @commit="commitEffects()"
                          />
                        </div>
                      </article>
                    </div>
                  </section>
                </div>
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
