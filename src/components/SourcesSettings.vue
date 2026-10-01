<script setup lang="ts">
import { reactive } from "vue";
import { Check, FolderOpen, HardDrive, LoaderCircle, Plus, Server, Trash2 } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { describeError } from "../api";
import { pushToast } from "../lib/toast";
import { useSourcesStore } from "../stores/sources";
import type { SongSource } from "../lib/sources";

const { t } = useI18n();
const sources = useSourcesStore();

const passwordDrafts = reactive<Record<string, string>>({});
const status = reactive<Record<string, { testing: boolean; writeTesting: boolean; readOk: boolean; writeOk: boolean }>>({});

function stateFor(id: string) {
  status[id] ??= { testing: false, writeTesting: false, readOk: false, writeOk: false };
  return status[id];
}

async function add(kind: "webdav" | "local") {
  const draft = sources.draft(kind);
  await sources.addSource(draft);
  passwordDrafts[draft.id] = "";
}

async function chooseFolder(source: SongSource) {
  try {
    const folder = await sources.pickFolder();
    if (!folder) return;
    await sources.updateSource(source.id, { rootPath: folder });
  } catch (error) {
    pushToast("error", describeError(error));
  }
}

async function saveWebdav(source: SongSource) {
  const risk = await sources.checkUrlRisk(source.url ?? "", !!source.allowInsecure);
  if (risk?.safety === "insecurePublic" && !source.allowInsecure) {
    pushToast("error", t("settings.webdav.insecurePublicBlocked"));
    return;
  }
  const warning = await sources.setPassword(source.id, passwordDrafts[source.id] ?? "");
  if (warning) pushToast("warning", t("settings.webdav.keychainUnavailable"));
  passwordDrafts[source.id] = "";
  pushToast("success", t("settings.saved"));
}

async function test(source: SongSource) {
  const current = stateFor(source.id);
  current.testing = true;
  current.readOk = false;
  try {
    await sources.testConnection(source.id);
    current.readOk = true;
    current.writeOk = false;
    if (source.kind === "webdav") {
      current.writeTesting = true;
      try {
        await sources.testWrite(source.id);
        current.writeOk = true;
      } catch (error) {
        pushToast("error", t("settings.webdav.writeFailed", { value: describeError(error) }));
      } finally {
        current.writeTesting = false;
      }
    }
    pushToast("success", t("settings.webdav.testOk"));
  } catch (error) {
    pushToast("error", describeError(error));
  } finally {
    current.testing = false;
  }
}

async function remove(source: SongSource) {
  await sources.removeSource(source.id);
  delete passwordDrafts[source.id];
}
</script>

