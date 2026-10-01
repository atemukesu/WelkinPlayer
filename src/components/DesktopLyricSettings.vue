<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { useDesktopLyricsStore } from "../stores/desktopLyrics";
import { requestDesktopLyricPermission } from "../composables/useDesktopLyrics";
import { DEFAULT_DESKTOP_LYRIC } from "../lib/preferences";
import { DESKTOP_LYRIC_GEOMETRY_KEY } from "../lib/desktopLyric";
import { useSystemFonts } from "../lib/fonts";
import DesktopLyricField from "./DesktopLyricField.vue";
import FontFamilyList from "./FontFamilyList.vue";

const { t } = useI18n();
const store = useDesktopLyricsStore();
const { allFonts } = useSystemFonts();

const settings = computed(() => store.settings);
const isAndroid = computed(() => store.platform === "android");
const needsPermission = computed(() => isAndroid.value && settings.value.enabled && !store.permissionGranted);

function reset() {
  Object.assign(store.settings, { ...DEFAULT_DESKTOP_LYRIC, fontFamilies: [] });
  // Forget the remembered floating-window bounds too, so a window that was
  // resized too small snaps back to the multi-line default next time it opens.
  try {
    localStorage.removeItem(DESKTOP_LYRIC_GEOMETRY_KEY);
  } catch {
    /* ignore */
  }
}
</script>

<template>
  <section class="ak-frame grid gap-6 border border-line bg-surface p-6 lg:grid-cols-[1fr_1.3fr]">
    <div>
      <h2 class="text-sm font-bold uppercase tracking-[0.2em]">{{ t("settings.desktopLyrics.title") }}</h2>
      <p class="mt-2 text-sm text-muted">{{ t("settings.desktopLyrics.desc") }}</p>
      <p class="mt-4 flex items-center gap-2 text-[11px] uppercase tracking-[0.2em] text-dim">
        <span class="h-2 w-2" :class="store.active ? 'bg-accent' : 'bg-dim'"></span>
        {{ store.active ? t("settings.desktopLyrics.active") : t("settings.desktopLyrics.inactive") }}
      </p>
    </div>

    <div class="grid gap-6">
      <DesktopLyricField v-model="settings.enabled" type="toggle" :label="t('settings.desktopLyrics.enable')" :hint="t('settings.desktopLyrics.enableHint')" />

      <div v-if="needsPermission" class="grid gap-2 border border-accent/60 bg-accent-soft p-3">
        <strong class="text-[13px] font-bold uppercase tracking-[0.2em] text-accent">{{ t("settings.desktopLyrics.permission") }}</strong>
        <p class="text-[11px] normal-case tracking-normal text-muted">{{ t("settings.desktopLyrics.permissionHint") }}</p>
        <div class="flex justify-end">
          <button type="button" class="ak-clip-tr h-9 bg-accent px-4 text-[13px] font-bold uppercase tracking-[0.2em] text-accent-fg" @click="requestDesktopLyricPermission()">
            {{ t("settings.desktopLyrics.requestPermission") }}
          </button>
        </div>
      </div>

      <!-- Display -->
      <div class="grid gap-4 border border-line bg-bg/40 p-4">
        <DesktopLyricField v-model="settings.translation" type="toggle" :label="t('settings.desktopLyrics.translation')" />
        <DesktopLyricField v-model="settings.karaoke" type="toggle" :label="t('settings.desktopLyrics.karaoke')" :hint="t('settings.desktopLyrics.karaokeHint')" />
      </div>

      <!-- Typography -->
      <div class="grid gap-4 border border-line bg-bg/40 p-4">
        <DesktopLyricField v-model="settings.fontSize" type="slider" :label="t('settings.desktopLyrics.fontSize')" :min="14" :max="96" :step="1" unit="px" />
        <DesktopLyricField v-model="settings.translationSize" type="slider" :label="t('settings.desktopLyrics.translationSize')" :min="10" :max="56" :step="1" unit="px" />
        <DesktopLyricField v-model="settings.fontWeight" type="slider" :label="t('settings.desktopLyrics.fontWeight')" :min="100" :max="900" :step="100" />
        <div class="grid gap-2 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">
          <span>{{ t("settings.desktopLyrics.fontFamilies") }}</span>
          <p class="text-[11px] font-normal normal-case tracking-normal text-dim">{{ t("settings.desktopLyrics.fontOrderHint") }}</p>
          <FontFamilyList v-model="settings.fontFamilies" :available="allFonts" />
        </div>
      </div>

      <!-- Colors -->
      <div class="grid gap-4 border border-line bg-bg/40 p-4">
        <DesktopLyricField v-model="settings.textColor" type="color" :label="t('settings.desktopLyrics.textColor')" />
        <DesktopLyricField v-model="settings.activeColor" type="color" :label="t('settings.desktopLyrics.activeColor')" />
        <DesktopLyricField v-model="settings.translationColor" type="color" :label="t('settings.desktopLyrics.translationColor')" />
        <DesktopLyricField v-model="settings.opacity" type="slider" :label="t('settings.desktopLyrics.opacity')" :min="10" :max="100" :step="1" unit="%" />
      </div>

      <!-- Outline -->
      <div class="grid gap-4 border border-line bg-bg/40 p-4">
        <DesktopLyricField v-model="settings.stroke" type="toggle" :label="t('settings.desktopLyrics.stroke')" />
        <DesktopLyricField v-model="settings.strokeColor" type="color" :label="t('settings.desktopLyrics.strokeColor')" />
      </div>

      <!-- Desktop window behaviour -->
      <div v-if="!isAndroid" class="grid gap-4 border border-line bg-bg/40 p-4">
        <DesktopLyricField v-model="settings.locked" type="toggle" :label="t('settings.desktopLyrics.locked')" :hint="t('settings.desktopLyrics.lockedHint')" />
        <DesktopLyricField v-model="settings.skipTaskbar" type="toggle" :label="t('settings.desktopLyrics.skipTaskbar')" />
      </div>

      <div class="flex justify-end">
        <button type="button" class="ak-clip-tr h-10 border border-line px-5 text-[13px] font-semibold uppercase tracking-[0.25em]" @click="reset">
          {{ t("settings.desktopLyrics.reset") }}
        </button>
      </div>
    </div>
  </section>
</template>
