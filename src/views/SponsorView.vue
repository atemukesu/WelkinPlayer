<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { ArrowLeft, BadgeCheck, Check, Coffee, Copy, ExternalLink, HeartHandshake, KeyRound, LoaderCircle, X } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { openUrl } from "@tauri-apps/plugin-opener";
import { describeError, invoke, toAppError } from "../api";
import { pushToast } from "../lib/toast";
import { useLicenseStore } from "../stores/license";

const emit = defineEmits<{ back: [] }>();
const { t } = useI18n();
const license = useLicenseStore();
const sponsorUrl = "https://ifdian.net/a/atommix";

const claimOpen = ref(false);
const claimLoading = ref(false);
const claimResult = ref("");
const claimError = ref("");

const activateOpen = ref(false);
const activationCode = ref("");
const activationLoading = ref(false);
const activationError = ref("");

/** Maps stable Rust error codes to localized activation messages. */
const ACTIVATION_ERROR_KEYS: Record<string, string> = {
  LICENSE_SIGNER: "sponsor.activateModal.invalidSigner",
  LICENSE_DEVICE: "sponsor.activateModal.deviceMismatch",
  LICENSE_EXPIRED: "sponsor.activateModal.expired",
  LICENSE_TIER: "sponsor.activateModal.invalidTier",
};

/** Formatted expiry date, or an em dash when unknown. */
const activationExpiry = computed(() => {
  const seconds = license.status.expiresAt;
  if (!seconds) return "—";
  return new Date(seconds * 1000).toLocaleDateString();
});

async function openSponsor() {
  try {
    await openUrl(sponsorUrl);
  } catch {
    try {
      window.open(sponsorUrl, "_blank", "noopener,noreferrer");
    } catch {
      pushToast("error", t("sponsor.openFailed"));
    }
  }
}

function openClaim() {
  claimResult.value = "";
  claimError.value = "";
  claimLoading.value = false;
  claimOpen.value = true;
}

/** Ask Rust to assemble the single-line JSON claim; nothing is built here. */
async function generateClaim() {
  if (claimLoading.value) return;
  claimLoading.value = true;
  claimError.value = "";
  try {
    claimResult.value = await invoke<string>("build_sponsor_claim");
  } catch (error) {
    claimError.value = t("sponsor.claimModal.failed", { value: describeError(error) });
  } finally {
    claimLoading.value = false;
  }
}

async function copyClaim() {
  if (!claimResult.value) return;
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(claimResult.value);
    } else {
      throw new Error("clipboard unavailable");
    }
    pushToast("success", t("sponsor.claimModal.copied"));
  } catch {
    const field = document.createElement("textarea");
    field.value = claimResult.value;
    field.style.position = "fixed";
    field.style.opacity = "0";
    document.body.appendChild(field);
    field.select();
    const copied = document.execCommand("copy");
    document.body.removeChild(field);
    pushToast(copied ? "success" : "error", t(copied ? "sponsor.claimModal.copied" : "sponsor.claimModal.copyFailed"));
  }
}

function openActivate() {
  activationCode.value = "";
  activationError.value = "";
  activateOpen.value = true;
}

/** Verify the code in Rust and, on success, persist and reflect Pro status. */
async function submitActivation() {
  const code = activationCode.value.trim();
  if (!code || activationLoading.value) return;
  activationLoading.value = true;
  activationError.value = "";
  try {
    const status = await license.activate(code);
    pushToast("success", t("sponsor.activateModal.success", { signer: status.signer ?? "" }));
    activationCode.value = "";
    activateOpen.value = false;
  } catch (error) {
    const appError = toAppError(error);
    const key = ACTIVATION_ERROR_KEYS[appError.code];
    activationError.value = key
      ? t(key)
      : t("sponsor.activateModal.failed", { value: appError.message });
  } finally {
    activationLoading.value = false;
  }
}

onMounted(() => {
  void license.refresh();
});
</script>

