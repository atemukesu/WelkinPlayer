<script setup lang="ts">
import { computed, defineAsyncComponent, nextTick, onMounted, onUnmounted, ref, watch, watchEffect } from "vue";
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
import { usePlaybackStore } from "./stores/playback";
import { useLyricsStore } from "./stores/lyrics";
import AppNavigation from "./components/AppNavigation.vue";
import QueuePanel from "./components/QueuePanel.vue";
import PlayerBar from "./components/PlayerBar.vue";
import SetupWizard from "./components/SetupWizard.vue";
import ToastStack from "./components/ToastStack.vue";
import LibraryView from "./views/LibraryView.vue";
import PlayerView from "./views/PlayerView.vue";
import SettingsView from "./views/SettingsView.vue";
import TracksView from "./views/TracksView.vue";
import FavoritesView from "./views/FavoritesView.vue";
import StatsView from "./views/StatsView.vue";
import GroupedLibraryView from "./views/GroupedLibraryView.vue";
import PlaylistEditorView from "./views/PlaylistEditorView.vue";
import LyricsEditorView from "./views/LyricsEditorView.vue";
import TrackEditorView from "./views/TrackEditorView.vue";
import TrackInfoView from "./views/TrackInfoView.vue";

const AmllPlayerView = defineAsyncComponent(() => import("./views/AmllPlayerView.vue"));

const { t, locale } = useI18n();
const player = usePlayerStore();
const webdav = useWebdavStore();
const profile = useProfileStore();
const playback = usePlaybackStore();
const lyrics = useLyricsStore();
const { loadingLibrary, enriching, refreshing, enrichDone, enrichTotal, friendlyError, loadCachedLibrary, loadRemoteLibrary, refreshLibrary, downloadTrackMetadata } = useLibrary();
const view = ref<View>(getViewFromHash());
const baseView = ref<View>(view.value === "player" || view.value === "track-info" ? "library" : view.value);
const theme = ref<Theme>(localStorage.getItem("welkin-theme") === "dark" ? "dark" : "light");
const accent = ref<Accent>(readAccent());
const sidebarCollapsed = ref(localStorage.getItem("welkin-sidebar-collapsed") === "1");
const testing = ref(false);
const downloadingTrackId = ref<number | null>(null);
const booted = ref(false);
const playlistFilter = ref<string | null>(null);
const artistFilter = ref<string | null>(null);
const albumFilter = ref<string | null>(null);
const editingPlaylist = ref<string | null>(null);
const editingLyricsPath = ref<string | null>(null);
const editingTrackPath = ref<string | null>(null);
const viewingTrackPath = ref<string | null>(null);
const playbackRestored = ref(false);
const showQueue = ref(false);
/** Whether the AMLL full-screen player should be open. */
const amllActive = computed(() => view.value === "player" && !!player.currentTrack && lyrics.useAmll);
/** Stays true until AmllPlayerView finishes its slide-out, then it unmounts. */
const amllMounted = ref(false);
watch(amllActive, (active) => { if (active) amllMounted.value = true; }, { immediate: true });
let restoringPlayback = false;
let lastPositionSavedAt = 0;
/** Local-only safety-net cadence (ms) for periodic progress writes while playing. */
const POSITION_SAVE_INTERVAL_MS = 30_000;

watch(view, (next) => { if (next !== "player" && next !== "track-info") baseView.value = next; });
watchEffect(() => { document.documentElement.classList.toggle("dark", theme.value === "dark"); document.documentElement.dataset.accent = accent.value; localStorage.setItem("welkin-theme", theme.value); localStorage.setItem("welkin-accent", accent.value); });
watchEffect(() => { document.documentElement.lang = locale.value; localStorage.setItem("welkin-locale", locale.value); });

watch(() => player.currentTrack?.path, (path) => { if (booted.value && path && player.isPlaying) profile.recordPlay(path); });

// Resume the last track/position once the profile, playback state and library are ready.
watch([() => playback.ready, () => player.tracks.length], () => {
  if (playbackRestored.value || !playback.ready || player.tracks.length === 0) return;
  playbackRestored.value = true;
  const path = playback.path;
  if (!path) return;
  const track = player.tracks.find((item) => item.path === path);
  if (!track) return;
  restoringPlayback = true;
  player.currentTrack = track;
  player.isPlaying = false;
  const position = playback.position;
  if (position > 0) void nextTick(() => seekTo(position));
  window.setTimeout(() => { restoringPlayback = false; }, 1200);
});

// Persist the playback position: on track changes, on pause/seek/hide/exit, and
// as a low-frequency safety net while audio keeps playing.
function savePlaybackPosition(force = false) {
  const path = player.currentTrack?.path;
  if (!booted.value || restoringPlayback || !path) return;
  lastPositionSavedAt = Date.now();
  playback.record(path, player.position);
  if (force) void playback.flush();
}
function savePlaybackPositionOnLeave() { savePlaybackPosition(true); }
function onVisibilityChange() { if (document.visibilityState === "hidden") savePlaybackPosition(true); }

