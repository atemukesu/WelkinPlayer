<script setup lang="ts">
import { nextTick, onMounted, onUnmounted, ref, watch, watchEffect } from "vue";
import { useI18n } from "vue-i18n";
import { describeError, invoke } from "./api";
import { initAudio, seekTo } from "./lib/audio";
import { accents } from "./lib/app";
import type { Accent, Theme, View } from "./lib/app";
import { useLibrary } from "./composables/useLibrary";
import { pushToast } from "./lib/toast";
import { usePlayerStore } from "./stores/player";
import type { Track } from "./stores/player";
import { useWebdavStore } from "./stores/webdav";
import { useProfileStore } from "./stores/profile";
import { useLyricsStore } from "./stores/lyrics";
import AppNavigation from "./components/AppNavigation.vue";
import NowPlayingPanel from "./components/NowPlayingPanel.vue";
import PlayerBar from "./components/PlayerBar.vue";
import SetupWizard from "./components/SetupWizard.vue";
import ToastStack from "./components/ToastStack.vue";
import LibraryView from "./views/LibraryView.vue";
import PlayerView from "./views/PlayerView.vue";
import SettingsView from "./views/SettingsView.vue";
import TracksView from "./views/TracksView.vue";
import FavoritesView from "./views/FavoritesView.vue";
import PlaylistEditorView from "./views/PlaylistEditorView.vue";

const { t, locale } = useI18n();
const player = usePlayerStore();
const webdav = useWebdavStore();
const profile = useProfileStore();
const lyrics = useLyricsStore();
const { loadingLibrary, enriching, refreshing, enrichDone, enrichTotal, friendlyError, loadCachedLibrary, loadRemoteLibrary, refreshLibrary, downloadTrackMetadata } = useLibrary();
const view = ref<View>(getViewFromHash());
const baseView = ref<View>(view.value === "player" ? "library" : view.value);
const theme = ref<Theme>(localStorage.getItem("welkin-theme") === "dark" ? "dark" : "light");
const accent = ref<Accent>(readAccent());
const panelWidth = ref(readPanelWidth());
const panelCollapsed = ref(localStorage.getItem("welkin-panel-collapsed") === "1");
const sidebarCollapsed = ref(localStorage.getItem("welkin-sidebar-collapsed") === "1");
const panelResizing = ref(false);
const testing = ref(false);
const downloadingTrackId = ref<number | null>(null);
const booted = ref(false);
const playlistFilter = ref<string | null>(null);
const editingPlaylist = ref<string | null>(null);
const playbackRestored = ref(false);
let applyingProfile = false;
let restoringPlayback = false;
let lastPositionSavedAt = 0;

watch(view, (next) => { if (next !== "player") baseView.value = next; });
watchEffect(() => { document.documentElement.classList.toggle("dark", theme.value === "dark"); document.documentElement.dataset.accent = accent.value; localStorage.setItem("welkin-theme", theme.value); localStorage.setItem("welkin-accent", accent.value); });
watchEffect(() => { document.documentElement.lang = locale.value; localStorage.setItem("welkin-locale", locale.value); });

watch([theme, accent], () => { if (booted.value && !applyingProfile) profile.setAppearance({ theme: theme.value, accent: accent.value }); });
watch(locale, (value) => { if (booted.value && !applyingProfile) profile.setAppearance({ locale: value as "zh-CN" | "en" }); });
watch(
  () => [lyrics.source, lyrics.lineSize, lyrics.translationSize, lyrics.lineSpacing, lyrics.translate],
  () => {
    if (booted.value && !applyingProfile) {
      profile.setLyrics({ source: lyrics.source, lineSize: lyrics.lineSize, translationSize: lyrics.translationSize, lineSpacing: lyrics.lineSpacing, translate: lyrics.translate });
    }
  },
);
watch(() => player.currentTrack?.path, (path) => { if (booted.value && path && player.isPlaying) profile.recordPlay(path); });
watch(() => profile.revision, () => { if (booted.value) applyProfile(); });

// Resume the last track/position once both the profile and library are ready.
watch([() => profile.ready, () => player.tracks.length], () => {
  if (playbackRestored.value || !profile.ready || player.tracks.length === 0) return;
  playbackRestored.value = true;
  const path = profile.profile.lastTrack;
  if (!path) return;
  const track = player.tracks.find((item) => item.path === path);
  if (!track) return;
  restoringPlayback = true;
  player.currentTrack = track;
  player.isPlaying = false;
  const position = profile.profile.lastPosition ?? 0;
  if (position > 0) void nextTick(() => seekTo(position));
  window.setTimeout(() => { restoringPlayback = false; }, 1200);
});

// Persist the playback position: on track changes, on pause/seek/hide/exit, and
// as a low-frequency safety net while audio keeps playing.
function savePlaybackPosition(force = false) {
  const path = player.currentTrack?.path;
  if (!booted.value || restoringPlayback || !path) return;
  lastPositionSavedAt = Date.now();
  profile.recordPlayback(path, player.position);
  if (force) void profile.flush();
}
function savePlaybackPositionOnLeave() { savePlaybackPosition(true); }
function onVisibilityChange() { if (document.visibilityState === "hidden") savePlaybackPosition(true); }

