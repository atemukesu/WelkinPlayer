<!--
 Copyright 2026 Atemukesu
 SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { Signal, X } from "@lucide/vue";
import { meteredPlaybackRequest, resolveMeteredPlayback } from "../lib/meteredPlayback";
import type { MeteredPlaybackChoice } from "../lib/meteredPlayback";

const { t } = useI18n();

const request = meteredPlaybackRequest;
const cloudCount = computed(() => {
  const value = request.value;
  return value ? Math.max(0, value.total - value.offline) : 0;
});
const hasOffline = computed(() => (request.value?.offline ?? 0) > 0);

function choose(choice: MeteredPlaybackChoice) {
  resolveMeteredPlayback(choice);
}
</script>

<template>
  <Teleport to="body">
    <Transition name="modal">
      <div
        v-if="request"
        class="fixed inset-0 z-[80] grid place-items-center bg-black/70 p-4"
        role="dialog"
        aria-modal="true"
        @pointerdown.self="choose('cancel')"
      >
        <div class="ak-frame w-full max-w-md border border-line bg-surface p-5">
          <div class="flex items-start justify-between gap-3">
            <h3 class="flex items-center gap-2 text-sm font-bold uppercase tracking-[0.2em]">
              <Signal :size="16" :stroke-width="1.8" class="text-accent" />
              {{ t("playback.metered.title") }}
            </h3>
            <button type="button" class="grid h-8 w-8 shrink-0 place-items-center text-dim hover:text-fg" :aria-label="t('controls.close')" @click="choose('cancel')">
              <X :size="16" />
            </button>
          </div>

          <p class="mt-3 text-[13px] leading-relaxed text-muted">
            {{ t("playback.metered.message", { count: cloudCount }) }}
          </p>

          <div class="mt-5 grid gap-2">
            <button
              type="button"
              class="flex h-11 items-center justify-between border px-3 text-left text-[13px] font-semibold tracking-wide transition-colors"
              :class="hasOffline ? 'border-line text-muted hover:border-accent hover:text-fg' : 'cursor-not-allowed border-line/60 text-dim'"
              :disabled="!hasOffline"
              @click="choose('cached')"
            >
              <span>{{ t("playback.metered.cached") }}</span>
              <span class="text-[11px] font-normal tabular-nums">{{ t("playback.metered.cachedCount", { count: request?.offline ?? 0 }) }}</span>
            </button>
            <p v-if="!hasOffline" class="-mt-1 text-[11px] text-dim">{{ t("playback.metered.noCached") }}</p>

            <button
              type="button"
              class="flex h-11 items-center justify-between border border-line px-3 text-left text-[13px] font-semibold tracking-wide text-muted transition-colors hover:border-accent hover:text-fg"
              @click="choose('all')"
            >
              <span>{{ t("playback.metered.all") }}</span>
              <span class="text-[11px] font-normal tabular-nums">{{ t("playback.metered.cloudCount", { count: cloudCount }) }}</span>
            </button>

            <button
              type="button"
              class="mt-1 flex h-10 items-center justify-center border border-transparent text-[12px] font-semibold uppercase tracking-[0.2em] text-dim transition-colors hover:border-line hover:text-fg"
              @click="choose('cancel')"
            >
              {{ t("playback.metered.cancel") }}
            </button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
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
