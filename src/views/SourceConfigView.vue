<!--
 Copyright 2026 Atemukesu
 SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { ArrowLeft } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import SourcesSettings from "../components/SourcesSettings.vue";

const emit = defineEmits<{ back: [] }>();
const { t } = useI18n();
</script>

<template>
  <div class="mx-auto w-full max-w-4xl p-6 lg:p-8">
    <header class="border-b border-line pb-6">
      <button type="button" class="flex items-center gap-2 text-[12px] font-semibold uppercase tracking-[0.2em] text-dim transition-colors hover:text-fg" @click="emit('back')"><ArrowLeft :size="16" />{{ t("controls.return") }}</button>
      <p class="mt-4 flex items-center gap-2 text-[13px] font-semibold uppercase tracking-[0.35em] text-accent"><span class="h-2 w-2 bg-accent"></span>{{ t("sources.eyebrow") }}</p>
      <h1 class="mt-4 text-4xl font-black uppercase leading-none tracking-tight sm:text-5xl">{{ t("settings.sources.title") }}</h1>
      <p class="mt-4 max-w-2xl text-sm text-muted">{{ t("settings.sources.desc") }}</p>
    </header>

    <div class="mt-8"><SourcesSettings /></div>
  </div>
</template>
