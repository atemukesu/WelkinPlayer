<script setup lang="ts">
import { useI18n } from "vue-i18n";
import FontFamilyList from "./FontFamilyList.vue";
import type { LyricDisplaySettings } from "../lib/profile";

const model = defineModel<LyricDisplaySettings>({ required: true });
const props = defineProps<{ availableFonts: string[]; showSpacing?: boolean }>();
const { t } = useI18n();
</script>

<template>
  <label class="grid gap-2 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">
    <span class="flex items-center justify-between">
      <span>{{ t("settings.lyrics.size") }}</span>
      <output class="font-mono text-accent">{{ model.lineSize }}px</output>
    </span>
    <input
      v-model.number="model.lineSize"
      class="ak-slider"
      type="range"
      min="16"
      max="48"
      step="1"
      :style="{ '--fill': `${((model.lineSize - 16) / 32) * 100}%` }"
    />
  </label>

  <label class="grid gap-2 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">
    <span class="flex items-center justify-between">
      <span>{{ t("settings.lyrics.translationSize") }}</span>
      <output class="font-mono text-accent">{{ model.translationSize }}px</output>
    </span>
    <input
      v-model.number="model.translationSize"
      class="ak-slider"
      type="range"
      min="12"
      max="32"
      step="1"
      :style="{ '--fill': `${((model.translationSize - 12) / 20) * 100}%` }"
    />
  </label>

  <label v-if="props.showSpacing" class="grid gap-2 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">
    <span class="flex items-center justify-between">
      <span>{{ t("settings.lyrics.spacing") }}</span>
      <output class="font-mono text-accent">{{ model.lineSpacing }}px</output>
    </span>
    <input
      v-model.number="model.lineSpacing"
      class="ak-slider"
      type="range"
      min="8"
      max="36"
      step="2"
      :style="{ '--fill': `${((model.lineSpacing - 8) / 28) * 100}%` }"
    />
  </label>

  <label class="grid gap-2 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">
    <span class="flex items-center justify-between">
      <span>{{ t("settings.lyrics.fontWeight") }}</span>
      <output class="font-mono text-accent">{{ model.fontWeight }}</output>
    </span>
    <input
      v-model.number="model.fontWeight"
      class="ak-slider"
      type="range"
      min="100"
      max="900"
      step="100"
      :style="{ '--fill': `${((model.fontWeight - 100) / 800) * 100}%` }"
    />
  </label>

  <div class="grid gap-2 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">
    <span>{{ t("settings.lyrics.fontFamilies") }}</span>
    <p class="text-[11px] font-normal normal-case tracking-normal text-dim">{{ t("settings.lyrics.fontOrderHint") }}</p>
    <FontFamilyList v-model="model.fontFamilies" :available="props.availableFonts" />
  </div>

  <label class="flex items-center justify-between gap-4">
    <span class="text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("settings.lyrics.translate") }}</span>
    <input v-model="model.translate" class="ak-switch" type="checkbox" />
  </label>
</template>
