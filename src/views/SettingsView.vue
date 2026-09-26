<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { Eye, EyeOff, Moon, Sun } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { describeError, invoke } from "../api";
import { pushToast } from "../lib/toast";
import { accents } from "../lib/app";
import type { Accent, Theme } from "../lib/app";
import { localeOptions } from "../i18n";
import { useWebdavStore } from "../stores/webdav";
import { useProfileStore } from "../stores/profile";
import LyricsSettings from "../components/LyricsSettings.vue";

defineProps<{ testing: boolean }>();
const theme = defineModel<Theme>("theme", { required: true });
const accent = defineModel<Accent>("accent", { required: true });
const emit = defineEmits<{ save: []; test: []; friendlyError: [error: unknown] }>();
const { t, locale } = useI18n();
const webdav = useWebdavStore(); const profile = useProfileStore();
const showPassword = ref(false); const pingState = ref<"idle" | "pending" | "ok" | "error">("idle"); const echoInput = ref(""); const echoState = ref<"idle" | "pending" | "ok" | "error">("idle"); const cacheDir = ref(""); const writeState = ref<"idle" | "pending" | "ok" | "error">("idle");
const webdavStatusLabel = computed(() => webdav.status === "connecting" ? t("settings.webdav.statusConnecting") : webdav.status === "connected" ? t("settings.webdav.statusConnected") : webdav.status === "error" ? t("settings.webdav.statusError", { value: webdav.error }) : t("settings.webdav.statusIdle"));
async function saveCacheDir() { try { cacheDir.value = await invoke<string>("set_cache_dir", { dir: cacheDir.value }); pushToast("success", t("settings.cache.saved")); } catch (error) { emit("friendlyError", error); } }
async function testWrite() { writeState.value = "pending"; try { await invoke<string>("test_webdav_write"); writeState.value = "ok"; pushToast("success", t("settings.webdav.writeOk")); } catch (error) { writeState.value = "error"; pushToast("error", t("settings.webdav.writeFailed", { value: describeError(error) })); } }
async function ping(notify = false) { pingState.value = "pending"; try { const value = await invoke<string>("ping"); pingState.value = "ok"; if (notify) pushToast("success", t("settings.ipc.ok", { value })); } catch (error) { pingState.value = "error"; if (notify) pushToast("error", t("settings.ipc.error", { value: describeError(error) })); } }
async function runEcho() { echoState.value = "pending"; try { const value = await invoke<string>("echo", { message: echoInput.value }); echoState.value = "ok"; pushToast("success", t("settings.ipc.echoOk", { value })); } catch (error) { echoState.value = "error"; pushToast("error", t("settings.ipc.echoError", { value: describeError(error) })); } }
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
  <div class="mx-auto w-full max-w-4xl p-6 lg:p-8"><header class="border-b border-line pb-6"><p class="flex items-center gap-2 text-[13px] font-semibold uppercase tracking-[0.35em] text-accent"><span class="h-2 w-2 bg-accent"></span>{{ t("settings.eyebrow") }}</p><h1 class="mt-4 text-4xl font-black uppercase leading-none tracking-tight sm:text-5xl">{{ t("settings.title") }}</h1></header><form class="mt-8 space-y-6" @submit.prevent="emit('save')">
    <section class="ak-frame grid gap-6 border border-line bg-surface p-6 lg:grid-cols-[1fr_1.3fr]"><div><h2 class="text-sm font-bold uppercase tracking-[0.2em]">{{ t("settings.profile.title") }}</h2><p class="mt-2 text-sm text-muted">{{ t("settings.profile.desc") }}</p></div><div class="grid gap-4"><label class="grid gap-2 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("settings.profile.nickname") }}<input v-model="profile.profile.nickname" maxlength="32" class="h-10 border border-line bg-bg px-3 text-sm font-normal normal-case tracking-normal text-fg outline-none focus:border-accent" :placeholder="t('settings.profile.nicknamePlaceholder')" /></label><p class="flex items-center gap-2 text-[11px] normal-case text-dim"><span class="h-2 w-2" :class="profile.hasRemoteCopy ? 'bg-accent' : 'bg-dim'"></span>{{ profile.hasRemoteCopy ? t("settings.profile.synced") : t("settings.profile.localOnly") }}</p></div></section>
    <section class="ak-frame grid gap-6 border border-line bg-surface p-6 lg:grid-cols-[1fr_1.3fr]"><div><h2 class="text-sm font-bold uppercase tracking-[0.2em]">{{ t("settings.appearance.title") }}</h2><p class="mt-2 text-sm text-muted">{{ t("settings.appearance.desc") }}</p></div><div class="grid gap-5"><div class="flex items-center justify-between gap-4"><span class="text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("theme.label") }}</span><div class="inline-flex border border-line"><button type="button" class="flex h-9 items-center gap-2 px-3 text-[13px] font-semibold uppercase tracking-[0.2em]" :class="theme === 'light' ? 'bg-accent text-accent-fg' : 'text-muted hover:text-fg'" @click="theme = 'light'"><Sun :size="14" />{{ t("theme.light") }}</button><button type="button" class="flex h-9 items-center gap-2 border-l border-line px-3 text-[13px] font-semibold uppercase tracking-[0.2em]" :class="theme === 'dark' ? 'bg-accent text-accent-fg' : 'text-muted hover:text-fg'" @click="theme = 'dark'"><Moon :size="14" />{{ t("theme.dark") }}</button></div></div><div class="flex items-center justify-between gap-4"><span class="text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("accent.label") }}</span><div class="flex items-center gap-2"><button v-for="item in accents" :key="item.id" type="button" class="h-5 w-5 border-2 transition-transform hover:scale-110" :style="{ backgroundColor: item.swatch, borderColor: accent === item.id ? 'var(--fg)' : 'transparent' }" :title="t(item.key)" @click="accent = item.id"></button></div></div><div class="flex items-center justify-between gap-4"><span class="text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("language.label") }}</span><div class="inline-flex border border-line"><button v-for="(item, index) in localeOptions" :key="item.id" type="button" class="h-9 px-4 text-[13px] font-semibold uppercase tracking-[0.2em]" :class="[index ? 'border-l border-line' : '', locale === item.id ? 'bg-accent text-accent-fg' : 'text-muted hover:text-fg']" @click="locale = item.id">{{ t(item.labelKey) }}</button></div></div></div></section>
    <section class="ak-frame grid gap-6 border border-line bg-surface p-6 lg:grid-cols-[1fr_1.3fr]"><div><h2 class="text-sm font-bold uppercase tracking-[0.2em]">{{ t("settings.cache.title") }}</h2><p class="mt-2 text-sm text-muted">{{ t("settings.cache.desc") }}</p></div><div class="grid gap-4"><label class="grid gap-2 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("settings.cache.dir") }}<input v-model="cacheDir" class="h-10 border border-line bg-bg px-3 text-sm font-normal normal-case tracking-normal text-fg outline-none focus:border-accent" /></label><div class="flex justify-end"><button type="button" class="ak-clip-tr h-10 border border-line px-5 text-[13px] font-semibold uppercase tracking-[0.25em]" @click="saveCacheDir">{{ t("settings.cache.save") }}</button></div></div></section>
    <section class="ak-frame grid gap-6 border border-line bg-surface p-6 lg:grid-cols-[1fr_1.3fr]"><div><h2 class="text-sm font-bold uppercase tracking-[0.2em]">{{ t("settings.webdav.title") }}</h2><p class="mt-2 text-sm text-muted">{{ t("settings.webdav.desc") }}</p></div><div class="grid gap-4"><label class="grid gap-2 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("settings.webdav.serverUrl") }}<input v-model="webdav.url" type="url" class="h-10 border border-line bg-bg px-3 text-sm font-normal normal-case tracking-normal text-fg outline-none focus:border-accent" /></label><label class="grid gap-2 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("settings.webdav.username") }}<input v-model="webdav.username" autocomplete="username" class="h-10 border border-line bg-bg px-3 text-sm font-normal normal-case tracking-normal text-fg outline-none focus:border-accent" /></label><label class="grid gap-2 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("settings.webdav.password") }}<span class="flex items-stretch gap-2"><input v-model="webdav.password" :type="showPassword ? 'text' : 'password'" autocomplete="current-password" class="h-10 min-w-0 flex-1 border border-line bg-bg px-3 text-sm font-normal normal-case tracking-normal text-fg outline-none focus:border-accent" /><button type="button" class="grid h-10 w-10 place-items-center border border-line text-dim" @click="showPassword = !showPassword"><EyeOff v-if="showPassword" :size="16" /><Eye v-else :size="16" /></button></span><span class="text-[11px] font-normal normal-case tracking-normal text-dim">{{ t("settings.webdav.passwordHint") }}</span></label><p class="flex items-center gap-2 font-mono text-[13px] uppercase tracking-[0.15em]" :class="webdav.status === 'connected' ? 'text-accent' : webdav.status === 'error' ? 'text-red-500' : 'text-dim'"><span class="h-2 w-2" :class="webdav.status === 'connected' ? 'bg-accent' : webdav.status === 'error' ? 'bg-red-500' : 'bg-dim'"></span>{{ webdavStatusLabel }}</p><div class="flex flex-wrap justify-end gap-4"><button type="button" class="ak-clip-tr h-10 border border-line px-5 text-[13px] font-semibold uppercase tracking-[0.25em] disabled:opacity-50" :disabled="writeState === 'pending'" @click="testWrite">{{ t("settings.webdav.writeTest") }}</button><button type="button" class="ak-clip-tr h-10 border border-line px-5 text-[13px] font-semibold uppercase tracking-[0.25em] disabled:opacity-50" :disabled="testing" @click="emit('test')">{{ t("settings.webdav.test") }}</button><button class="ak-clip-tr h-10 bg-accent px-5 text-[13px] font-bold uppercase tracking-[0.25em] text-accent-fg disabled:opacity-50" :disabled="webdav.saving" type="submit">{{ t("settings.webdav.save") }}</button></div></div></section>
     <LyricsSettings />
    <section class="ak-frame grid gap-6 border border-line bg-surface p-6 lg:grid-cols-[1fr_1.3fr]"><div><h2 class="text-sm font-bold uppercase tracking-[0.2em]">{{ t("settings.ipc.title") }}</h2><p class="mt-2 text-sm text-muted">{{ t("settings.ipc.desc") }}</p></div><div class="grid content-start gap-4"><div class="flex justify-end"><button type="button" class="ak-clip-tr h-9 border border-line px-4 text-[13px] font-semibold uppercase tracking-[0.2em] disabled:opacity-50" :disabled="pingState === 'pending'" @click="ping(true)">{{ t("settings.ipc.run") }}</button></div><div class="grid gap-2"><span class="text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("settings.ipc.echoLabel") }}</span><div class="flex items-stretch gap-2"><input v-model="echoInput" class="h-10 min-w-0 flex-1 border border-line bg-bg px-3 text-sm font-normal normal-case tracking-normal text-fg outline-none focus:border-accent" :placeholder="t('settings.ipc.echoPlaceholder')" /><button type="button" class="ak-clip-tr h-10 border border-line px-4 text-[13px] font-semibold uppercase tracking-[0.2em] disabled:opacity-50" :disabled="echoState === 'pending'" @click="runEcho">{{ t("settings.ipc.echoRun") }}</button></div></div></div></section>
  </form></div>
</template>
