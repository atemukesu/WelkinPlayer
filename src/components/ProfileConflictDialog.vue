<!--
  Copyright 2026 Atemukesu
  SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { AlertTriangle, Cloud, HardDrive } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import type { ProfileConflict } from "../lib/profileSync";

defineProps<{ conflicts: ProfileConflict[]; saving?: boolean }>();
const emit = defineEmits<{ choose: [choice: "local" | "remote"] }>();
const { t } = useI18n();

function describe(conflict: ProfileConflict): string {
  switch (conflict.kind) {
    case "playlistTracks":
      return t("profileSync.conflictPlaylistTracks", { name: conflict.name });
    case "playlistName":
      return t("profileSync.conflictPlaylistName", { name: conflict.name });
    case "playlistCover":
      return t("profileSync.conflictPlaylistCover", { name: conflict.name });
    case "playlistDeleted":
      return t("profileSync.conflictPlaylistDeleted", { name: conflict.name });
    case "nickname":
      return t("profileSync.conflictNickname");
    case "favorite":
      return t("profileSync.conflictFavorite", { name: conflict.name });
    case "lyrics":
      return t("profileSync.conflictLyrics", { name: conflict.name });
    case "remoteChanged":
      return t("profileSync.conflictRemoteChanged");
  }
}

function stateLabel(value: string | undefined): string {
  if (value === "on") return t("profileSync.stateOn");
  if (value === "off") return t("profileSync.stateOff");
  return value ?? "";
}
</script>

<template>
  <Teleport to="body">
    <div class="fixed inset-0 z-[90] grid place-items-center bg-black/60 p-4 backdrop-blur-sm">
      <div class="ak-frame w-full max-w-2xl border border-line bg-bg shadow-2xl">
        <header class="flex items-start gap-3 border-b border-line px-5 py-4">
          <span class="mt-0.5 grid h-9 w-9 shrink-0 place-items-center border border-amber-500/50 text-amber-400"><AlertTriangle :size="18" /></span>
          <div class="min-w-0">
            <h2 class="text-lg font-black tracking-tight">{{ t("profileSync.title") }}</h2>
            <p class="mt-1 text-xs leading-relaxed text-muted">{{ t("profileSync.description") }}</p>
          </div>
        </header>

        <div class="max-h-[45vh] overflow-y-auto px-5 py-4">
          <ul class="grid gap-2">
            <li v-for="item in conflicts" :key="item.id" class="border border-line bg-surface px-3 py-2.5">
              <p class="text-[13px] font-semibold">{{ describe(item) }}</p>
              <div class="mt-1 flex flex-wrap gap-x-4 gap-y-1 font-mono text-[11px] text-muted">
                <span class="flex items-center gap-1.5"><HardDrive :size="12" />{{ t("profileSync.localShort") }}: {{ stateLabel(item.local) }}</span>
                <span class="flex items-center gap-1.5"><Cloud :size="12" />{{ t("profileSync.remoteShort") }}: {{ stateLabel(item.remote) }}</span>
              </div>
            </li>
          </ul>
        </div>

        <footer class="flex flex-col gap-2 border-t border-line px-5 py-4 sm:flex-row sm:justify-end">
          <button type="button" class="ak-clip-tr h-11 border border-line px-5 text-[13px] font-semibold text-muted transition-colors hover:border-red-500/60 hover:text-red-400 disabled:opacity-50" :disabled="saving" @click="emit('choose', 'local')">{{ t("profileSync.keepLocal") }}</button>
          <button type="button" class="ak-clip-tr h-11 bg-accent px-5 text-[13px] font-bold text-accent-fg transition-transform hover:scale-[1.02] active:scale-95 disabled:opacity-50" :disabled="saving" @click="emit('choose', 'remote')">{{ t("profileSync.keepRemote") }}</button>
        </footer>
      </div>
    </div>
  </Teleport>
</template>
