<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { useLyricsStore } from "../stores/lyrics";
import { useSystemFonts } from "../lib/fonts";
import LayeredSelect from "./LayeredSelect.vue";
import LyricDisplayControls from "./LyricDisplayControls.vue";

const { t } = useI18n();
const lyrics = useLyricsStore();
const { allFonts } = useSystemFonts();

const lyricSourceOptions = computed(() => [
  { value: "local" as const, label: t("settings.lyrics.sourceLocal") },
  { value: "disabled" as const, label: t("settings.lyrics.sourceDisabled") },
]);
</script>

<template>
  <section class="ak-frame grid gap-6 border border-line bg-surface p-6 lg:grid-cols-[1fr_1.3fr]">
    <div>
      <h2 class="text-sm font-bold uppercase tracking-[0.2em]">{{ t("settings.lyrics.title") }}</h2>
      <p class="mt-2 text-sm text-muted">{{ t("settings.lyrics.desc") }}</p>
    </div>
    <div class="grid gap-6">
      <label class="grid gap-2 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">
        {{ t("settings.lyrics.source") }}
        <LayeredSelect v-model="lyrics.source" :label="t('settings.lyrics.source')" :options="lyricSourceOptions" />
      </label>

      <label class="flex items-center justify-between gap-4">
        <span class="grid gap-1 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">
          {{ t("settings.lyrics.useAmll") }}
          <span class="text-[11px] font-normal normal-case tracking-normal text-dim">{{ t("settings.lyrics.useAmllHint") }}</span>
        </span>
        <input v-model="lyrics.useAmll" class="ak-switch shrink-0" type="checkbox" />
      </label>

      <div class="grid gap-4 border border-line bg-bg/40 p-4">
        <div class="flex items-center justify-between gap-3">
          <h3 class="text-[13px] font-bold uppercase tracking-[0.2em]">{{ t("settings.lyrics.classicTitle") }}</h3>
          <span
            v-if="!lyrics.useAmll"
            class="border border-accent px-2 py-0.5 text-[10px] font-semibold uppercase tracking-[0.2em] text-accent"
          >{{ t("settings.lyrics.active") }}</span>
        </div>
        <p class="text-[11px] normal-case tracking-normal text-dim">{{ t("settings.lyrics.classicHint") }}</p>
        <LyricDisplayControls v-model="lyrics.classic" :available-fonts="allFonts" show-spacing />
      </div>

      <div class="grid gap-4 border border-line bg-bg/40 p-4">
        <div class="flex items-center justify-between gap-3">
          <h3 class="text-[13px] font-bold uppercase tracking-[0.2em]">{{ t("settings.lyrics.amllTitle") }}</h3>
          <span
            v-if="lyrics.useAmll"
            class="border border-accent px-2 py-0.5 text-[10px] font-semibold uppercase tracking-[0.2em] text-accent"
          >{{ t("settings.lyrics.active") }}</span>
        </div>
        <p class="text-[11px] normal-case tracking-normal text-dim">{{ t("settings.lyrics.amllHint") }}</p>
        <LyricDisplayControls v-model="lyrics.amll" :available-fonts="allFonts" />
      </div>
    </div>
  </section>
</template>
