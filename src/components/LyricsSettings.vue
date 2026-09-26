<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { useLyricsStore } from "../stores/lyrics";
import { useSystemFonts } from "../lib/fonts";
import type { LyricProvider } from "../lib/preferences";
import LyricProviderOrder from "./LyricProviderOrder.vue";
import LyricDisplayControls from "./LyricDisplayControls.vue";

const { t } = useI18n();
const lyrics = useLyricsStore();
const { allFonts } = useSystemFonts();

/** Local same-name lyrics always win, so the list only orders the online fallbacks. */
const onlineProviders = computed(() => lyrics.providers.filter((provider) => provider !== "local"));

function setOnlineProviders(next: LyricProvider[]) {
  lyrics.setProviders(["local", ...next.filter((provider) => provider !== "local")]);
}
</script>

<template>
  <section class="ak-frame grid gap-6 border border-line bg-surface p-6 lg:grid-cols-[1fr_1.3fr]">
    <div>
      <h2 class="text-sm font-bold uppercase tracking-[0.2em]">{{ t("settings.lyrics.title") }}</h2>
      <p class="mt-2 text-sm text-muted">{{ t("settings.lyrics.desc") }}</p>
    </div>
    <div class="grid gap-6">
      <label class="flex items-center justify-between gap-4">
        <span class="grid gap-1 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">
          {{ t("settings.lyrics.enable") }}
          <span class="text-[11px] font-normal normal-case tracking-normal text-dim">{{ t("settings.lyrics.enableHint") }}</span>
        </span>
        <input v-model="lyrics.enabled" class="ak-switch shrink-0" type="checkbox" />
      </label>

      <div class="grid gap-3">
        <div class="grid gap-1">
          <span class="text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("settings.lyrics.providers") }}</span>
          <span class="text-[11px] font-normal normal-case tracking-normal text-dim">{{ t("settings.lyrics.providersHint") }}</span>
        </div>
        <LyricProviderOrder
          :model-value="onlineProviders"
          :disabled="!lyrics.enabled"
          @update:model-value="setOnlineProviders"
        />
      </div>

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
        <LyricDisplayControls v-model="lyrics.classic" :available-fonts="allFonts" show-spacing show-ruby />
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
        <LyricDisplayControls v-model="lyrics.amll" :available-fonts="allFonts" show-narrow-spacing />
      </div>
    </div>
  </section>
</template>
