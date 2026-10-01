<script setup lang="ts">
import { onMounted, ref } from "vue";
import { Moon, Music, Save, Sun } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { invoke } from "../api";
import { pushToast } from "../lib/toast";
import { accents } from "../lib/app";
import type { Accent, Theme } from "../lib/app";
import { localeOptions } from "../i18n";
import { useProfileStore } from "../stores/profile";
import { useSourcesStore } from "../stores/sources";
import LyricsSettings from "../components/LyricsSettings.vue";
import SourcesSettings from "../components/SourcesSettings.vue";
import pkg from "../../package.json";

const theme = defineModel<Theme>("theme", { required: true });
const accent = defineModel<Accent>("accent", { required: true });
const emit = defineEmits<{ save: []; sponsor: []; friendlyError: [error: unknown] }>();
const { t, locale } = useI18n();
const appVersion = pkg.version;
const profile = useProfileStore();
const sources = useSourcesStore();
const cacheDir = ref("");
async function saveCacheDir() { try { cacheDir.value = await invoke<string>("set_cache_dir", { dir: cacheDir.value }); pushToast("success", t("settings.cache.saved")); } catch (error) { emit("friendlyError", error); } }
onMounted(() => { void invoke<string>("get_cache_dir").then((dir) => (cacheDir.value = dir)).catch(() => {}); });
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

    <form class="mt-8 space-y-6" @submit.prevent="emit('save')">
      <section class="ak-frame grid gap-6 border border-line bg-surface p-6 lg:grid-cols-[1fr_1.3fr]">
        <div><h2 class="text-sm font-bold uppercase tracking-[0.2em]">{{ t("settings.profile.title") }}</h2><p class="mt-2 text-sm text-muted">{{ t("settings.profile.desc") }}</p></div>
        <div class="grid gap-4">
          <label class="grid gap-2 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("settings.profile.nickname") }}<input v-model="profile.profile.nickname" maxlength="32" class="h-10 border border-line bg-bg px-3 text-sm font-normal normal-case tracking-normal text-fg outline-none focus:border-accent" :placeholder="t('settings.profile.nicknamePlaceholder')" /></label>
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
        <div class="grid gap-4"><label class="grid gap-2 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("settings.cache.dir") }}<input v-model="cacheDir" class="h-10 border border-line bg-bg px-3 text-sm font-normal normal-case tracking-normal text-fg outline-none focus:border-accent" /></label><div class="flex justify-end"><button type="button" class="ak-clip-tr h-10 border border-line px-5 text-[13px] font-semibold uppercase tracking-[0.25em]" @click="saveCacheDir">{{ t("settings.cache.save") }}</button></div></div>
      </section>

      <SourcesSettings />

      <LyricsSettings />

      <div class="flex justify-end">
        <button type="submit" class="ak-clip-tr flex h-11 items-center gap-2 bg-accent px-6 text-[13px] font-bold uppercase tracking-[0.25em] text-accent-fg disabled:opacity-50" :disabled="sources.saving"><Save :size="15" />{{ t("settings.saved") }}</button>
      </div>
    </form>
  </div>
</template>
