<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { ArrowRight, Check, Eye, EyeOff, HardDrive, LoaderCircle, Palette, Server, Sparkles, User } from "@lucide/vue";
import { describeError, invoke } from "../api";
import { accents } from "../lib/app";
import type { Accent, Theme } from "../lib/app";
import { pushToast } from "../lib/toast";
import { useProfileStore } from "../stores/profile";
import { useWebdavStore } from "../stores/webdav";

const theme = defineModel<Theme>("theme", { required: true });
const accent = defineModel<Accent>("accent", { required: true });
const emit = defineEmits<{ finish: [] }>();

const { t } = useI18n();
const profile = useProfileStore();
const webdav = useWebdavStore();

const step = ref(0);
const nickname = ref(profile.profile.nickname);
const showPassword = ref(false);
const testing = ref(false);
const writeTesting = ref(false);
const connectionOk = ref(false);
const writeOk = ref(false);
/** True once the server returned an existing, initialized profile. */
const existingProfile = ref(false);

const steps = [
  { key: "wizard.steps.webdav", icon: Server },
  { key: "wizard.steps.welcome", icon: Sparkles },
  { key: "wizard.steps.appearance", icon: Palette },
  { key: "wizard.steps.done", icon: Check },
];

const webdavConfigured = computed(() => webdav.url.trim().length > 0 && webdav.username.trim().length > 0);
const isLast = computed(() => step.value === steps.length - 1);
const canContinue = computed(() => {
  if (step.value === 0) return !testing.value && !writeTesting.value;
  return true;
});

async function next() {
  // Ask the server first, then decide: an existing profile skips the rest.
  if (step.value === 0 && webdavConfigured.value && !connectionOk.value) {
    await verifyWebdav();
    if (!connectionOk.value) return;
  }
  if (step.value < steps.length - 1) step.value += 1;
}
function back() {
  if (existingProfile.value) return;
  if (step.value > 0) step.value -= 1;
}

async function verifyWebdav() {
  if (!webdavConfigured.value) {
    pushToast("warning", t("wizard.webdav.missing"));
    return;
  }
  testing.value = true;
  connectionOk.value = false;
  writeOk.value = false;
  try {
    const risk = await webdav.checkUrlRisk().catch(() => null);
    if (risk?.safety === "insecurePublic" && !webdav.allowInsecure) {
      pushToast("error", t("settings.webdav.insecurePublicBlocked"));
      return;
    }
    const { warning } = await webdav.persist();
    if (webdav.error) {
      pushToast("error", t("settings.webdav.saveFailed", { value: webdav.error }));
      return;
    }
    if (warning) pushToast("warning", t("settings.webdav.keychainUnavailable"));
    if (risk?.safety === "insecurePrivate" && !webdav.allowInsecure) {
      pushToast("warning", t("settings.webdav.insecureWarning"));
    }
    await invoke<string>("test_webdav_connection");
    connectionOk.value = true;
    pushToast("success", t("settings.webdav.testOk"));
    await verifyWrite();
    await probeRemote();
  } catch (error) {
    pushToast("error", describeError(error));
  } finally {
    testing.value = false;
  }
}

async function verifyWrite() {
  writeTesting.value = true;
  try {
    await invoke<string>("test_webdav_write");
    writeOk.value = true;
    pushToast("success", t("settings.webdav.writeOk"));
  } catch (error) {
    pushToast("error", t("settings.webdav.writeFailed", { value: describeError(error) }));
  } finally {
    writeTesting.value = false;
  }
}

/**
 * Probe the server for an existing profile. When one is found it is loaded and
 * the wizard jumps to the last step, leaving the remote document untouched.
 */
async function probeRemote() {
  const result = await profile.fetchRemote();
  if (result === "found") {
    existingProfile.value = true;
    nickname.value = profile.profile.nickname;
    pushToast("success", t("wizard.webdav.existing"));
    step.value = steps.length - 1;
  } else if (result === "error") {
    pushToast("warning", t("wizard.webdav.probeFailed"));
  }
}

async function finish() {
  // An existing profile is already authoritative; closing must not rewrite it.
  if (existingProfile.value) {
    profile.finishExisting();
    emit("finish");
    return;
  }
  profile.setNickname(nickname.value);
  if (webdavConfigured.value && !webdav.saving) await webdav.persist();
  await profile.completeSetup();
  emit("finish");
}

