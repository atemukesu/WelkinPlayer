<script setup lang="ts">
import { X } from "@lucide/vue";
import { dismissToast, toasts } from "../lib/toast";
import type { ToastType } from "../lib/toast";

const TYPE_CLASS: Record<ToastType, string> = {
  success: "ak-toast--success",
  info: "ak-toast--info",
  warning: "ak-toast--warning",
  error: "ak-toast--error",
};
</script>

<template>
  <div
    class="pointer-events-none fixed right-4 bottom-40 z-[100] flex w-[min(92vw,360px)] flex-col items-stretch gap-3 md:right-6 md:bottom-24"
  >
    <TransitionGroup name="toast">
      <div
        v-for="toast in toasts"
        :key="toast.id"
        class="ak-toast pointer-events-auto flex items-start gap-3 px-4 py-3"
        :class="TYPE_CLASS[toast.type]"
      >
        <span class="mt-1.5 h-2 w-2 shrink-0 bg-current"></span>
        <p class="min-w-0 flex-1 text-[13px] font-semibold leading-snug">{{ toast.message }}</p>
        <button
          type="button"
          class="shrink-0 opacity-70 transition-opacity hover:opacity-100"
          aria-label="Dismiss"
          @click="dismissToast(toast.id)"
        >
          <X :size="14" :stroke-width="2.2" />
        </button>
      </div>
    </TransitionGroup>
  </div>
</template>
