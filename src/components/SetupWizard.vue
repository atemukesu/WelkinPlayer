<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { ArrowRight, Check, FolderOpen, HardDrive, LoaderCircle, Palette, Plus, Server, Sparkles, User } from "@lucide/vue";
import { describeError } from "../api";
import { accents } from "../lib/app";
import type { Accent, Theme } from "../lib/app";
import { pushToast } from "../lib/toast";
import { isAndroid } from "../lib/desktopLyric";
import { useProfileStore } from "../stores/profile";
import { useSourcesStore } from "../stores/sources";
import { SOURCE_KINDS } from "../lib/sources";
import type { SongSource, SourceKind } from "../lib/sources";
import LayeredSelect from "./LayeredSelect.vue";

const theme = defineModel<Theme>("theme", { required: true });
const accent = defineModel<Accent>("accent", { required: true });
const emit = defineEmits<{ finish: [] }>();

const { t } = useI18n();
const profile = useProfileStore();
const sources = useSourcesStore();
const android = isAndroid();
/** Android needs "all files access" before local folders can be listed. */
const needsFileAccess = computed(() => android && !sources.fileAccessGranted);

function refreshFileAccess() {
  if (document.visibilityState === "visible") void sources.checkFileAccess();
}

const step = ref(0);
const nickname = ref(profile.profile.nickname);
const passwordDrafts = ref<Record<string, string>>({});
const testing = ref<string | null>(null);
/** True once a server returned an existing, initialized profile. */
const existingProfile = ref(false);

const steps = [
  { key: "wizard.steps.sources", icon: Server },
  { key: "wizard.steps.welcome", icon: Sparkles },
  { key: "wizard.steps.appearance", icon: Palette },
  { key: "wizard.steps.done", icon: Check },
];

const isLast = computed(() => step.value === steps.length - 1);
const hasCloud = computed(() => sources.cloudSources.length > 0);
const canContinue = computed(() => {
  if (step.value === 0) return sources.hasSources && !testing.value;
  return true;
});

const sourceKindOptions = computed(() => SOURCE_KINDS.map((item) => ({ value: item.kind, label: t(item.labelKey) })));
const newKind = ref<SourceKind>("webdav");
function add() {
  const draft = sources.draft(newKind.value);
  void sources.addSource(draft);
  passwordDrafts.value = { ...passwordDrafts.value, [draft.id]: "" };
}

async function chooseFolder(source: SongSource) {
  try {
    if (android) {
      await sources.checkFileAccess();
      if (!sources.fileAccessGranted) {
        pushToast("warning", t("settings.sources.localPermission"));
        await sources.requestFileAccess();
        return;
      }
    }
    const folder = await sources.pickFolder();
    if (folder) await sources.updateSource(source.id, { rootPath: folder });
  } catch (error) {
    pushToast("error", describeError(error));
  }
}

async function verify(source: SongSource) {
  testing.value = source.id;
  try {
    if (source.kind === "webdav") {
      const risk = await sources.checkUrlRisk(source.url ?? "", !!source.allowInsecure);
      if (risk?.safety === "insecurePublic" && !source.allowInsecure) {
        pushToast("error", t("settings.webdav.insecurePublicBlocked"));
        return;
      }
      const warning = await sources.setPassword(source.id, passwordDrafts.value[source.id] ?? "");
      if (warning) pushToast("warning", t("settings.webdav.keychainUnavailable"));
      passwordDrafts.value = { ...passwordDrafts.value, [source.id]: "" };
    }
    await sources.testConnection(source.id);
    pushToast("success", t("settings.webdav.testOk"));
    if (source.kind === "webdav") {
      try { await sources.testWrite(source.id); } catch (error) { pushToast("warning", t("settings.webdav.writeFailed", { value: describeError(error) })); }
    }
    if (source.id === sources.syncId && source.kind === "webdav") await probeRemote();
  } catch (error) {
    pushToast("error", describeError(error));
  } finally {
    testing.value = null;
  }
}