<template>
  <section class="ak-frame grid gap-6 border border-line bg-surface p-6">
    <div class="flex flex-wrap items-start justify-between gap-4">
      <div>
        <h2 class="text-sm font-bold uppercase tracking-[0.2em]">{{ t("settings.sources.title") }}</h2>
        <p class="mt-2 max-w-xl text-sm text-muted">{{ t("settings.sources.desc") }}</p>
      </div>
      <div class="flex items-center gap-2">
        <button type="button" class="ak-clip-tr flex h-9 items-center gap-2 border border-line px-3 text-[12px] font-semibold uppercase tracking-[0.15em]" @click="add('webdav')"><Plus :size="14" /><Server :size="14" />{{ t("settings.sources.addWebdav") }}</button>
        <button type="button" class="ak-clip-tr flex h-9 items-center gap-2 border border-line px-3 text-[12px] font-semibold uppercase tracking-[0.15em]" @click="add('local')"><Plus :size="14" /><HardDrive :size="14" />{{ t("settings.sources.addLocal") }}</button>
      </div>
    </div>

    <p v-if="!sources.hasCloudSync" class="border-l-2 border-accent/60 pl-3 text-xs text-muted">{{ t("settings.sources.localOnly") }}</p>
    <p v-if="sources.hasSources && !sources.syncId" class="border-l-2 border-accent/60 pl-3 text-xs text-muted">{{ t("settings.sources.syncMissing") }}</p>

    <p v-if="sources.sources.length === 0" class="text-sm text-muted">{{ t("settings.sources.empty") }}</p>

    <div v-else class="grid gap-4">
      <article v-for="source in sources.sources" :key="source.id" class="grid gap-4 border border-line bg-bg p-4">
        <div class="flex flex-wrap items-center gap-3">
          <span class="grid h-9 w-9 shrink-0 place-items-center bg-accent/10 text-accent"><Server v-if="source.kind === 'webdav'" :size="16" /><HardDrive v-else :size="16" /></span>
          <input v-model="source.name" class="h-9 min-w-0 flex-1 border border-line bg-surface px-2 text-sm outline-none focus:border-accent" :placeholder="t('settings.sources.name')" @change="sources.updateSource(source.id, { name: source.name })" />
          <button type="button" class="ak-clip-tr h-9 shrink-0 border px-3 text-[11px] font-semibold uppercase tracking-[0.15em]" :class="sources.syncId === source.id ? 'border-accent text-accent' : 'border-line text-muted hover:text-fg'" :title="t('settings.sources.useForSync')" @click="sources.setSyncId(source.id)"><Check v-if="sources.syncId === source.id" :size="13" />{{ sources.syncId === source.id ? t("settings.sources.syncCurrent") : t("settings.sources.useForSync") }}</button>
          <button type="button" class="grid h-9 w-9 shrink-0 place-items-center border border-line text-dim transition-colors hover:border-accent hover:text-accent" :title="t('settings.sources.remove')" @click="remove(source)"><Trash2 :size="15" /></button>
        </div>

        <template v-if="source.kind === 'webdav'">
          <div class="grid gap-3 sm:grid-cols-2">
            <label class="grid gap-1 text-[12px] font-semibold text-dim">{{ t("settings.webdav.serverUrl") }}<input v-model="source.url" type="url" class="h-9 border border-line bg-surface px-2 text-sm font-normal text-fg outline-none focus:border-accent" placeholder="https://dav.example.com/music" @change="sources.updateSource(source.id, { url: source.url })" /></label>
            <label class="grid gap-1 text-[12px] font-semibold text-dim">{{ t("settings.webdav.username") }}<input v-model="source.username" autocomplete="username" class="h-9 border border-line bg-surface px-2 text-sm font-normal text-fg outline-none focus:border-accent" @change="sources.updateSource(source.id, { username: source.username })" /></label>
          </div>
          <div class="grid gap-3 sm:grid-cols-2">
            <label class="grid gap-1 text-[12px] font-semibold text-dim">{{ t("settings.webdav.password") }}<input v-model="passwordDrafts[source.id]" type="password" autocomplete="current-password" class="h-9 border border-line bg-surface px-2 text-sm font-normal text-fg outline-none focus:border-accent" :placeholder="sources.hasPassword(source.id) ? t('settings.webdav.passwordStored') : ''" /></label>
            <label class="flex items-end gap-2 pb-1 text-[12px] font-semibold text-dim"><input v-model="source.allowInsecure" type="checkbox" class="ak-check" @change="sources.updateSource(source.id, { allowInsecure: source.allowInsecure })" />{{ t("settings.webdav.allowInsecure") }}</label>
          </div>
          <div class="flex flex-wrap items-center gap-3">
            <button type="button" class="ak-clip-tr h-9 border border-line px-4 text-[12px] font-semibold" :disabled="stateFor(source.id).testing" @click="test(source)"><LoaderCircle v-if="stateFor(source.id).testing" :size="14" class="animate-spin" />{{ t("settings.sources.test") }}</button>
            <button type="button" class="ak-clip-tr h-9 border border-line px-4 text-[12px] font-semibold" @click="saveWebdav(source)">{{ t("settings.sources.savePassword") }}</button>
            <span v-if="stateFor(source.id).readOk" class="flex items-center gap-1.5 text-[12px] font-semibold text-accent"><Check :size="13" />{{ t("settings.sources.readOk") }}</span>
            <span v-if="stateFor(source.id).writeOk" class="flex items-center gap-1.5 text-[12px] font-semibold text-accent"><Check :size="13" />{{ t("settings.sources.writeOk") }}</span>
          </div>
        </template>

        <template v-else>
          <label class="grid gap-1 text-[12px] font-semibold text-dim">{{ t("settings.sources.localFolder") }}<span class="flex items-stretch gap-2"><input v-model="source.rootPath" class="h-9 min-w-0 flex-1 border border-line bg-surface px-2 text-sm font-normal text-fg outline-none focus:border-accent" :placeholder="t('settings.sources.localFolderPlaceholder')" @change="sources.updateSource(source.id, { rootPath: source.rootPath })" /><button type="button" class="ak-clip-tr flex h-9 shrink-0 items-center gap-2 border border-line px-3 text-[12px] font-semibold" @click="chooseFolder(source)"><FolderOpen :size="14" />{{ t("settings.sources.chooseFolder") }}</button></span></label>
          <div class="flex flex-wrap items-center gap-3">
            <button type="button" class="ak-clip-tr h-9 border border-line px-4 text-[12px] font-semibold" :disabled="stateFor(source.id).testing" @click="test(source)"><LoaderCircle v-if="stateFor(source.id).testing" :size="14" class="animate-spin" />{{ t("settings.sources.test") }}</button>
            <span v-if="stateFor(source.id).readOk" class="flex items-center gap-1.5 text-[12px] font-semibold text-accent"><Check :size="13" />{{ t("settings.sources.readOk") }}</span>
          </div>
        </template>
      </article>
    </div>

    <p class="flex items-center gap-2 text-[11px] text-dim"><span class="h-2 w-2" :class="sources.hasCloudSync ? 'bg-accent' : 'bg-dim'"></span>{{ sources.hasCloudSync ? t("settings.sources.cloudAvailable") : t("settings.sources.cloudUnavailable") }}</p>
  </section>
</template>
