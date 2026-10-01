<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { HardDrive, Moon, Music, Sun } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { invoke } from "../api";
import { formatBytes } from "../lib/format";
import { pushToast } from "../lib/toast";
import { accents } from "../lib/app";
import type { Accent, Theme } from "../lib/app";
import { localeOptions } from "../i18n";
import { useProfileStore } from "../stores/profile";
import { useSourcesStore } from "../stores/sources";
import LyricsSettings from "../components/LyricsSettings.vue";
import DesktopLyricSettings from "../components/DesktopLyricSettings.vue";
import pkg from "../../package.json";

const theme = defineModel<Theme>("theme", { required: true });
const accent = defineModel<Accent>("accent", { required: true });
const emit = defineEmits<{ sponsor: []; sources: []; friendlyError: [error: unknown] }>();
const { t, locale } = useI18n();
const appVersion = pkg.version;
const profile = useProfileStore();
const sources = useSourcesStore();
const cacheDir = ref("");
const cacheLimit = ref(1024);
const cacheUsed = ref(0);
const cacheLimitBytes = ref(1024 * 1024 * 1024);
/** Manually pinned tracks; kept outside the automatic budget. */
const cachePinned = ref(0);
const cachePercent = computed(() => (cacheLimitBytes.value > 0 ? Math.min(100, (cacheUsed.value / cacheLimitBytes.value) * 100) : 0));
let usageTimer = 0;
async function refreshUsage() {
  try {
    const usage = await invoke<{ usedBytes: number; limitBytes: number; pinnedBytes: number }>("cache_usage");
    cacheUsed.value = usage.usedBytes;
    cacheLimitBytes.value = usage.limitBytes;
    cachePinned.value = usage.pinnedBytes;
  } catch { /* ignore */ }
}
async function saveCacheDir() { try { cacheDir.value = await invoke<string>("set_cache_dir", { dir: cacheDir.value }); pushToast("success", t("settings.cache.saved")); } catch (error) { emit("friendlyError", error); } }
async function saveCacheLimit() { try { cacheLimit.value = await invoke<number>("set_stream_cache_limit", { limitMb: cacheLimit.value }); pushToast("success", t("settings.cache.limitSaved")); void refreshUsage(); } catch (error) { emit("friendlyError", error); } }
onMounted(() => {
  void invoke<string>("get_cache_dir").then((dir) => (cacheDir.value = dir)).catch(() => {});
  void invoke<number>("get_stream_cache_limit").then((limit) => (cacheLimit.value = limit)).catch(() => {});
  void refreshUsage();
  usageTimer = window.setInterval(() => void refreshUsage(), 3000);
});
onUnmounted(() => { window.clearInterval(usageTimer); });
</script>

<style scoped>
button {
  transition-property: color, background-color, border-color, transform, opacity;
  transition-duration: 240ms;
  transition-timing-function: cubic-bezier(0.2, 0.8, 0.2, 1);
}

button.ak-clip-tr:not(.bg-accent):hover:not(:disabled) {
  background-color: var(--accent);
  color: var(--accent-fg);
}

button.ak-clip-tr.bg-accent:hover:not(:disabled) {
  transform: scale(1.02);
}

button.ak-clip-tr.bg-accent:active:not(:disabled) {
  transform: scale(0.95);
}

button.text-dim:hover:not(:disabled) {
  color: var(--fg);
}
</style>