/** Probe the sync server for an existing profile; an existing one skips setup. */
async function probeRemote() {
  const result = await profile.fetchRemote();
  if (result === "found") {
    existingProfile.value = true;
    nickname.value = profile.profile.nickname;
    pushToast("success", t("wizard.webdav.existing"));
    step.value = steps.length - 1;
  }
}

async function next() {
  if (step.value === 0 && !sources.hasSources) {
    pushToast("warning", t("wizard.sources.missing"));
    return;
  }
  if (step.value < steps.length - 1) step.value += 1;
}
function back() {
  if (existingProfile.value) return;
  if (step.value > 0) step.value -= 1;
}

async function finish() {
  if (existingProfile.value) {
    profile.finishExisting();
    emit("finish");
    return;
  }
  profile.setNickname(nickname.value);
  await sources.persist();
  await profile.completeSetup();
  emit("finish");
}

onMounted(() => {
  sources.hydrate().catch(() => {});
  if (android) {
    void sources.checkFileAccess();
    document.addEventListener("visibilitychange", refreshFileAccess);
  }
});

onBeforeUnmount(() => {
  document.removeEventListener("visibilitychange", refreshFileAccess);
});
</script>

<template>
  <div class="fixed inset-0 z-[70] flex items-center justify-center bg-bg/95 p-4 backdrop-blur-sm">
    <div class="ak-frame flex w-full max-w-2xl flex-col border border-line bg-surface shadow-2xl">
      <header class="flex items-center gap-3 border-b border-line px-6 py-5">
        <span class="grid h-10 w-10 place-items-center bg-accent text-accent-fg"><Sparkles :size="20" :stroke-width="2.2" /></span>
        <div>
          <p class="font-mono text-[10px] uppercase tracking-[0.3em] text-dim">{{ t("wizard.eyebrow") }}</p>
          <h1 class="text-lg font-black tracking-tight">{{ t("wizard.title") }}</h1>
        </div>
        <span class="ml-auto font-mono text-[11px] tabular-nums text-dim">{{ step + 1 }} / {{ steps.length }}</span>
      </header>

      <nav class="flex border-b border-line">
        <button v-for="(item, index) in steps" :key="item.key" type="button" class="flex flex-1 items-center justify-center gap-2 px-3 py-3 text-[11px] font-semibold uppercase tracking-[0.15em] transition-colors" :class="index === step ? 'bg-accent text-accent-fg' : index < step ? 'text-fg' : 'text-dim'" @click="!existingProfile && index <= step && (step = index)">
          <component :is="item.icon" :size="14" :stroke-width="2" />
          <span class="hidden sm:inline">{{ t(item.key) }}</span>
        </button>
      </nav>

      <div class="min-h-[320px] flex-1 overflow-y-auto px-6 py-6">
        <section v-if="step === 0" class="grid gap-5">
          <div>
            <h2 class="text-xl font-black tracking-tight">{{ t("wizard.sources.title") }}</h2>
            <p class="mt-2 text-sm text-muted">{{ t("wizard.sources.desc") }}</p>
          </div>

          <div class="flex flex-wrap items-center gap-2">
            <LayeredSelect v-model="newKind" :options="sourceKindOptions" :label="t('settings.sources.kind')" class="w-40" />
            <button type="button" class="ak-clip-tr inline-flex h-10 items-center gap-2 border border-line px-4 text-[12px] font-semibold uppercase tracking-[0.2em] whitespace-nowrap" @click="add"><Plus :size="14" />{{ t("settings.sources.add") }}</button>
          </div>

          <p v-if="!sources.hasSources" class="border-l-2 border-accent/60 pl-3 text-xs text-muted">{{ t("wizard.sources.empty") }}</p>
          <p v-else-if="!hasCloud" class="border-l-2 border-accent/60 pl-3 text-xs text-muted">{{ t("wizard.sources.localOnly") }}</p>

          <div v-for="source in sources.sources" :key="source.id" class="grid gap-3 border border-line bg-bg p-4">
            <div class="flex flex-wrap items-center gap-x-3 gap-y-2">
              <div class="flex min-w-[12rem] flex-1 items-center gap-3">
                <Server v-if="source.kind === 'webdav'" :size="17" class="shrink-0 text-accent" /><HardDrive v-else :size="17" class="shrink-0 text-accent" />
                <input v-model="source.name" class="h-10 min-w-0 flex-1 border border-line bg-surface px-3 text-sm outline-none focus:border-accent" :placeholder="t('settings.sources.name')" @change="sources.updateSource(source.id, { name: source.name })" />
              </div>
              <div class="ml-auto flex shrink-0 items-center gap-2">
                <button type="button" class="ak-clip-tr inline-flex h-10 items-center gap-2 border px-4 text-[12px] font-semibold uppercase tracking-[0.2em] whitespace-nowrap" :class="sources.syncId === source.id ? 'border-accent text-accent' : 'border-line text-muted'" @click="sources.setSyncId(source.id)">{{ sources.syncId === source.id ? t("settings.sources.syncCurrent") : t("settings.sources.useForSync") }}</button>
                <button type="button" class="grid h-10 w-10 place-items-center border border-line text-dim hover:text-fg" @click="sources.removeSource(source.id)">×</button>
              </div>
            </div>

            <template v-if="source.kind === 'webdav'">
              <input v-model="source.url" type="url" class="h-10 border border-line bg-surface px-3 text-sm outline-none focus:border-accent" placeholder="https://dav.example.com/music" @change="sources.updateSource(source.id, { url: source.url })" />
              <div class="grid gap-3 sm:grid-cols-2">
                <input v-model="source.username" autocomplete="username" class="h-10 border border-line bg-surface px-3 text-sm outline-none focus:border-accent" :placeholder="t('settings.webdav.username')" @change="sources.updateSource(source.id, { username: source.username })" />
                <input v-model="passwordDrafts[source.id]" type="password" autocomplete="current-password" class="h-10 border border-line bg-surface px-3 text-sm outline-none focus:border-accent" :placeholder="sources.hasPassword(source.id) ? t('settings.webdav.passwordStored') : t('settings.webdav.password')" />
              </div>
            </template>
            <template v-else>
              <span class="flex items-stretch gap-2">
                <input v-model="source.rootPath" class="h-10 min-w-0 flex-1 border border-line bg-surface px-3 text-sm outline-none focus:border-accent" :placeholder="t('settings.sources.localFolderPlaceholder')" @change="sources.updateSource(source.id, { rootPath: source.rootPath })" />
                <button type="button" class="ak-clip-tr flex h-10 shrink-0 items-center gap-2 border border-line px-4 text-[12px] font-semibold uppercase tracking-[0.2em] whitespace-nowrap" @click="chooseFolder(source)"><FolderOpen :size="14" />{{ t("settings.sources.chooseFolder") }}</button>
              </span>
              <p v-if="needsFileAccess" class="flex flex-wrap items-center gap-2 border-l-2 border-accent/60 pl-3 text-xs text-muted">
                {{ t("settings.sources.localPermission") }}
                <button type="button" class="ak-clip-tr inline-flex h-8 items-center border border-line px-3 text-[11px] font-semibold uppercase tracking-[0.2em] whitespace-nowrap" @click="sources.requestFileAccess()">{{ t("settings.sources.grantPermission") }}</button>
              </p>
            </template>

            <div class="flex justify-end">
              <button type="button" class="ak-clip-tr flex h-10 items-center gap-2 border border-line px-5 text-[12px] font-semibold uppercase tracking-[0.2em] whitespace-nowrap" :disabled="testing === source.id" @click="verify(source)"><LoaderCircle v-if="testing === source.id" :size="14" class="animate-spin" />{{ t("wizard.sources.verify") }}</button>
            </div>
          </div>

          <p class="text-[12px] text-dim">{{ t("wizard.sources.hint") }}</p>
        </section>

        <section v-else-if="step === 1" class="grid gap-5">
          <div>
            <h2 class="text-xl font-black tracking-tight">{{ t("wizard.welcome.title") }}</h2>
            <p class="mt-2 text-sm text-muted">{{ t("wizard.welcome.desc") }}</p>
          </div>
          <label class="grid gap-2 text-[13px] font-semibold text-dim">
            {{ t("wizard.welcome.nickname") }}
            <span class="flex items-center gap-2 border border-line bg-bg px-3">
              <User :size="16" class="text-dim" />
              <input v-model="nickname" class="h-10 min-w-0 flex-1 bg-transparent text-sm text-fg outline-none" :placeholder="t('wizard.welcome.nicknamePlaceholder')" @keydown.enter="next" />
            </span>
          </label>
        </section>

        <section v-else-if="step === 2" class="grid gap-6">
          <div>
            <h2 class="text-xl font-black tracking-tight">{{ t("wizard.appearance.title") }}</h2>
            <p class="mt-2 text-sm text-muted">{{ t("wizard.appearance.desc") }}</p>
          </div>
          <div class="grid gap-5">
            <div class="flex items-center justify-between gap-4">
              <span class="text-[13px] font-semibold text-dim">{{ t("theme.label") }}</span>
              <div class="inline-flex border border-line">
                <button type="button" class="h-9 px-4 text-[13px] font-semibold" :class="theme === 'light' ? 'bg-accent text-accent-fg' : 'text-muted hover:text-fg'" @click="theme = 'light'">{{ t("theme.light") }}</button>
                <button type="button" class="h-9 border-l border-line px-4 text-[13px] font-semibold" :class="theme === 'dark' ? 'bg-accent text-accent-fg' : 'text-muted hover:text-fg'" @click="theme = 'dark'">{{ t("theme.dark") }}</button>
              </div>
            </div>
            <div class="flex items-center justify-between gap-4">
              <span class="text-[13px] font-semibold text-dim">{{ t("accent.label") }}</span>
              <div class="flex items-center gap-2">
                <button v-for="item in accents" :key="item.id" type="button" class="h-6 w-6 border-2 transition-transform hover:scale-110" :style="{ backgroundColor: item.swatch, borderColor: accent === item.id ? 'var(--fg)' : 'transparent' }" :title="t(item.key)" @click="accent = item.id"></button>
              </div>
            </div>
          </div>
        </section>

        <section v-else class="grid place-items-center gap-4 py-8 text-center">
          <span class="grid h-16 w-16 place-items-center bg-accent text-accent-fg"><Check :size="30" :stroke-width="2.4" /></span>
          <h2 class="text-xl font-black tracking-tight">{{ existingProfile ? t("wizard.done.existingTitle") : t("wizard.done.title") }}</h2>
          <p class="max-w-md text-sm text-muted">{{ existingProfile ? t("wizard.done.existingDesc") : t("wizard.done.desc", { name: nickname || t("wizard.done.friend") }) }}</p>
          <p v-if="!hasCloud" class="max-w-md text-xs text-dim">{{ t("wizard.sources.localOnly") }}</p>
        </section>
      </div>

      <footer class="flex items-center justify-between gap-3 border-t border-line px-6 py-4">
        <button type="button" class="h-10 px-4 text-[13px] font-semibold text-muted transition-colors hover:text-fg disabled:opacity-40" :disabled="step === 0 || existingProfile" @click="back">{{ t("wizard.back") }}</button>
        <button v-if="!isLast" type="button" class="ak-clip-tr flex h-10 items-center gap-2 bg-accent px-5 text-[13px] font-bold text-accent-fg disabled:opacity-50" :disabled="!canContinue" @click="next">
          {{ t("wizard.next") }}<ArrowRight :size="16" />
        </button>
        <button v-else type="button" class="ak-clip-tr flex h-10 items-center gap-2 bg-accent px-5 text-[13px] font-bold text-accent-fg" @click="finish">
          {{ t("wizard.finish") }}<Check :size="16" />
        </button>
      </footer>
    </div>
  </div>
</template>
