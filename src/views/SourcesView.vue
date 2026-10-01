<script setup lang="ts">
import { computed } from "vue";
import { HardDrive, Server, Settings2 } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { usePlayerStore } from "../stores/player";
import { useSourcesStore } from "../stores/sources";
import type { SongSource } from "../lib/sources";

defineProps<{ loading: boolean }>();
const emit = defineEmits<{ open: [id: string]; settings: [] }>();
const { t } = useI18n();
const player = usePlayerStore();
const sources = useSourcesStore();

interface SourceRow {
  source: SongSource;
  count: number;
  isSync: boolean;
}

const rows = computed<SourceRow[]>(() =>
  sources.sources.map((source) => ({
    source,
    count: player.tracks.filter((track) => track.sourceId === source.id).length,
    isSync: source.id === sources.syncId,
  })),
);
</script>

<template>
  <div class="mx-auto w-full max-w-5xl px-6 py-6 lg:px-8 lg:py-8">
    <header class="border-b border-line pb-6">
      <p class="flex items-center gap-2 text-[13px] font-semibold uppercase tracking-[0.35em] text-accent"><span class="h-2 w-2 bg-accent"></span>{{ t("sources.eyebrow") }}</p>
      <h1 class="mt-4 text-4xl font-black uppercase leading-none tracking-tight sm:text-5xl">{{ t("sources.title") }}</h1>
      <p class="mt-4 max-w-2xl text-sm text-muted">{{ t("sources.desc") }}</p>
    </header>

    <p v-if="!sources.hasCloudSync && sources.hasSources" class="ak-frame mt-6 border border-accent/60 bg-accent/5 p-4 text-sm text-fg">{{ t("sources.localOnlyWarning") }}</p>

    <div v-if="rows.length === 0" class="ak-frame mt-8 grid place-items-center gap-4 border border-dashed border-line bg-surface/50 px-6 py-20 text-center">
      <HardDrive :size="36" class="text-dim" />
      <p class="text-base font-semibold uppercase tracking-[0.2em]">{{ t("sources.emptyTitle") }}</p>
      <p class="max-w-md text-sm text-muted">{{ t("sources.emptyDesc") }}</p>
      <button type="button" class="ak-clip-tr flex h-10 items-center gap-2 bg-accent px-5 text-[13px] font-bold uppercase tracking-[0.25em] text-accent-fg" @click="emit('settings')"><Settings2 :size="15" />{{ t("sources.manage") }}</button>
    </div>

    <div v-else class="mt-8 grid gap-4 sm:grid-cols-2">
      <button v-for="row in rows" :key="row.source.id" type="button" class="ak-frame group flex items-center gap-4 border border-line bg-surface p-5 text-left transition-colors hover:border-accent" @click="emit('open', row.source.id)">
        <span class="grid h-12 w-12 shrink-0 place-items-center bg-accent/10 text-accent"><Server v-if="row.source.kind === 'webdav'" :size="20" :stroke-width="2" /><HardDrive v-else :size="20" :stroke-width="2" /></span>
        <div class="min-w-0 flex-1">
          <p class="truncate text-sm font-bold">{{ sources.label(row.source) }}</p>
          <p class="mt-1 font-mono text-[11px] uppercase tracking-[0.15em] text-dim">{{ row.source.kind === "webdav" ? t("sources.kindWebdav") : t("sources.kindLocal") }} · {{ t("library.tracks", { count: row.count }) }}</p>
        </div>
        <span v-if="row.isSync" class="shrink-0 border border-accent px-2 py-1 font-mono text-[10px] uppercase tracking-[0.2em] text-accent">{{ t("sources.syncBadge") }}</span>
      </button>
    </div>
  </div>
</template>
