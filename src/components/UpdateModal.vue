<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { Clock, Download, SkipForward, X } from "@lucide/vue";
import { openUrl } from "@tauri-apps/plugin-opener";
import { formatBytes, formatSpeed } from "../lib/format";
import { pushToast } from "../lib/toast";
import { isAndroid } from "../lib/desktopLyric";
import { useUpdateStore } from "../stores/update";

const emit = defineEmits<{ friendlyError: [error: unknown] }>();
const { t } = useI18n();
const update = useUpdateStore();
/** Android downloads the APK through the browser instead of installing in-app. */
const android = isAndroid();

/** Rounded download progress percentage, 0 while the size is unknown. */
const percent = computed(() => Math.round((update.progress ?? 0) * 100));
/** Bytes received so far, with the total when the backend reported one. */
const sizeText = computed(() =>
  update.total && update.total > 0 ? `${formatBytes(update.downloaded)} / ${formatBytes(update.total)}` : formatBytes(update.downloaded),
);
/** Current transfer rate, or null before the first sample / once finished. */
const speedText = computed(() => (update.speed && update.speed > 0 ? formatSpeed(update.speed) : null));
/** Human-readable release date, or null when the backend omitted it. */
const releaseDate = computed(() => {
  const raw = update.info?.date;
  if (!raw) return null;
  const parsed = new Date(raw);
  return Number.isNaN(parsed.getTime()) ? raw : parsed.toLocaleDateString();
});

/** "Update now": install in-app on desktop, or open the APK link on Android. */
async function updateNow() {
  if (android) {
    const url = update.info?.downloadUrl;
    if (!url) {
      pushToast("error", t("settings.update.noDownload"));
      return;
    }
    try {
      await openUrl(url);
      update.hide();
    } catch (error) {
      emit("friendlyError", error);
    }
    return;
  }
  update.clearSkip();
  try {
    await update.install();
  } catch (error) {
    emit("friendlyError", error);
  }
}
</script>

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

<template>
  <Teleport to="body">
    <Transition name="modal">
      <div v-if="update.open && update.info" class="fixed inset-0 z-[80] grid place-items-center bg-black/70 p-4" role="dialog" aria-modal="true" @pointerdown.self="update.hide()">
        <div class="ak-frame flex max-h-[90vh] w-full max-w-md flex-col border border-line bg-surface p-5">
          <div class="flex shrink-0 items-center justify-between">
            <h3 class="text-sm font-bold uppercase tracking-[0.2em]">{{ t("settings.update.modalTitle") }}</h3>
            <button type="button" class="grid h-8 w-8 place-items-center text-dim hover:text-fg" @click="update.hide()"><X :size="16" /></button>
          </div>

          <div class="mt-3 flex shrink-0 flex-wrap items-baseline gap-x-2 gap-y-1">
            <span class="text-2xl font-black tabular-nums text-accent">v{{ update.info?.version }}</span>
            <span class="text-xs text-dim">{{ t("settings.update.fromVersion", { version: update.info?.currentVersion }) }}</span>
          </div>
          <p v-if="releaseDate" class="mt-1 shrink-0 text-[11px] text-dim">{{ t("settings.update.released", { date: releaseDate }) }}</p>

          <div v-if="update.info?.notes" class="mt-4 flex min-h-0 flex-auto flex-col gap-1">
            <span class="shrink-0 text-[11px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("settings.update.notes") }}</span>
            <pre class="min-h-0 flex-auto overflow-y-auto whitespace-pre-wrap break-words border border-line bg-bg p-3 font-sans text-xs leading-relaxed text-muted">{{ update.info?.notes }}</pre>
          </div>

          <div v-if="update.installing" class="mt-5 grid shrink-0 gap-2.5">
            <span class="block h-2 w-full overflow-hidden bg-fg/10"><span class="block h-full bg-accent transition-[width]" :style="{ width: `${percent}%` }"></span></span>
            <div class="grid grid-cols-[1fr_5.5rem_2.5rem] items-baseline gap-x-3 text-[11px] tabular-nums text-dim">
              <span class="truncate">{{ sizeText }}</span>
              <span class="truncate text-right">{{ speedText ?? "" }}</span>
              <span class="text-right font-semibold text-fg">{{ percent }}%</span>
            </div>
            <button type="button" class="ak-clip-tr flex h-10 items-center justify-center border border-line px-5 text-[13px] font-semibold uppercase tracking-[0.25em] text-fg hover:border-accent hover:text-accent" @click="update.hide()">{{ t("settings.update.background") }}</button>
          </div>

          <div v-else class="mt-5 grid shrink-0 gap-2">
            <button type="button" class="ak-clip-tr flex h-11 items-center justify-center gap-2 bg-accent px-6 text-[13px] font-bold uppercase tracking-[0.25em] text-accent-fg hover:scale-[1.01] active:scale-95" @click="updateNow"><Download :size="15" :stroke-width="2.2" />{{ t("settings.update.updateNow") }}</button>
            <div class="grid grid-cols-1 gap-2 sm:grid-cols-2">
              <button type="button" class="ak-clip-tr flex h-10 items-center justify-center gap-2 border border-line px-3 text-[12px] font-semibold uppercase tracking-[0.15em] text-muted hover:border-accent hover:text-accent" @click="update.remindLater"><Clock :size="14" :stroke-width="2.2" />{{ t("settings.update.remindLater") }}</button>
              <button type="button" class="ak-clip-tr flex h-10 items-center justify-center gap-2 border border-line px-3 text-[12px] font-semibold uppercase tracking-[0.15em] text-muted hover:border-accent hover:text-accent" @click="update.skipVersion"><SkipForward :size="14" :stroke-width="2.2" />{{ t("settings.update.skipVersion") }}</button>
            </div>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>
