<!--
 Copyright 2026 Atemukesu
 SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { computed } from "vue";
import { CloudOff, Download, Gauge, HardDriveDownload, ListChecks, LoaderCircle, X } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { useCacheStore } from "../stores/cache";
import type { CacheItem } from "../stores/cache";
import { formatBytes, formatSpeed } from "../lib/format";

const { t } = useI18n();
const cache = useCacheStore();

const pending = computed(() => cache.pendingItems);
const overallPercent = computed(() =>
  cache.overallProgress === null ? null : Math.round(cache.overallProgress * 100),
);
const summary = computed(() => [
  { key: "downloads.totalSpeed", value: formatSpeed(cache.totalSpeed), icon: Gauge, accent: true },
  { key: "downloads.overallProgress", value: overallPercent.value === null ? "—" : `${overallPercent.value}%`, icon: Download, accent: false },
  { key: "downloads.activeCount", value: cache.activeCount + cache.queuedCount, icon: HardDriveDownload, accent: false },
  { key: "downloads.cachedCount", value: cache.completedItems.length, icon: ListChecks, accent: false },
]);

function progressOf(item: CacheItem): number {
  return item.size && item.size > 0 ? Math.min(1, item.written / item.size) : 0;
}
function percentOf(item: CacheItem): string {
  return item.size ? `${Math.round(progressOf(item) * 100)}%` : "—";
}
function sizeOf(item: CacheItem): string {
  return item.size ? `${formatBytes(item.written)} / ${formatBytes(item.size)}` : formatBytes(item.written);
}
function cancel(item: CacheItem) { void cache.unpin(item.key); }
</script>

<template>
  <div class="mx-auto w-full max-w-6xl px-4 py-5 sm:px-6 sm:py-6 lg:px-8 lg:py-8">
    <header class="border-b border-line pb-6">
      <div class="flex flex-wrap items-center gap-6">
        <span class="ak-frame grid h-32 w-32 shrink-0 place-items-center border border-line bg-accent/10 text-accent"><Download :size="46" :stroke-width="2" /></span>
        <div class="min-w-0">
          <p class="flex items-center gap-2 text-[13px] font-semibold uppercase tracking-[0.35em] text-accent"><span class="h-2 w-2 bg-accent"></span>{{ t("downloads.eyebrow") }}</p>
          <h1 class="mt-4 truncate text-4xl font-black leading-none tracking-tight sm:text-5xl">{{ t("downloads.downloadingTitle") }}</h1>
          <p class="mt-3 max-w-xl text-sm text-muted">{{ t("downloads.downloadingSubtitle") }}</p>
        </div>
      </div>
    </header>

    <section class="mt-6 grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-4">
      <div v-for="item in summary" :key="item.key" class="ak-frame flex items-center gap-4 border border-line bg-surface p-4">
        <span class="grid h-10 w-10 shrink-0 place-items-center" :class="item.accent ? 'bg-accent text-accent-fg' : 'bg-accent/10 text-accent'"><component :is="item.icon" :size="18" :stroke-width="2" /></span>
        <div class="min-w-0">
          <p class="font-mono text-2xl font-black tabular-nums leading-none">{{ item.value }}</p>
          <p class="mt-1.5 truncate text-[11px] uppercase tracking-[0.2em] text-dim">{{ t(item.key) }}</p>
        </div>
      </div>
    </section>

    <section class="mt-8">
      <div class="mb-3 flex flex-wrap items-end justify-between gap-3">
        <h2 class="flex items-center gap-3 text-sm font-bold uppercase tracking-[0.25em]"><span class="h-3 w-1 bg-accent"></span>{{ t("downloads.queueTitle") }}</h2>
        <span v-if="pending.length" class="font-mono text-[12px] tabular-nums text-dim">{{ cache.pendingWritten > 0 && cache.pendingTotal > 0 ? `${formatBytes(cache.pendingWritten)} / ${formatBytes(cache.pendingTotal)}` : "" }}</span>
      </div>

      <div v-if="pending.length" class="border border-line bg-surface">
        <div v-for="item in pending" :key="item.key" class="flex items-center gap-3 border-t border-line px-3 py-3 first:border-t-0 sm:gap-4">
          <span class="grid h-10 w-10 shrink-0 place-items-center bg-accent/10 text-accent"><LoaderCircle v-if="!item.active" :size="17" :stroke-width="2" class="animate-spin" /><Download v-else :size="17" :stroke-width="2" /></span>
          <div class="min-w-0 flex-1">
            <div class="flex min-w-0 items-center gap-2">
              <p class="truncate text-sm font-semibold tracking-wide">{{ cache.trackFor(item.key)?.title ?? cache.trackLabel(item.key) }}</p>
              <span class="shrink-0 text-[10px] font-semibold uppercase tracking-[0.15em]" :class="item.active ? 'text-accent' : 'text-dim'">{{ item.active ? t("downloads.active") : t("downloads.queued") }}</span>
            </div>
            <p class="mt-0.5 truncate text-xs text-muted">{{ cache.trackFor(item.key)?.artist || "—" }}</p>
            <div class="mt-2 flex items-center gap-3">
              <span class="h-1.5 min-w-0 flex-1 overflow-hidden bg-fg/5"><span class="block h-full bg-accent transition-[width] duration-500" :style="{ width: `${progressOf(item) * 100}%` }"></span></span>
              <span class="shrink-0 font-mono text-[11px] tabular-nums text-dim">{{ percentOf(item) }}</span>
            </div>
          </div>
          <div class="hidden shrink-0 text-right sm:block">
            <p class="font-mono text-[12px] tabular-nums text-fg">{{ item.active ? formatSpeed(item.speed) : t("downloads.waiting") }}</p>
            <p class="mt-0.5 font-mono text-[11px] tabular-nums text-dim">{{ sizeOf(item) }}</p>
          </div>
          <button type="button" class="grid h-8 w-8 shrink-0 place-items-center text-dim transition-colors hover:text-red-500" :title="t('downloads.cancel')" @click="cancel(item)"><X :size="16" :stroke-width="2" /></button>
        </div>
      </div>

      <div v-else class="ak-frame grid place-items-center gap-3 border border-dashed border-line bg-surface/50 px-6 py-16 text-center">
        <CloudOff :size="28" class="text-dim" />
        <p class="text-base font-semibold uppercase tracking-[0.2em]">{{ t("downloads.noActive") }}</p>
      </div>
    </section>
  </div>
</template>
