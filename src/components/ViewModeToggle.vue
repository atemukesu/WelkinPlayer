<!--
 Copyright 2026 Atemukesu
 SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { LayoutGrid, List } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { setTrackViewMode, trackViewMode } from "../lib/ui";

const { t } = useI18n();
</script>

<template>
  <div class="inline-flex border border-line">
    <button type="button" class="grid h-10 w-10 place-items-center transition-colors" :class="trackViewMode === 'list' ? 'bg-accent text-accent-fg' : 'text-dim hover:text-fg'" :title="t('library.viewList')" @click="setTrackViewMode('list')"><List :size="16" :stroke-width="2" /></button>
    <button type="button" class="grid h-10 w-10 place-items-center border-l border-line transition-colors" :class="trackViewMode === 'grid' ? 'bg-accent text-accent-fg' : 'text-dim hover:text-fg'" :title="t('library.viewGrid')" @click="setTrackViewMode('grid')"><LayoutGrid :size="16" :stroke-width="2" /></button>
  </div>
</template>
