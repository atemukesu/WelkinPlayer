<!--
 Copyright 2026 Atemukesu
 SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { computed } from "vue";
import { ChevronRight, Disc3, HardDrive, Mic2 } from "@lucide/vue";
import type { Component } from "vue";
import { useI18n } from "vue-i18n";
import { usePlayerStore } from "../stores/player";
import { useSourcesStore } from "../stores/sources";
import { groupTracks } from "../lib/grouping";
import type { View } from "../lib/app";

const emit = defineEmits<{ navigate: [view: View] }>();
const { t } = useI18n();
const player = usePlayerStore();
const sources = useSourcesStore();

interface CategoryCard {
  view: "artists" | "albums" | "sources";
  titleKey: string;
  descKey: string;
  countKey: string;
  count: number;
  icon: Component;
}

const cards = computed<CategoryCard[]>(() => [
  { view: "artists", titleKey: "categories.artists", descKey: "categories.artistsDesc", countKey: "library.collections.artistCount", count: groupTracks(player.tracks, "artists").length, icon: Mic2 },
  { view: "albums", titleKey: "categories.albums", descKey: "categories.albumsDesc", countKey: "library.collections.albumCount", count: groupTracks(player.tracks, "albums").length, icon: Disc3 },
  { view: "sources", titleKey: "categories.sources", descKey: "categories.sourcesDesc", countKey: "categories.sourceCount", count: sources.sources.length, icon: HardDrive },
]);
</script>

<template>
  <div class="mx-auto w-full max-w-5xl px-6 py-6 lg:px-8 lg:py-8">
    <header class="border-b border-line pb-6">
      <p class="flex items-center gap-2 text-[13px] font-semibold uppercase tracking-[0.35em] text-accent"><span class="h-2 w-2 bg-accent"></span>{{ t("categories.eyebrow") }}</p>
      <h1 class="mt-4 text-4xl font-black uppercase leading-none tracking-tight sm:text-5xl">{{ t("categories.title") }}</h1>
      <p class="mt-4 max-w-2xl text-sm text-muted">{{ t("categories.subtitle") }}</p>
    </header>

    <div class="mt-8 grid gap-4 sm:grid-cols-3">
      <button v-for="card in cards" :key="card.view" type="button" class="ak-frame group flex flex-col border border-line bg-surface p-5 text-left transition-colors hover:border-accent" @click="emit('navigate', card.view)">
        <span class="grid h-12 w-12 place-items-center bg-accent/10 text-accent"><component :is="card.icon" :size="22" :stroke-width="2" /></span>
        <p class="mt-5 text-lg font-black uppercase tracking-tight">{{ t(card.titleKey) }}</p>
        <p class="mt-2 flex-1 text-sm text-muted">{{ t(card.descKey) }}</p>
        <span class="mt-6 flex items-center justify-between border-t border-line pt-3 text-sm text-dim">
          <span>{{ t(card.countKey, { count: card.count }) }}</span>
          <ChevronRight :size="16" class="transition-colors group-hover:text-accent" />
        </span>
      </button>
    </div>
  </div>
</template>