onMounted(() => {
  // Credentials restored from a previous session: probe without asking again.
  if (webdavConfigured.value && webdav.hasStoredPassword && !connectionOk.value) void verifyWebdav();
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
        <button
          v-for="(item, index) in steps"
          :key="item.key"
          type="button"
          class="flex flex-1 items-center justify-center gap-2 px-3 py-3 text-[11px] font-semibold uppercase tracking-[0.15em] transition-colors"
          :class="index === step ? 'bg-accent text-accent-fg' : index < step ? 'text-fg' : 'text-dim'"
          @click="!existingProfile && index <= step && (step = index)"
        >
          <component :is="item.icon" :size="14" :stroke-width="2" />
          <span class="hidden sm:inline">{{ t(item.key) }}</span>
        </button>
      </nav>

      <div class="min-h-[288px] flex-1 px-6 py-6">
        <section v-if="step === 0" class="grid gap-5">
          <div>
            <h2 class="text-xl font-black tracking-tight">{{ t("wizard.webdav.title") }}</h2>
            <p class="mt-2 text-sm text-muted">{{ t("wizard.webdav.desc") }}</p>
          </div>
          <label class="grid gap-2 text-[13px] font-semibold text-dim">
            {{ t("settings.webdav.serverUrl") }}
            <input v-model="webdav.url" type="url" class="h-10 border border-line bg-bg px-3 text-sm text-fg outline-none focus:border-accent" placeholder="https://dav.example.com/music" @input="connectionOk = false; writeOk = false" />
          </label>
          <div class="grid gap-4 sm:grid-cols-2">
            <label class="grid gap-2 text-[13px] font-semibold text-dim">
              {{ t("settings.webdav.username") }}
              <input v-model="webdav.username" autocomplete="username" class="h-10 border border-line bg-bg px-3 text-sm text-fg outline-none focus:border-accent" @input="connectionOk = false; writeOk = false" />
            </label>
            <label class="grid gap-2 text-[13px] font-semibold text-dim">
              {{ t("settings.webdav.password") }}
              <span class="flex items-stretch gap-2">
                <input v-model="webdav.password" :type="showPassword ? 'text' : 'password'" autocomplete="current-password" class="h-10 min-w-0 flex-1 border border-line bg-bg px-3 text-sm text-fg outline-none focus:border-accent" :placeholder="webdav.hasStoredPassword ? t('settings.webdav.passwordStored') : ''" @input="webdav.markPasswordTouched(); connectionOk = false; writeOk = false" />
                <button v-if="webdav.hasStoredPassword" type="button" class="grid h-10 shrink-0 place-items-center border border-line px-3 text-[11px] text-dim hover:text-fg" @click="webdav.clearPassword()">{{ t("settings.webdav.passwordClear") }}</button>
                <button type="button" class="grid h-10 w-10 place-items-center border border-line text-dim hover:text-fg" @click="showPassword = !showPassword">
                  <EyeOff v-if="showPassword" :size="16" />
                  <Eye v-else :size="16" />
                </button>
              </span>
            </label>
          </div>
          <label class="flex items-center gap-2 text-[12px] font-semibold text-dim"><input v-model="webdav.allowInsecure" type="checkbox" class="ak-check" />{{ t("settings.webdav.allowInsecure") }}</label>
          <div class="flex flex-wrap items-center gap-3">
            <button type="button" class="ak-clip-tr flex h-10 items-center gap-2 border border-line px-4 text-[13px] font-semibold disabled:opacity-50" :disabled="testing || writeTesting" @click="verifyWebdav">
              <LoaderCircle v-if="testing || writeTesting" :size="15" class="animate-spin" />
              <HardDrive v-else :size="15" />
              {{ t("wizard.webdav.verify") }}
            </button>
            <span v-if="connectionOk" class="flex items-center gap-1.5 text-[12px] font-semibold text-accent"><Check :size="14" />{{ t("wizard.webdav.readOk") }}</span>
            <span v-if="writeOk" class="flex items-center gap-1.5 text-[12px] font-semibold text-accent"><Check :size="14" />{{ t("wizard.webdav.writeOkShort") }}</span>
          </div>
          <p class="text-[12px] text-dim">{{ t("wizard.webdav.hint") }}</p>
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