watch(() => player.currentTrack?.path, (path) => {
  if (!booted.value || restoringPlayback || !path) return;
  lastPositionSavedAt = Date.now();
  playback.record(path, 0);
});
watch(() => player.position, (position, previous) => {
  if (!booted.value || restoringPlayback) return;
  // A large jump between two timeupdate samples means the user seeked; save it
  // right away instead of waiting for the periodic safety net.
  const jumped = typeof previous === "number" && Math.abs(position - previous) > 2;
  if (!jumped && Date.now() - lastPositionSavedAt < POSITION_SAVE_INTERVAL_MS) return;
  savePlaybackPosition();
});
watch(() => player.isPlaying, (playing) => {
  if (!booted.value || restoringPlayback || playing) return;
  savePlaybackPosition(true);
});

function readAccent(): Accent { const value = localStorage.getItem("welkin-accent"); return (accents.some((item) => item.id === value) ? value : "amber") as Accent; }
function getViewFromHash(): View { const route = window.location.hash.slice(1); return route === "settings" || route === "player" || route === "tracks" || route === "favorites" || route === "stats" || route === "artists" || route === "albums" || route === "playlist-new" || route === "lyrics-edit" || route === "track-edit" || route === "track-info" ? route : "library"; }
function setView(nextView: View) { view.value = nextView; window.location.hash = nextView === "library" ? "" : nextView; }
function clearCollectionFilters() { playlistFilter.value = null; artistFilter.value = null; albumFilter.value = null; }
function navigate(nextView: View) { clearCollectionFilters(); setView(nextView); }
function syncViewFromHash() { view.value = getViewFromHash(); }
function openUserPlaylist(id: string) { artistFilter.value = null; albumFilter.value = null; playlistFilter.value = id; setView("tracks"); }
function openGroup(kind: "artists" | "albums", key: string) { playlistFilter.value = null; artistFilter.value = kind === "artists" ? key : null; albumFilter.value = kind === "albums" ? key : null; setView("tracks"); }
function onOpenGroup(key: string) { openGroup(baseView.value === "albums" ? "albums" : "artists", key); }
function createPlaylist() { clearCollectionFilters(); editingPlaylist.value = null; setView("playlist-new"); }
function editPlaylist(id: string) { clearCollectionFilters(); editingPlaylist.value = id; setView("playlist-new"); }
function editLyrics(track: Track) { if (!track.path) return; editingLyricsPath.value = track.path; closeQueue(); setView("lyrics-edit"); }
function editInfo(track: Track) { if (!track.path) return; editingTrackPath.value = track.path; closeQueue(); setView("track-edit"); }
function showTrackInfo(track: Track) { if (!track.path) return; viewingTrackPath.value = track.path; closeQueue(); setView("track-info"); }
function closeTrackInfo() { setView(baseView.value); }
function onPlaylistSaved(id: string) { artistFilter.value = null; albumFilter.value = null; playlistFilter.value = id; setView("tracks"); }
function onPlaylistEditorCancel() { navigate("library"); }
function toggleQueue() {
  if (showQueue.value) closeQueue();
  else openQueue();
}
function openQueue() { showQueue.value = true; }
function closeQueue() { showQueue.value = false; }

async function bootstrap() {
  await webdav.hydrate();
  const firstRun = await profile.hydrate();
  const seeded = await playback.hydrate({ path: profile.profile.lastTrack, position: profile.profile.lastPosition });
  if (seeded) profile.clearLegacyPlayback();
  booted.value = true;
  void loadCachedLibrary();
  if (!firstRun) {
    // The wizard owns the first remote fetch; returning users refresh silently.
    void profile.syncRemote();
    if (webdav.hasStoredPassword) void loadRemoteLibrary({ silent: true });
  }
}

function onWizardFinish() {
  if (webdav.hasStoredPassword) void loadRemoteLibrary();
  else void loadCachedLibrary();
}

