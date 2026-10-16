<!--
 Copyright 2026 Atemukesu
 SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { AudioLines, AudioWaveform, Disc3, Play } from "@lucide/vue";

/**
 * Visualiser picker.
 *
 * Reached from the sidebar entry between "all tracks" and "artists". Choosing a
 * style and confirming hands it to the full-screen layer in `App.vue`; the choice
 * is remembered so the next visit starts where the last one left off.
 */
const emit = defineEmits<{ start: [style: "mirror" | "bars" | "radial"] }>();
const { t } = useI18n();

const STORAGE_KEY = "welkin-visualizer-style";
type StyleId = "mirror" | "bars" | "radial";

const options: { id: StyleId; labelKey: string; hintKey: string; icon: unknown }[] = [
  { id: "mirror", labelKey: "visualizer.mirror", hintKey: "visualizer.mirrorHint", icon: AudioLines },
  { id: "bars", labelKey: "visualizer.bars", hintKey: "visualizer.barsHint", icon: AudioWaveform },
  { id: "radial", labelKey: "visualizer.radial", hintKey: "visualizer.radialHint", icon: Disc3 },
];

function readStored(): StyleId {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved === "mirror" || saved === "bars" || saved === "radial") return saved;
  } catch {
    /* private mode: fall through to the default */
  }
  return "mirror";
}

const selected = ref<StyleId>(readStored());

function choose(id: StyleId) {
  selected.value = id;
  try {
    localStorage.setItem(STORAGE_KEY, id);
  } catch {
    /* the choice simply does not persist */
  }
}
</script>

<template>
  <section class="mx-auto flex h-full w-full max-w-3xl flex-col gap-6 px-6 py-8">
    <header class="grid gap-2">
      <h1 class="text-2xl font-black tracking-tight">{{ t("visualizer.title") }}</h1>
      <p class="text-sm text-muted">{{ t("visualizer.hint") }}</p>
    </header>

    <div class="grid gap-3 sm:grid-cols-3">
      <button
        v-for="option in options"
        :key="option.id"
        type="button"
        class="grid gap-2 border p-4 text-left transition-colors"
        :class="selected === option.id ? 'ak-select' : 'border-line bg-surface text-muted hover:bg-fg/5 hover:text-fg'"
        :aria-pressed="selected === option.id"
        @click="choose(option.id)"
      >
        <component :is="option.icon" :size="22" :stroke-width="1.6" />
        <strong class="text-sm font-bold uppercase tracking-[0.18em]">{{ t(option.labelKey) }}</strong>
        <span class="text-xs leading-relaxed normal-case tracking-normal opacity-80">{{ t(option.hintKey) }}</span>
      </button>
    </div>

    <div class="mt-auto flex flex-wrap items-center gap-3 border-t border-line pt-5">
      <button
        type="button"
        class="flex h-11 items-center gap-2 px-6 text-sm font-bold uppercase tracking-[0.2em] ak-select"
        @click="emit('start', selected)"
      >
        <Play :size="16" :stroke-width="2" />
        {{ t("visualizer.start") }}
      </button>
      <span class="text-xs text-dim">{{ t("visualizer.exitHint") }}</span>
    </div>
  </section>
</template>
