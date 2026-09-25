import { computed, ref } from "vue";
import { defineStore } from "pinia";

export type SyncStatus = "idle" | "syncing" | "synced" | "error";

export const useSyncStore = defineStore("sync", () => {
  const status = ref<SyncStatus>("idle");
  const error = ref("");
  const pending = ref(0);
  const lastSyncedAt = ref<number | null>(null);

  const isSyncing = computed(() => status.value === "syncing");

  function begin(count = 0) {
    status.value = "syncing";
    error.value = "";
    pending.value = count;
  }

  function setPending(count: number) {
    pending.value = Math.max(0, count);
  }

  function complete() {
    status.value = "synced";
    pending.value = 0;
    lastSyncedAt.value = Date.now();
  }

  function fail(message: string) {
    status.value = "error";
    error.value = message;
  }

  function reset() {
    status.value = "idle";
    error.value = "";
    pending.value = 0;
  }

  return { status, error, pending, lastSyncedAt, isSyncing, begin, setPending, complete, fail, reset };
});