watch(() => player.currentTrack?.path, (path) => {
  if (!booted.value || restoringPlayback || !path) return;
  lastPositionSavedAt = Date.now();
  profile.recordPlayback(path, 0);
});
watch(() => player.position, (position, previous) => {
  if (!booted.value || restoringPlayback) return;
  // A large jump between two timeupdate samples means the user seeked; save it
  // right away instead of waiting for the periodic safety net.
  const jumped = typeof previous === "number" && Math.abs(position - previous) > 2;
  if (!jumped && Date.now() - lastPositionSavedAt < player.saveInterval * 1000) return;
  savePlaybackPosition();
});
watch(() => player.isPlaying, (playing) => {
  if (!booted.value || restoringPlayback || playing) return;
  savePlaybackPosition(true);
});

function readAccent(): Accent { const value = localStorage.getItem("welkin-accent"); return (accents.some((item) => item.id === value) ? value : "amber") as Accent; }
function getViewFromHash(): View { const route = window.location.hash.slice(1); return route === "settings" || route === "player" || route === "tracks" || route === "favorites" || route === "playlist-new" ? route : "library"; }
function setView(nextView: View) { view.value = nextView; window.location.hash = nextView === "library" ? "" : nextView; }
function navigate(nextView: View) { playlistFilter.value = null; setView(nextView); }
function syncViewFromHash() { view.value = getViewFromHash(); }
function openUserPlaylist(id: string) { playlistFilter.value = id; setView("tracks"); }
function createPlaylist() { playlistFilter.value = null; editingPlaylist.value = null; setView("playlist-new"); }
function editPlaylist(id: string) { playlistFilter.value = null; editingPlaylist.value = id; setView("playlist-new"); }
function onPlaylistSaved(id: string) { playlistFilter.value = id; setView("tracks"); }
function onPlaylistEditorCancel() { navigate("library"); }

function applyProfile() {
  applyingProfile = true;
  theme.value = profile.profile.appearance.theme;
  accent.value = profile.profile.appearance.accent;
  locale.value = profile.profile.appearance.locale;
  lyrics.source = profile.profile.lyrics.source;
  lyrics.lineSize = profile.profile.lyrics.lineSize;
  lyrics.translationSize = profile.profile.lyrics.translationSize;
  lyrics.lineSpacing = profile.profile.lyrics.lineSpacing;
  lyrics.translate = profile.profile.lyrics.translate;
  void nextTick(() => { applyingProfile = false; });
}

async function bootstrap() {
  await webdav.hydrate();
  const firstRun = await profile.hydrate({ theme: theme.value, accent: accent.value, locale: locale.value as "zh-CN" | "en" });
  applyProfile();
  booted.value = true;
  void loadCachedLibrary();
  if (!firstRun && webdav.password) void loadRemoteLibrary({ silent: true });
}

function onWizardFinish() {
  if (webdav.password) void loadRemoteLibrary();
  else void loadCachedLibrary();
}

async function saveSettings() {
  if (webdav.saving) return;
  profile.setNickname(profile.nickname);
  const { warning } = await webdav.persist();
  if (webdav.error) { pushToast("error", t("settings.webdav.saveFailed", { value: webdav.error })); return; }
  if (warning) { pushToast("warning", t("settings.webdav.keychainUnavailable")); return; }
  await profile.flush();
  pushToast("success", t("settings.saved")); await loadRemoteLibrary();
}
async function testConnection() { testing.value = true; try { await invoke<string>("test_webdav_connection"); pushToast("success", t("settings.webdav.testOk")); await loadRemoteLibrary(); } catch (error) { pushToast("error", friendlyError(error)); } finally { testing.value = false; } }
function showError(error: unknown) { pushToast("error", friendlyError(error)); }
async function downloadMetadata(track: Track) {
  downloadingTrackId.value = track.id;
  try {
    await downloadTrackMetadata(track);
    pushToast("success", t("library.menu.metadataDownloaded"));
  } catch (error) {
    pushToast("error", t("library.menu.metadataFailed", { value: friendlyError(error) }));
  } finally { downloadingTrackId.value = null; }
}
function openDetails(track: Track) { player.selectTrack(track); panelCollapsed.value = false; localStorage.setItem("welkin-panel-collapsed", "0"); }
function togglePanel() { panelCollapsed.value = !panelCollapsed.value; localStorage.setItem("welkin-panel-collapsed", panelCollapsed.value ? "1" : "0"); }
function toggleSidebar() { sidebarCollapsed.value = !sidebarCollapsed.value; localStorage.setItem("welkin-sidebar-collapsed", sidebarCollapsed.value ? "1" : "0"); }
function startResize(event: PointerEvent) { const startX = event.clientX; const startWidth = panelWidth.value; const target = event.currentTarget as HTMLElement; panelResizing.value = true; target.setPointerCapture(event.pointerId); const onMove = (moveEvent: PointerEvent) => { const max = Math.min(640, Math.max(280, window.innerWidth - 420)); panelWidth.value = Math.min(max, Math.max(280, startWidth + startX - moveEvent.clientX)); }; const onUp = () => { panelResizing.value = false; target.removeEventListener("pointermove", onMove); target.removeEventListener("pointerup", onUp); target.removeEventListener("pointercancel", onUp); localStorage.setItem("welkin-panel-width", String(Math.round(panelWidth.value))); }; target.addEventListener("pointermove", onMove); target.addEventListener("pointerup", onUp); target.addEventListener("pointercancel", onUp); }
function readPanelWidth() { const stored = Number(localStorage.getItem("welkin-panel-width")); return Number.isFinite(stored) && stored >= 280 && stored <= 640 ? stored : 380; }
function onGlobalKeydown(event: KeyboardEvent) {
  if (event.code !== "Space" && event.key !== " ") return;
  const target = event.target as HTMLElement | null;
  if (!target) return;
  const tag = target.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || target.isContentEditable) return;
  if (target.closest("button, a, [role='button'], [contenteditable='true']")) return;
  if (!player.currentTrack) return;
  event.preventDefault();
  player.togglePlayback();
}