<template>
  <div class="mx-auto w-full max-w-4xl p-6 lg:p-8">
    <header class="border-b border-line pb-6">
      <p class="flex items-center gap-2 text-[13px] font-semibold uppercase tracking-[0.35em] text-accent"><span class="h-2 w-2 bg-accent"></span>{{ t("settings.eyebrow") }}</p>
      <h1 class="mt-4 text-4xl font-black uppercase leading-none tracking-tight sm:text-5xl">{{ t("settings.title") }}</h1>
    </header>

    <section class="ak-frame relative mt-8 border border-line bg-surface">
      <div class="pointer-events-none absolute inset-0 overflow-hidden"><div class="absolute inset-0 opacity-[0.18]" style="background: radial-gradient(circle at 90% 4%, var(--accent), transparent 55%)"></div><div class="absolute inset-0 opacity-[0.06]" style="background-image: linear-gradient(var(--fg) 1px, transparent 1px), linear-gradient(90deg, var(--fg) 1px, transparent 1px); background-size: 26px 26px"></div></div>
      <div class="relative grid gap-8 p-8 lg:grid-cols-[1.5fr_1fr] lg:items-center">
        <div class="grid gap-6">
          <div class="flex items-center gap-5"><span class="grid h-24 w-24 shrink-0 place-items-center bg-accent text-accent-fg"><Music :size="42" :stroke-width="2" /></span><div class="min-w-0"><h2 class="text-3xl font-black uppercase leading-none tracking-tight sm:text-4xl">{{ t("settings.about.title") }}</h2><p class="mt-3 max-w-lg text-sm leading-[1.7] text-muted">{{ t("settings.about.tagline") }}</p></div></div>
          <div class="grid gap-1 text-sm leading-relaxed">
            <p class="flex flex-wrap items-baseline gap-1.5"><span class="font-semibold uppercase tracking-[0.2em] text-dim">{{ t("settings.about.version") }}</span><span class="text-dim">{{ locale === "zh-CN" ? "：" : ":" }}</span><span class="font-semibold tabular-nums">v{{ appVersion }}</span></p>
            <p class="flex flex-wrap items-baseline gap-1.5"><span class="font-semibold uppercase tracking-[0.2em] text-dim">{{ t("settings.about.author") }}</span><span class="text-dim">{{ locale === "zh-CN" ? "：" : ":" }}</span><span class="font-semibold">{{ t("settings.about.authorName") }}</span></p>
          </div>
        </div>
        <div class="flex items-center justify-center py-4"><span class="grid h-32 w-32 place-items-center rounded-full border border-line"><span class="grid h-[100px] w-[100px] place-items-center rounded-full border-2 border-line-strong text-center text-fg"><button type="button" class="px-3 text-base font-black uppercase leading-tight tracking-[0.1em] transition-opacity hover:opacity-80" @click="emit('sponsor')">{{ t("settings.about.edition") }}</button></span></span></div>
      </div>
    </section>

    <div class="mt-8 space-y-6">
      <section class="ak-frame grid gap-6 border border-line bg-surface p-6 lg:grid-cols-[1fr_1.3fr]">
        <div><h2 class="text-sm font-bold uppercase tracking-[0.2em]">{{ t("settings.profile.title") }}</h2><p class="mt-2 text-sm text-muted">{{ t("settings.profile.desc") }}</p></div>
        <div class="grid gap-4">
          <label class="grid gap-2 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("settings.profile.nickname") }}<input v-model="profile.profile.nickname" maxlength="32" class="h-10 border border-line bg-bg px-3 text-sm font-normal normal-case tracking-normal text-fg outline-none focus:border-accent" :placeholder="t('settings.profile.nicknamePlaceholder')" @change="profile.setNickname(profile.profile.nickname)" /></label>
          <p class="flex items-center gap-2 text-[11px] normal-case text-dim"><span class="h-2 w-2" :class="sources.hasCloudSync ? 'bg-accent' : 'bg-dim'"></span>{{ sources.hasCloudSync ? t("settings.profile.synced") : t("settings.profile.localOnly") }}</p>
        </div>
      </section>

      <section class="ak-frame grid gap-6 border border-line bg-surface p-6 lg:grid-cols-[1fr_1.3fr]">
        <div><h2 class="text-sm font-bold uppercase tracking-[0.2em]">{{ t("settings.appearance.title") }}</h2><p class="mt-2 text-sm text-muted">{{ t("settings.appearance.desc") }}</p></div>
        <div class="grid gap-5">
          <div class="flex items-center justify-between gap-4">
            <span class="text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("theme.label") }}</span>
            <div class="inline-flex border border-line"><button type="button" class="flex h-9 items-center gap-2 px-3 text-[13px] font-semibold uppercase tracking-[0.2em]" :class="theme === 'light' ? 'bg-accent text-accent-fg' : 'text-muted hover:text-fg'" @click="theme = 'light'"><Sun :size="14" />{{ t("theme.light") }}</button><button type="button" class="flex h-9 items-center gap-2 border-l border-line px-3 text-[13px] font-semibold uppercase tracking-[0.2em]" :class="theme === 'dark' ? 'bg-accent text-accent-fg' : 'text-muted hover:text-fg'" @click="theme = 'dark'"><Moon :size="14" />{{ t("theme.dark") }}</button></div>
          </div>
          <div class="flex items-center justify-between gap-4">
            <span class="text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("accent.label") }}</span>
            <div class="flex items-center gap-2"><button v-for="item in accents" :key="item.id" type="button" class="h-5 w-5 border-2 transition-transform hover:scale-110" :style="{ backgroundColor: item.swatch, borderColor: accent === item.id ? 'var(--fg)' : 'transparent' }" :title="t(item.key)" @click="accent = item.id"></button></div>
          </div>
          <div class="flex items-center justify-between gap-4">
            <span class="text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("language.label") }}</span>
            <div class="inline-flex border border-line"><button v-for="(item, index) in localeOptions" :key="item.id" type="button" class="h-9 px-4 text-[13px] font-semibold uppercase tracking-[0.2em]" :class="[index ? 'border-l border-line' : '', locale === item.id ? 'bg-accent text-accent-fg' : 'text-muted hover:text-fg']" @click="locale = item.id">{{ t(item.labelKey) }}</button></div>
          </div>
        </div>
      </section>

      <section class="ak-frame grid gap-6 border border-line bg-surface p-6 lg:grid-cols-[1fr_1.3fr]">
        <div><h2 class="text-sm font-bold uppercase tracking-[0.2em]">{{ t("settings.cache.title") }}</h2><p class="mt-2 text-sm text-muted">{{ t("settings.cache.desc") }}</p></div>
        <div class="grid gap-4">
          <label class="grid gap-2 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("settings.cache.dir") }}<input v-model="cacheDir" class="h-10 border border-line bg-bg px-3 text-sm font-normal normal-case tracking-normal text-fg outline-none focus:border-accent" /></label>
          <label class="grid gap-2 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("settings.cache.limit") }}<input v-model.number="cacheLimit" type="number" min="128" step="128" class="h-10 border border-line bg-bg px-3 text-sm font-normal normal-case tracking-normal text-fg outline-none focus:border-accent" /></label>
          <p class="text-[11px] leading-relaxed text-dim">{{ t("settings.cache.limitDesc") }}</p>
          <div class="grid gap-2">
            <div class="flex items-baseline justify-between text-[11px] font-semibold uppercase tracking-[0.2em] text-dim"><span>{{ t("settings.cache.used") }}</span><span class="tabular-nums">{{ formatBytes(cacheUsed) }} / {{ formatBytes(cacheLimitBytes) }}</span></div>
            <span class="block h-2 w-full overflow-hidden bg-fg/10"><span class="block h-full bg-accent transition-[width]" :style="{ width: `${cachePercent}%` }"></span></span>
            <div class="flex items-baseline justify-between text-[11px] font-semibold uppercase tracking-[0.2em] text-dim"><span>{{ t("settings.cache.pinned") }}</span><span class="tabular-nums">{{ formatBytes(cachePinned) }}</span></div>
          </div>
          <div class="flex justify-end gap-2"><button type="button" class="ak-clip-tr h-10 border border-line px-5 text-[13px] font-semibold uppercase tracking-[0.25em]" @click="saveCacheDir">{{ t("settings.cache.save") }}</button><button type="button" class="ak-clip-tr h-10 border border-line px-5 text-[13px] font-semibold uppercase tracking-[0.25em]" @click="saveCacheLimit">{{ t("settings.cache.limitSave") }}</button></div>
        </div>
      </section>

      <section class="ak-frame grid gap-6 border border-line bg-surface p-6 lg:grid-cols-[1fr_1.3fr]">
        <div><h2 class="text-sm font-bold uppercase tracking-[0.2em]">{{ t("settings.sources.title") }}</h2><p class="mt-2 text-sm text-muted">{{ t("settings.sources.desc") }}</p></div>
        <div class="grid gap-4">
          <p class="text-sm text-muted">{{ t("settings.sources.count", { count: sources.sources.length }) }}</p>
          <p class="flex items-center gap-2 text-[11px] text-dim"><span class="h-2 w-2" :class="sources.hasCloudSync ? 'bg-accent' : 'bg-dim'"></span>{{ sources.hasCloudSync ? t("settings.sources.cloudAvailable") : t("settings.sources.cloudUnavailable") }}</p>
          <div class="flex justify-end"><button type="button" class="ak-clip-tr flex h-10 items-center gap-2 bg-accent px-5 text-[13px] font-bold uppercase tracking-[0.25em] text-accent-fg" @click="emit('sources')"><HardDrive :size="15" />{{ t("settings.sources.manage") }}</button></div>
        </div>
      </section>

      <LyricsSettings />

      <DesktopLyricSettings />
    </div>
  </div>
</template>