async function saveSettings() {
  if (webdav.saving) return;
  profile.setNickname(profile.nickname);

  const risk = await webdav.checkUrlRisk().catch(() => null);
  if (risk?.safety === "insecurePublic" && !webdav.allowInsecure) {
    pushToast("error", t("settings.webdav.insecurePublicBlocked"));
    return;
  }

  const { warning } = await webdav.persist();
  if (webdav.error) { pushToast("error", t("settings.webdav.saveFailed", { value: webdav.error })); return; }
  if (warning) { pushToast("warning", t("settings.webdav.keychainUnavailable")); return; }
  if (risk?.safety === "insecurePrivate" && !webdav.allowInsecure) {
    pushToast("warning", t("settings.webdav.insecureWarning"));
  }
  await profile.flush();
  pushToast("success", t("settings.saved")); await loadRemoteLibrary();
}
async function testConnection() { testing.value = true; try { const risk = await webdav.checkUrlRisk().catch(() => null); if (risk?.safety === "insecurePrivate" && !webdav.allowInsecure) pushToast("warning", t("settings.webdav.insecureWarning")); await invoke<string>("test_webdav_connection"); pushToast("success", t("settings.webdav.testOk")); await loadRemoteLibrary(); } catch (error) { pushToast("error", friendlyError(error)); } finally { testing.value = false; } }
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
function toggleSidebar() { sidebarCollapsed.value = !sidebarCollapsed.value; localStorage.setItem("welkin-sidebar-collapsed", sidebarCollapsed.value ? "1" : "0"); }
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
  <AmllPlayerView v-if="amllMounted && player.currentTrack" :active="amllActive" :return-view="baseView" @navigate="navigate" @closed="amllMounted = false" />
  <Transition name="slide"><PlayerView v-if="view === 'player' && player.currentTrack && !lyrics.useAmll" :return-view="baseView" @navigate="navigate" @queue="toggleQueue" /></Transition>
  <Transition name="info"><TrackInfoView v-if="view === 'track-info' && viewingTrackPath" :path="viewingTrackPath" @back="closeTrackInfo" /></Transition>
  <SetupWizard v-if="booted && profile.firstRun" v-model:theme="theme" v-model:accent="accent" @finish="onWizardFinish" />
  <div class="flex h-screen flex-col overflow-hidden bg-bg text-fg" :class="booted ? '' : 'opacity-0'"><AppNavigation placement="header" :active-view="baseView" @navigate="navigate" /><div class="relative flex min-h-0 flex-1"><AppNavigation placement="sidebar" :active-view="baseView" :collapsed="sidebarCollapsed" :active-playlist-id="playlistFilter" :active-artist="artistFilter" :active-album="albumFilter" @navigate="navigate" @toggle="toggleSidebar" @create-playlist="createPlaylist" @open-playlist="openUserPlaylist" /><main class="min-w-0 flex-1 overflow-y-auto"><Transition name="page" mode="out-in"><LibraryView v-if="baseView === 'library'" key="library" :loading="loadingLibrary" :enriching="enriching" :enrich-done="enrichDone" :enrich-total="enrichTotal" :downloading-track-id="downloadingTrackId" @settings="navigate('settings')" @navigate="navigate" @open-playlist="openUserPlaylist" @edit-playlist="editPlaylist" @download-metadata="downloadMetadata" @edit-lyrics="editLyrics" @edit-info="editInfo" @show-info="showTrackInfo" /><TracksView v-else-if="baseView === 'tracks'" key="tracks" :playlist-id="playlistFilter" :artist="artistFilter" :album="albumFilter" :loading="loadingLibrary" :enriching="enriching" :refreshing="refreshing" :enrich-done="enrichDone" :enrich-total="enrichTotal" :downloading-track-id="downloadingTrackId" @refresh="refreshLibrary" @edit="editPlaylist" @download-metadata="downloadMetadata" @edit-lyrics="editLyrics" @edit-info="editInfo" @show-info="showTrackInfo" /><GroupedLibraryView v-else-if="baseView === 'artists' || baseView === 'albums'" :key="baseView" :kind="baseView === 'artists' ? 'artists' : 'albums'" :loading="loadingLibrary" @open="onOpenGroup" /><FavoritesView v-else-if="baseView === 'favorites'" key="favorites" :loading="loadingLibrary" :enriching="enriching" :enrich-done="enrichDone" :enrich-total="enrichTotal" :downloading-track-id="downloadingTrackId" @download-metadata="downloadMetadata" @edit-lyrics="editLyrics" @edit-info="editInfo" @show-info="showTrackInfo" /><StatsView v-else-if="baseView === 'stats'" key="stats" :loading="loadingLibrary" :downloading-track-id="downloadingTrackId" @download-metadata="downloadMetadata" @edit-lyrics="editLyrics" @edit-info="editInfo" @show-info="showTrackInfo" /><PlaylistEditorView v-else-if="baseView === 'playlist-new'" key="playlist-new" :playlist-id="editingPlaylist" @done="onPlaylistSaved" @cancel="onPlaylistEditorCancel" /><LyricsEditorView v-else-if="baseView === 'lyrics-edit'" key="lyrics-edit" :path="editingLyricsPath" @back="navigate('library')" @friendly-error="showError" /><TrackEditorView v-else-if="baseView === 'track-edit'" key="track-edit" :path="editingTrackPath" @back="navigate('library')" @friendly-error="showError" /><SettingsView v-else key="settings" :testing="testing" v-model:theme="theme" v-model:accent="accent" @save="saveSettings" @test="testConnection" @friendly-error="showError" /></Transition></main>
        <Transition name="queue-panel"><div v-if="showQueue" class="absolute inset-y-0 right-0 z-20 w-80 border-l border-line bg-surface"><QueuePanel @close="closeQueue" /></div></Transition>
      </div><PlayerBar v-if="player.currentTrack" @open="navigate('player')" @queue="toggleQueue" /><AppNavigation placement="mobile" :active-view="baseView" :active-playlist-id="playlistFilter" :active-artist="artistFilter" :active-album="albumFilter" @navigate="navigate" @create-playlist="createPlaylist" @open-playlist="openUserPlaylist" /><ToastStack /></div></template>