<template>
  <div class="mx-auto w-full max-w-4xl p-6 lg:p-8">
    <button type="button" class="flex items-center gap-2 text-[13px] font-semibold uppercase tracking-[0.25em] text-dim transition-colors hover:text-fg" @click="emit('back')"><ArrowLeft :size="15" :stroke-width="2" />{{ t("controls.return") }}</button>

    <header class="mt-6 border-b border-line pb-6">
      <p class="flex items-center gap-2 text-[13px] font-semibold uppercase tracking-[0.35em] text-accent"><span class="h-2 w-2 bg-accent"></span>{{ t("sponsor.eyebrow") }}</p>
      <h1 class="mt-4 text-4xl font-black uppercase leading-none tracking-tight sm:text-5xl">{{ t("sponsor.title") }}</h1>
      <p class="mt-3 max-w-2xl text-sm leading-[1.7] text-muted">{{ t("sponsor.subtitle") }}</p>
    </header>

    <section class="ak-frame relative mt-8 overflow-hidden border border-line bg-surface">
      <div class="pointer-events-none absolute inset-0 opacity-[0.18]" style="background: radial-gradient(circle at 90% 4%, var(--accent), transparent 55%)"></div>
      <div class="relative grid gap-8 p-8 lg:grid-cols-[auto_1fr] lg:items-center">
        <span class="grid h-24 w-24 shrink-0 place-items-center bg-accent text-accent-fg"><HeartHandshake :size="42" :stroke-width="2" /></span>
        <div class="min-w-0">
          <h2 class="text-3xl font-black uppercase leading-none tracking-tight sm:text-4xl">{{ t("sponsor.cardTitle") }}</h2>
          <p class="mt-3 max-w-lg text-sm leading-[1.7] text-muted">{{ t("sponsor.cardDesc") }}</p>
        </div>
      </div>
    </section>

    <section class="mt-8 grid gap-4 sm:grid-cols-3">
      <div v-for="item in ['free', 'noAds', 'dev']" :key="item" class="ak-frame flex items-start gap-3 border border-line bg-surface p-4">
        <span class="mt-0.5 grid h-6 w-6 shrink-0 place-items-center bg-accent/10 text-accent"><Check :size="14" :stroke-width="2.4" /></span>
        <p class="text-sm leading-relaxed text-muted">{{ t(`sponsor.points.${item}`) }}</p>
      </div>
    </section>

    <section class="ak-frame mt-8 overflow-hidden border border-line bg-surface">
      <div class="border-b border-line px-6 py-4">
        <h2 class="flex items-center gap-3 text-sm font-bold uppercase tracking-[0.25em]"><span class="h-3 w-1 bg-accent"></span>{{ t("sponsor.compare.title") }}</h2>
      </div>
      <table class="w-full border-collapse text-left text-sm">
        <thead>
          <tr class="border-b border-line text-[11px] uppercase tracking-[0.2em] text-dim">
            <th class="px-6 py-3 font-semibold">{{ t("sponsor.compare.feature") }}</th>
            <th class="w-24 px-3 py-3 text-center font-semibold">{{ t("sponsor.compare.free") }}</th>
            <th class="w-24 px-3 py-3 text-center font-semibold text-accent">{{ t("sponsor.compare.pro") }}</th>
          </tr>
        </thead>
        <tbody>
          <tr class="border-b border-line">
            <td class="px-6 py-3 text-muted">{{ t("sponsor.compare.rows.thanks") }}</td>
            <td class="px-3 py-3 text-center text-dim">—</td>
            <td class="px-3 py-3 text-center text-accent"><Check class="mx-auto" :size="16" :stroke-width="2.4" /></td>
          </tr>
          <tr class="border-b border-line">
            <td class="px-6 py-3 text-muted">{{ t("sponsor.compare.rows.splash") }}</td>
            <td class="px-3 py-3 text-center text-dim">—</td>
            <td class="px-3 py-3 text-center text-accent"><Check class="mx-auto" :size="16" :stroke-width="2.4" /></td>
          </tr>
          <tr>
            <td class="px-6 py-3 text-muted">{{ t("sponsor.compare.rows.badge") }}</td>
            <td class="px-3 py-3 text-center font-semibold text-dim">{{ t("sponsor.compare.badgeFree") }}</td>
            <td class="px-3 py-3 text-center font-bold text-accent">{{ t("sponsor.compare.badgePro") }}</td>
          </tr>
        </tbody>
      </table>
      <p class="border-t border-line px-6 py-4 text-xs text-dim">{{ t("sponsor.compare.proNote") }}</p>
    </section>

    <section class="ak-frame mt-8 grid gap-6 border border-line bg-surface p-8">
      <div class="flex items-center gap-4">
        <span class="grid h-12 w-12 shrink-0 place-items-center bg-accent/10 text-accent"><Coffee :size="24" :stroke-width="2" /></span>
        <div class="min-w-0">
          <h2 class="text-lg font-bold uppercase tracking-[0.15em]">{{ t("sponsor.ctaTitle") }}</h2>
          <p class="mt-1 text-sm text-muted">{{ t("sponsor.ctaDesc") }}</p>
        </div>
      </div>
      <div class="flex flex-col gap-3 sm:flex-row sm:items-center">
        <button type="button" class="ak-clip-tr flex h-12 items-center justify-center gap-2 bg-accent px-6 text-[13px] font-bold uppercase tracking-[0.25em] text-accent-fg transition-transform hover:scale-[1.01] active:scale-95" @click="openClaim">{{ t("sponsor.claim") }}<KeyRound :size="15" :stroke-width="2.2" /></button>
        <button type="button" class="ak-clip-tr flex h-12 items-center justify-center gap-2 border border-line px-6 text-[13px] font-bold uppercase tracking-[0.25em] text-fg transition-colors hover:border-accent hover:text-accent" @click="openActivate">{{ license.isPro ? t("sponsor.activateModal.activeTitle") : t("sponsor.activate") }}<BadgeCheck v-if="license.isPro" :size="15" :stroke-width="2.4" /><Check v-else :size="15" :stroke-width="2.4" /></button>
      </div>
      <button type="button" class="flex w-fit max-w-full items-center gap-1.5 truncate font-mono text-xs tracking-wide text-dim transition-colors hover:text-fg" @click="openSponsor"><span class="truncate">{{ sponsorUrl }}</span><ExternalLink class="shrink-0" :size="13" :stroke-width="2" /></button>
    </section>

    <Teleport to="body">
      <Transition name="modal">
        <div v-if="claimOpen" class="fixed inset-0 z-[80] grid place-items-center bg-black/70 p-4" role="dialog" aria-modal="true" @pointerdown.self="claimOpen = false">
        <div class="ak-frame w-full max-w-md border border-line bg-surface p-5">
          <div class="flex items-center justify-between">
            <h3 class="text-sm font-bold uppercase tracking-[0.2em]">{{ t("sponsor.claimModal.title") }}</h3>
            <button type="button" class="grid h-8 w-8 place-items-center text-dim hover:text-fg" @click="claimOpen = false"><X :size="16" /></button>
          </div>

          <template v-if="!claimResult">
            <p class="mt-3 text-sm leading-relaxed text-muted">{{ t("sponsor.claimModal.intro") }}</p>
            <ul class="mt-3 grid gap-1.5 text-sm leading-relaxed text-muted">
              <li>{{ t("sponsor.claimModal.infoOs") }}</li>
              <li>{{ t("sponsor.claimModal.infoName") }}</li>
            </ul>
            <p v-if="claimError" class="mt-3 text-xs leading-relaxed text-accent">{{ claimError }}</p>
            <button type="button" class="ak-clip-tr mt-5 flex h-11 w-full items-center justify-center gap-2 bg-accent px-6 text-[13px] font-bold uppercase tracking-[0.25em] text-accent-fg transition-transform hover:scale-[1.01] active:scale-95 disabled:opacity-50" :disabled="claimLoading" @click="generateClaim">
              <LoaderCircle v-if="claimLoading" class="animate-spin" :size="15" :stroke-width="2.2" /><KeyRound v-else :size="15" :stroke-width="2.2" />
              {{ claimLoading ? t("sponsor.claimModal.generating") : t("sponsor.claimModal.generate") }}
            </button>
          </template>

          <template v-else>
            <p class="mt-3 text-sm leading-relaxed text-muted">{{ t("sponsor.claimModal.resultHint") }}</p>
            <pre class="mt-3 max-h-48 overflow-auto whitespace-pre-wrap break-all border border-line bg-bg p-3 font-mono text-xs leading-relaxed text-fg">{{ claimResult }}</pre>
            <button type="button" class="ak-clip-tr mt-4 flex h-11 w-full items-center justify-center gap-2 bg-accent px-6 text-[13px] font-bold uppercase tracking-[0.25em] text-accent-fg transition-transform hover:scale-[1.01] active:scale-95" @click="copyClaim"><Copy :size="15" :stroke-width="2.2" />{{ t("sponsor.claimModal.copy") }}</button>
            <button type="button" class="ak-clip-tr mt-4 flex h-11 w-full items-center justify-center gap-2 border border-line px-6 text-[13px] font-bold uppercase tracking-[0.25em] text-fg transition-colors hover:border-accent hover:text-accent" @click="openSponsor">{{ t("sponsor.claimModal.openSponsor") }}<ExternalLink :size="15" :stroke-width="2.2" /></button>
          </template>
        </div>
      </div>
      </Transition>
    </Teleport>

    <Teleport to="body">
      <Transition name="modal">
      <div v-if="activateOpen" class="fixed inset-0 z-[80] grid place-items-center bg-black/70 p-4" role="dialog" aria-modal="true" @pointerdown.self="activateOpen = false">
        <div class="ak-frame w-full max-w-md border border-line bg-surface p-5">
          <div class="flex items-center justify-between">
            <h3 class="text-sm font-bold uppercase tracking-[0.2em]">{{ t("sponsor.activateModal.title") }}</h3>
            <button type="button" class="grid h-8 w-8 place-items-center text-dim hover:text-fg" @click="activateOpen = false"><X :size="16" /></button>
          </div>
          <template v-if="license.isPro">
            <div class="mt-3 flex items-center gap-3 border border-accent/40 bg-accent/10 px-4 py-3 text-accent">
              <BadgeCheck :size="20" :stroke-width="2.2" />
              <span class="text-sm font-bold uppercase tracking-[0.15em]">{{ t("sponsor.activateModal.activeTitle") }}</span>
            </div>
            <p class="mt-3 text-sm leading-relaxed text-muted">{{ t("sponsor.activateModal.activeDesc") }}</p>
            <dl class="mt-3 grid gap-1.5 text-sm">
              <div class="flex justify-between gap-4"><dt class="text-dim">{{ t("sponsor.activateModal.signer") }}</dt><dd class="min-w-0 truncate font-semibold">{{ license.status.signer ?? "—" }}</dd></div>
              <div class="flex justify-between gap-4"><dt class="text-dim">{{ t("sponsor.activateModal.expires") }}</dt><dd class="font-semibold tabular-nums">{{ activationExpiry }}</dd></div>
            </dl>
          </template>
          <template v-else>
            <p class="mt-3 text-sm leading-relaxed text-muted">{{ t("sponsor.activateModal.desc") }}</p>
            <input v-model="activationCode" class="mt-3 h-11 w-full border border-line bg-bg px-3 font-mono text-sm text-fg outline-none focus:border-accent disabled:opacity-50" :placeholder="t('sponsor.activateModal.placeholder')" :disabled="activationLoading" @keydown.enter="submitActivation" />
            <p v-if="activationError" class="mt-3 text-xs leading-relaxed text-accent">{{ activationError }}</p>
            <button type="button" class="ak-clip-tr mt-4 flex h-11 w-full items-center justify-center gap-2 bg-accent px-6 text-[13px] font-bold uppercase tracking-[0.25em] text-accent-fg transition-transform hover:scale-[1.01] active:scale-95 disabled:opacity-50" :disabled="!activationCode.trim() || activationLoading" @click="submitActivation"><LoaderCircle v-if="activationLoading" class="animate-spin" :size="15" :stroke-width="2.2" /><Check v-else :size="15" :stroke-width="2.4" />{{ activationLoading ? t("sponsor.activateModal.activating") : t("sponsor.activateModal.submit") }}</button>
          </template>
        </div>
      </div>
      </Transition>
    </Teleport>
  </div>
</template>

<style scoped>
button {
  transition-property: color, background-color, border-color, transform, opacity;
  transition-duration: 240ms;
  transition-timing-function: cubic-bezier(0.2, 0.8, 0.2, 1);
}

.modal-enter-active,
.modal-leave-active {
  transition: opacity 220ms ease;
}

.modal-enter-from,
.modal-leave-to {
  opacity: 0;
}

.modal-enter-active .ak-frame,
.modal-leave-active .ak-frame {
  transition: transform 260ms cubic-bezier(0.2, 0.8, 0.2, 1), opacity 260ms cubic-bezier(0.2, 0.8, 0.2, 1);
}

.modal-enter-from .ak-frame,
.modal-leave-to .ak-frame {
  opacity: 0;
  transform: translateY(12px) scale(0.97);
}
</style>