onMounted(() => { window.addEventListener("hashchange", syncViewFromHash); window.addEventListener("keydown", onGlobalKeydown); window.addEventListener("beforeunload", savePlaybackPositionOnLeave); window.addEventListener("pagehide", savePlaybackPositionOnLeave); document.addEventListener("visibilitychange", onVisibilityChange); void initAudio(); void invoke<string>("ping").catch((error) => console.warn("[welkin] ping failed", describeError(error))); void bootstrap(); });
onUnmounted(() => { savePlaybackPosition(true); window.removeEventListener("hashchange", syncViewFromHash); window.removeEventListener("keydown", onGlobalKeydown); window.removeEventListener("beforeunload", savePlaybackPositionOnLeave); window.removeEventListener("pagehide", savePlaybackPositionOnLeave); document.removeEventListener("visibilitychange", onVisibilityChange); void profile.flush(); });
</script>

<template>
  <Transition name="slide"><PlayerView v-if="view === 'player' && player.currentTrack" :return-view="baseView" @navigate="navigate" /></Transition>
  <SetupWizard v-if="booted && profile.firstRun" v-model:theme="theme" v-model:accent="accent" @finish="onWizardFinish" />
  <div class="flex h-screen flex-col overflow-hidden bg-bg text-fg" :class="booted ? '' : 'opacity-0'"><AppNavigation placement="header" :active-view="baseView" @navigate="navigate" /><div class="flex min-h-0 flex-1"><AppNavigation placement="sidebar" :active-view="baseView" :collapsed="sidebarCollapsed" :active-playlist-id="playlistFilter" @navigate="navigate" @toggle="toggleSidebar" @create-playlist="createPlaylist" @open-playlist="openUserPlaylist" /><main class="min-w-0 flex-1 overflow-y-auto"><Transition name="page" mode="out-in"><LibraryView v-if="baseView === 'library'" key="library" :loading="loadingLibrary" :enriching="enriching" :enrich-done="enrichDone" :enrich-total="enrichTotal" :downloading-track-id="downloadingTrackId" @settings="navigate('settings')" @navigate="navigate" @open-playlist="openUserPlaylist" @edit-playlist="editPlaylist" @details="openDetails" @download-metadata="downloadMetadata" /><TracksView v-else-if="baseView === 'tracks'" key="tracks" :playlist-id="playlistFilter" :loading="loadingLibrary" :enriching="enriching" :refreshing="refreshing" :enrich-done="enrichDone" :enrich-total="enrichTotal" :downloading-track-id="downloadingTrackId" @refresh="refreshLibrary" @edit="editPlaylist" @details="openDetails" @download-metadata="downloadMetadata" /><FavoritesView v-else-if="baseView === 'favorites'" key="favorites" :loading="loadingLibrary" :enriching="enriching" :enrich-done="enrichDone" :enrich-total="enrichTotal" :downloading-track-id="downloadingTrackId" @details="openDetails" @download-metadata="downloadMetadata" /><PlaylistEditorView v-else-if="baseView === 'playlist-new'" key="playlist-new" :playlist-id="editingPlaylist" @done="onPlaylistSaved" @cancel="onPlaylistEditorCancel" /><SettingsView v-else key="settings" :testing="testing" v-model:theme="theme" v-model:accent="accent" @save="saveSettings" @test="testConnection" @friendly-error="showError" /></Transition></main><NowPlayingPanel v-if="baseView === 'library' || baseView === 'tracks' || baseView === 'favorites'" :collapsed="panelCollapsed" :width="panelWidth" :resizing="panelResizing" @toggle="togglePanel" @resize="startResize" /></div><PlayerBar v-if="player.currentTrack" @open="navigate('player')" /><AppNavigation placement="mobile" :active-view="baseView" @navigate="navigate" /><ToastStack /></div>
</template>
