<!--
 Copyright 2026 Atemukesu
 SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { Bug, Copy, ExternalLink, GitFork, HardDrive, Moon, Music2, Sun, Users } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { openUrl } from "@tauri-apps/plugin-opener";
import { invoke } from "../api";
import { formatBytes, formatSpeed } from "../lib/format";
import { pushToast } from "../lib/toast";
import { accents } from "../lib/app";
import type { Accent, Theme } from "../lib/app";
import { localeOptions } from "../i18n";
import { useProfileStore } from "../stores/profile";
import { useSourcesStore } from "../stores/sources";
import { useLicenseStore } from "../stores/license";
import { useWindowBehaviorStore } from "../stores/windowBehavior";
import { useUpdateStore } from "../stores/update";
import { useCacheStore } from "../stores/cache";
import { isDesktop } from "../lib/desktopLyric";
import LyricsSettings from "../components/LyricsSettings.vue";
import DesktopLyricSettings from "../components/DesktopLyricSettings.vue";
import pkg from "../../package.json";

const theme = defineModel<Theme>("theme", { required: true });
const accent = defineModel<Accent>("accent", { required: true });
const emit = defineEmits<{ sponsor: []; sources: []; friendlyError: [error: unknown] }>();
const { t, locale } = useI18n();
const appVersion = pkg.version;
/** Project links and the user group shown in the community card. */
const repoUrl = "https://github.com/atemukesu/WelkinPlayer";
const issuesUrl = "https://github.com/atemukesu/WelkinPlayer/issues";
const qqGroup = "1109689488";
const profile = useProfileStore();
const sources = useSourcesStore();
const license = useLicenseStore();
const windowBehavior = useWindowBehaviorStore();
const update = useUpdateStore();
const cache = useCacheStore();
/** The close-to-tray preference is desktop-only; Android never shows it. */
const desktop = isDesktop();
/** Edition label shown in the About card's edition dial. */
const editionLabel = computed(() => (license.isPro ? t("sponsor.compare.badgePro") : t("settings.about.edition")));
/** Download progress readout: bytes received, with the total when known. */
const updateSizeText = computed(() =>
  update.total && update.total > 0 ? `${formatBytes(update.downloaded)} / ${formatBytes(update.total)}` : formatBytes(update.downloaded),
);
/** Current update transfer rate, or null while unknown. */
const updateSpeedText = computed(() => (update.speed && update.speed > 0 ? formatSpeed(update.speed) : null));
const cacheDir = ref("");
const smartLimit = ref(1024);
const smartEnabled = ref(true);
const streamEnabled = ref(true);
const smartBytes = ref(0);
const smartLimitBytes = ref(1024 * 1024 * 1024);
const streamBytes = ref(0);
const streamLimitBytes = ref(256 * 1024 * 1024);
/** Manually pinned tracks; kept outside the automatic budget. */
const cachePinned = ref(0);
const clearingCache = ref(false);
const smartPercent = computed(() => (smartLimitBytes.value > 0 ? Math.min(100, (smartBytes.value / smartLimitBytes.value) * 100) : 0));
const streamPercent = computed(() => (streamLimitBytes.value > 0 ? Math.min(100, (streamBytes.value / streamLimitBytes.value) * 100) : 0));
let usageTimer = 0;
async function refreshUsage() {
  try {
    const usage = await invoke<{ streamBytes: number; streamLimitBytes: number; smartBytes: number; smartLimitBytes: number; pinnedBytes: number }>("cache_usage");
    streamBytes.value = usage.streamBytes;
    streamLimitBytes.value = usage.streamLimitBytes;
    smartBytes.value = usage.smartBytes;
    smartLimitBytes.value = usage.smartLimitBytes;
    cachePinned.value = usage.pinnedBytes;
  } catch { /* ignore */ }
}
async function saveCacheDir() { try { cacheDir.value = await invoke<string>("set_cache_dir", { dir: cacheDir.value }); pushToast("success", t("settings.cache.saved")); } catch (error) { emit("friendlyError", error); } }
async function saveCacheLimit() { try { smartLimit.value = await invoke<number>("set_smart_cache_limit", { limitMb: smartLimit.value }); pushToast("success", t("settings.cache.limitSaved")); void refreshUsage(); } catch (error) { emit("friendlyError", error); } }
/** Persist the smart-cache toggle; revert the switch if the backend rejects it. */
async function toggleSmartEnabled() {
  const next = !smartEnabled.value;
  smartEnabled.value = next;
  try {
    smartEnabled.value = await invoke<boolean>("set_smart_cache_enabled", { enabled: next });
    void cache.refresh();
  } catch (error) {
    smartEnabled.value = !next;
    emit("friendlyError", error);
  }
}
/** Persist the read-ahead buffer toggle; revert the switch on failure. */
async function toggleStreamEnabled() {
  const next = !streamEnabled.value;
  streamEnabled.value = next;
  try {
    streamEnabled.value = await invoke<boolean>("set_stream_cache_enabled", { enabled: next });
  } catch (error) {
    streamEnabled.value = !next;
    emit("friendlyError", error);
  }
}
/** Clear the automatic caches; pinned tracks are kept. */
async function clearCache() {
  clearingCache.value = true;
  try {
    await invoke("clear_cache");
    pushToast("success", t("settings.cache.cleared"));
    void cache.refresh();
    void refreshUsage();
  } catch (error) {
    emit("friendlyError", error);
  } finally {
    clearingCache.value = false;
  }
}
/** Open an external link, falling back to a browser tab and finally a toast. */
async function openLink(url: string) {
  try {
    await openUrl(url);
  } catch {
    try {
      window.open(url, "_blank", "noopener,noreferrer");
    } catch {
      pushToast("error", t("settings.community.openFailed"));
    }
  }
}
/** Copy the QQ group number, with a legacy fallback for the clipboard API. */
async function copyQq() {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(qqGroup);
    } else {
      throw new Error("clipboard unavailable");
    }
    pushToast("success", t("settings.community.qqCopied"));
  } catch {
    const field = document.createElement("textarea");
    field.value = qqGroup;
    field.style.position = "fixed";
    field.style.opacity = "0";
    document.body.appendChild(field);
    field.select();
    const copied = document.execCommand("copy");
    document.body.removeChild(field);
    pushToast(copied ? "success" : "error", t(copied ? "settings.community.qqCopied" : "settings.community.qqCopyFailed"));
  }
}
/** Manual check from the settings card; opens the modal or reports up to date. */
async function checkUpdate() {
  try {
    const info = await update.check();
    if (info.available) update.show();
    else pushToast("success", t("settings.update.latest"));
  } catch (error) {
    emit("friendlyError", error);
  }
}
onMounted(() => {
  void invoke<string>("get_cache_dir").then((dir) => (cacheDir.value = dir)).catch(() => {});
  void invoke<{ smartEnabled: boolean; smartLimitMb: number; streamEnabled: boolean }>("get_cache_config")
    .then((config) => {
      smartEnabled.value = config.smartEnabled;
      streamEnabled.value = config.streamEnabled;
      smartLimit.value = config.smartLimitMb;
    })
    .catch(() => {});
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

/* Edition dial. Standard is a plain dial; Pro wraps it in a slowly rotating
   accent aura and makes hover/active react, matching the splash language. */
.ak-tier-dial {
  position: relative;
  isolation: isolate;
}

.ak-tier-aura {
  position: absolute;
  inset: 0;
  margin: auto;
  width: 8rem;
  height: 8rem;
  border-radius: 9999px;
  opacity: 0;
  pointer-events: none;
  background: conic-gradient(from 0deg, transparent 0 54%, var(--accent) 78%, transparent 100%);
  filter: blur(10px);
  animation: ak-tier-spin 4.5s linear infinite;
  transition: opacity 520ms cubic-bezier(0.2, 0.8, 0.2, 1);
}

.ak-tier-disc {
  z-index: 1;
  background: var(--surface);
  transition: transform 320ms cubic-bezier(0.2, 0.8, 0.2, 1), box-shadow 320ms cubic-bezier(0.2, 0.8, 0.2, 1), border-color 320ms cubic-bezier(0.2, 0.8, 0.2, 1);
}

.ak-tier-inner {
  transition: border-color 320ms cubic-bezier(0.2, 0.8, 0.2, 1);
}

.ak-tier-button {
  transition: opacity 240ms cubic-bezier(0.2, 0.8, 0.2, 1), text-shadow 320ms cubic-bezier(0.2, 0.8, 0.2, 1);
}

.ak-tier-button:hover {
  opacity: 0.85;
}

.ak-tier-dial.is-pro .ak-tier-aura {
  opacity: 0.9;
}

.ak-tier-dial.is-pro .ak-tier-disc {
  border-color: var(--accent);
  box-shadow: 0 0 26px var(--accent-soft);
}

.ak-tier-dial.is-pro .ak-tier-inner {
  border-color: var(--accent);
}

.ak-tier-dial.is-pro .ak-tier-button {
  text-shadow: 0 0 14px var(--accent-soft);
}

.ak-tier-dial.is-pro:hover .ak-tier-disc {
  transform: scale(1.05);
  box-shadow: 0 0 42px var(--accent-soft);
}

.ak-tier-dial.is-pro:hover .ak-tier-aura {
  opacity: 1;
}

.ak-tier-dial.is-pro:active .ak-tier-disc {
  transform: scale(0.97);
}

@keyframes ak-tier-spin {
  to { transform: rotate(360deg); }
}
</style>

<template>
  <div class="mx-auto w-full max-w-4xl p-6 lg:p-8">
    <header class="border-b border-line pb-6">
      <p class="flex items-center gap-2 text-[13px] font-semibold uppercase tracking-[0.35em] text-accent"><span class="h-2 w-2 bg-accent"></span>{{ t("settings.eyebrow") }}</p>
      <h1 class="mt-4 text-4xl font-black uppercase leading-none tracking-tight sm:text-5xl">{{ t("settings.title") }}</h1>
    </header>

    <section class="ak-frame relative mt-8 border border-line bg-surface">
      <div v-if="license.isPro" class="pointer-events-none absolute inset-0 overflow-hidden"><div class="absolute inset-0 opacity-[0.18]" style="background: radial-gradient(circle at 90% 4%, var(--accent), transparent 55%)"></div><div class="absolute inset-0 opacity-[0.06]" style="background-image: linear-gradient(var(--fg) 1px, transparent 1px), linear-gradient(90deg, var(--fg) 1px, transparent 1px); background-size: 26px 26px"></div></div>
      <div class="relative grid gap-8 p-8 lg:grid-cols-[1.5fr_1fr] lg:items-center">
        <div class="grid gap-6">
          <div class="flex items-center gap-5"><span class="grid h-24 w-24 shrink-0 place-items-center bg-accent text-accent-fg"><Music2 :size="42" :stroke-width="2" /></span><div class="min-w-0"><h2 class="text-3xl font-black uppercase leading-none tracking-tight sm:text-4xl">{{ t("settings.about.title") }}</h2><p class="mt-3 max-w-lg text-sm leading-[1.7] text-muted">{{ t("settings.about.tagline") }}</p></div></div>
          <div class="grid gap-1 text-sm leading-relaxed">
            <p class="flex flex-wrap items-baseline gap-1.5"><span class="font-semibold uppercase tracking-[0.2em] text-dim">{{ t("settings.about.version") }}</span><span class="text-dim">{{ locale === "zh-CN" ? "：" : ":" }}</span><span class="font-semibold tabular-nums">v{{ appVersion }}</span></p>
            <p class="flex flex-wrap items-baseline gap-1.5"><span class="font-semibold uppercase tracking-[0.2em] text-dim">{{ t("settings.about.author") }}</span><span class="text-dim">{{ locale === "zh-CN" ? "：" : ":" }}</span><span class="font-semibold">{{ t("settings.about.authorName") }}</span></p>
          </div>
        </div>
        <div class="ak-tier-dial flex items-center justify-center py-4" :class="{ 'is-pro': license.isPro }">
          <span class="ak-tier-aura" aria-hidden="true"></span>
          <span class="ak-tier-disc relative grid h-32 w-32 place-items-center rounded-full border border-line">
            <span class="ak-tier-inner grid h-[100px] w-[100px] place-items-center rounded-full border-2 border-line-strong text-center text-fg">
              <button type="button" class="ak-tier-button px-3 text-base font-black uppercase leading-tight tracking-[0.1em]" @click="emit('sponsor')">{{ editionLabel }}</button>
            </span>
          </span>
        </div>
      </div>
    </section>

    <div class="mt-8 space-y-6">
      <section class="ak-frame grid gap-6 border border-line bg-surface p-6 lg:grid-cols-[1fr_1.3fr]">
        <div><h2 class="text-sm font-bold uppercase tracking-[0.2em]">{{ t("settings.update.title") }}</h2><p class="mt-2 text-sm text-muted">{{ t("settings.update.desc") }}</p></div>
        <div class="grid gap-4">
          <p class="flex flex-wrap items-baseline gap-1.5 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("settings.update.current") }}<span class="font-semibold tabular-nums text-fg">v{{ appVersion }}</span></p>
          <div v-if="update.available" class="grid gap-3 border border-line bg-bg p-4">
            <p class="text-sm font-semibold text-accent">{{ t("settings.update.available", { version: update.info?.version }) }}</p>
            <div v-if="update.installing" class="grid gap-2.5">
              <span class="block h-2 w-full overflow-hidden bg-fg/10"><span class="block h-full bg-accent transition-[width]" :style="{ width: `${Math.round((update.progress ?? 0) * 100)}%` }"></span></span>
              <div class="grid grid-cols-[1fr_5.5rem_2.5rem] items-baseline gap-x-3 text-[11px] tabular-nums text-dim">
                <span class="truncate">{{ updateSizeText }}</span>
                <span class="truncate text-right">{{ updateSpeedText ?? "" }}</span>
                <span class="text-right font-semibold text-fg">{{ Math.round((update.progress ?? 0) * 100) }}%</span>
              </div>
            </div>
            <button type="button" class="ak-clip-tr flex h-10 items-center justify-center border border-line px-5 text-[13px] font-semibold uppercase tracking-[0.25em] text-fg hover:border-accent hover:text-accent" @click="update.show()">{{ t("settings.update.view") }}</button>
          </div>
          <template v-else>
            <p v-if="update.error" class="text-[12px] leading-relaxed text-rose-500">{{ update.error }}</p>
            <p v-else-if="update.checked" class="text-sm text-muted">{{ t("settings.update.latest") }}</p>
          </template>
          <div class="flex justify-end">
            <button type="button" class="ak-clip-tr h-10 border border-line px-5 text-[13px] font-semibold uppercase tracking-[0.25em] disabled:opacity-60" :disabled="update.checking || update.installing" @click="checkUpdate">{{ t(update.checking ? "settings.update.checking" : "settings.update.check") }}</button>
          </div>
        </div>
      </section>

      <section class="ak-frame grid gap-6 border border-line bg-surface p-6 lg:grid-cols-[1fr_1.3fr]">
        <div><h2 class="text-sm font-bold uppercase tracking-[0.2em]">{{ t("settings.community.title") }}</h2><p class="mt-2 text-sm text-muted">{{ t("settings.community.desc") }}</p></div>
        <div class="grid min-w-0 gap-3">
          <div class="flex flex-col gap-3 border border-line bg-bg px-4 py-3 overflow-hidden sm:flex-row sm:items-center sm:justify-between">
            <span class="flex min-w-0 items-center gap-3"><GitFork class="shrink-0 text-accent" :size="18" :stroke-width="2" /><span class="min-w-0"><span class="block text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("settings.community.repo") }}</span><span class="block break-all font-mono text-xs leading-relaxed text-fg">{{ repoUrl }}</span></span></span>
            <button type="button" class="ak-clip-tr flex min-h-9 w-full shrink-0 items-center justify-center gap-2 py-2 text-center leading-tight whitespace-normal bg-accent px-4 text-[12px] font-bold uppercase tracking-[0.2em] text-accent-fg sm:w-auto" @click="openLink(repoUrl)"><ExternalLink :size="14" />{{ t("settings.community.repoAction") }}</button>
          </div>
          <div class="flex flex-col gap-3 border border-line bg-bg px-4 py-3 overflow-hidden sm:flex-row sm:items-center sm:justify-between">
            <span class="flex min-w-0 items-center gap-3"><Bug class="shrink-0 text-accent" :size="18" :stroke-width="2" /><span class="min-w-0"><span class="block text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("settings.community.issues") }}</span><span class="block break-all font-mono text-xs leading-relaxed text-fg">{{ issuesUrl }}</span></span></span>
            <button type="button" class="ak-clip-tr flex min-h-9 w-full shrink-0 items-center justify-center gap-2 py-2 text-center leading-tight whitespace-normal border border-line px-4 text-[12px] font-bold uppercase tracking-[0.2em] text-fg sm:w-auto" @click="openLink(issuesUrl)"><ExternalLink :size="14" />{{ t("settings.community.issuesAction") }}</button>
          </div>
          <div class="flex flex-col gap-3 border border-line bg-bg px-4 py-3 overflow-hidden sm:flex-row sm:items-center sm:justify-between">
            <span class="flex min-w-0 items-center gap-3"><Users class="shrink-0 text-accent" :size="18" :stroke-width="2" /><span class="min-w-0"><span class="block text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("settings.community.qq") }}</span><span class="block break-all font-mono text-xs leading-relaxed text-fg">{{ t("settings.community.qqNumber") }}</span></span></span>
            <button type="button" class="ak-clip-tr flex min-h-9 w-full shrink-0 items-center justify-center gap-2 py-2 text-center leading-tight whitespace-normal border border-line px-4 text-[12px] font-bold uppercase tracking-[0.2em] text-fg sm:w-auto" @click="copyQq"><Copy :size="14" />{{ t("settings.community.qqCopy") }}</button>
          </div>
        </div>
      </section>

      <section class="ak-frame grid gap-6 border border-line bg-surface p-6 lg:grid-cols-[1fr_1.3fr]">
        <div><h2 class="text-sm font-bold uppercase tracking-[0.2em]">{{ t("settings.profile.title") }}</h2><p class="mt-2 text-sm text-muted">{{ t("settings.profile.desc") }}</p></div>
        <div class="grid gap-4">
          <label class="grid gap-2 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("settings.profile.nickname") }}<input v-model="profile.profile.nickname" maxlength="32" class="h-10 border border-line bg-bg px-3 text-sm font-normal normal-case tracking-normal text-fg outline-none focus:border-accent" :placeholder="t('settings.profile.nicknamePlaceholder')" @change="profile.setNickname(profile.profile.nickname)" /></label>
          <p v-if="license.isPro" class="text-[11px] font-semibold leading-relaxed text-amber-500">{{ t("settings.profile.nicknameWarning") }}</p>
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

      <section v-if="desktop" class="ak-frame grid gap-6 border border-line bg-surface p-6 lg:grid-cols-[1fr_1.3fr]">
        <div><h2 class="text-sm font-bold uppercase tracking-[0.2em]">{{ t("settings.window.title") }}</h2><p class="mt-2 text-sm text-muted">{{ t("settings.window.desc") }}</p></div>
        <div class="grid gap-4">
          <div class="flex items-center justify-between gap-4">
            <span class="text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("settings.window.action") }}</span>
            <div class="inline-flex border border-line"><button type="button" class="h-9 px-4 text-[13px] font-semibold uppercase tracking-[0.2em]" :class="!windowBehavior.closeToTray ? 'bg-accent text-accent-fg' : 'text-muted hover:text-fg'" @click="windowBehavior.closeToTray = false">{{ t("settings.window.close") }}</button><button type="button" class="h-9 border-l border-line px-4 text-[13px] font-semibold uppercase tracking-[0.2em]" :class="windowBehavior.closeToTray ? 'bg-accent text-accent-fg' : 'text-muted hover:text-fg'" @click="windowBehavior.closeToTray = true">{{ t("settings.window.tray") }}</button></div>
          </div>
          <p class="text-[11px] leading-relaxed text-dim">{{ t("settings.window.hint") }}</p>
        </div>
      </section>

      <section class="ak-frame grid gap-6 border border-line bg-surface p-6 lg:grid-cols-[1fr_1.3fr]">
        <div><h2 class="text-sm font-bold uppercase tracking-[0.2em]">{{ t("settings.cache.title") }}</h2><p class="mt-2 text-sm text-muted">{{ t("settings.cache.desc") }}</p></div>
        <div class="grid gap-5">
          <label class="grid gap-2 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("settings.cache.dir") }}<input v-model="cacheDir" class="h-10 border border-line bg-bg px-3 text-sm font-normal normal-case tracking-normal text-fg outline-none focus:border-accent" /></label>
          <div class="flex justify-end"><button type="button" class="ak-clip-tr h-10 border border-line px-5 text-[13px] font-semibold uppercase tracking-[0.25em]" @click="saveCacheDir">{{ t("settings.cache.save") }}</button></div>

          <div class="grid gap-4 border border-line bg-bg p-4">
            <label class="flex items-start justify-between gap-4">
              <span class="grid gap-1 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("settings.cache.smartTitle") }}<span class="text-[11px] font-normal normal-case tracking-normal text-dim">{{ t("settings.cache.smartDesc") }}</span></span>
              <input :checked="smartEnabled" class="ak-switch shrink-0" type="checkbox" :aria-label="t('settings.cache.smartEnabled')" @change="toggleSmartEnabled" />
            </label>
            <label class="grid gap-2 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("settings.cache.smartLimit") }}<input v-model.number="smartLimit" type="number" min="128" step="128" class="h-10 border border-line bg-bg px-3 text-sm font-normal normal-case tracking-normal text-fg outline-none focus:border-accent" /></label>
            <div class="grid gap-2">
              <div class="flex items-baseline justify-between text-[11px] font-semibold uppercase tracking-[0.2em] text-dim"><span>{{ t("settings.cache.smartUsed") }}</span><span class="tabular-nums">{{ formatBytes(smartBytes) }} / {{ formatBytes(smartLimitBytes) }}</span></div>
              <span class="block h-2 w-full overflow-hidden bg-fg/10"><span class="block h-full bg-accent transition-[width]" :style="{ width: `${smartPercent}%` }"></span></span>
            </div>
            <div class="flex justify-end"><button type="button" class="ak-clip-tr h-10 border border-line px-5 text-[13px] font-semibold uppercase tracking-[0.25em]" @click="saveCacheLimit">{{ t("settings.cache.limitSave") }}</button></div>
          </div>

          <div class="grid gap-4 border border-line bg-bg p-4">
            <label class="flex items-start justify-between gap-4">
              <span class="grid gap-1 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("settings.cache.streamTitle") }}<span class="text-[11px] font-normal normal-case tracking-normal text-dim">{{ t("settings.cache.streamDesc") }}</span></span>
              <input :checked="streamEnabled" class="ak-switch shrink-0" type="checkbox" :aria-label="t('settings.cache.streamEnabled')" @change="toggleStreamEnabled" />
            </label>
            <div class="grid gap-2">
              <div class="flex items-baseline justify-between text-[11px] font-semibold uppercase tracking-[0.2em] text-dim"><span>{{ t("settings.cache.streamUsed") }}</span><span class="tabular-nums">{{ formatBytes(streamBytes) }} / {{ formatBytes(streamLimitBytes) }}</span></div>
              <span class="block h-2 w-full overflow-hidden bg-fg/10"><span class="block h-full bg-accent transition-[width]" :style="{ width: `${streamPercent}%` }"></span></span>
            </div>
          </div>

          <div class="flex items-baseline justify-between text-[11px] font-semibold uppercase tracking-[0.2em] text-dim"><span>{{ t("settings.cache.pinned") }}</span><span class="tabular-nums">{{ formatBytes(cachePinned) }}</span></div>

          <p class="text-[11px] leading-relaxed text-dim">{{ t("settings.cache.clearHint") }}</p>
          <div class="flex justify-end">
            <button type="button" class="ak-clip-tr h-10 border border-rose-500/60 px-5 text-[13px] font-semibold uppercase tracking-[0.25em] text-rose-500 disabled:opacity-60" :disabled="clearingCache" @click="clearCache">{{ t(clearingCache ? "settings.cache.clearing" : "settings.cache.clear") }}</button>
          </div>
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
