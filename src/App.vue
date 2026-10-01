<script setup lang="ts">
import { computed, defineAsyncComponent, nextTick, onMounted, onUnmounted, ref, watch, watchEffect } from "vue";
import { useI18n } from "vue-i18n";
import { describeError, invoke } from "./api";
import { initAudio, seekTo } from "./lib/audio";
import { accents } from "./lib/app";
import type { Accent, Theme, View } from "./lib/app";
import { albumKey, primaryArtist } from "./lib/grouping";
import { trackKey } from "./lib/sources";
import { useLibrary } from "./composables/useLibrary";
import { startDesktopLyrics, stopDesktopLyrics } from "./composables/useDesktopLyrics";

/** The floating lyrics layer must never be able to break app boot. */
function bootDesktopLyrics() {
  try {
    startDesktopLyrics();
  } catch (error) {
    console.warn("[welkin] desktop lyrics failed to start", error);
  }
}
import { pushToast } from "./lib/toast";
import { usePlayerStore } from "./stores/player";
import type { Track } from "./stores/player";
import { useSourcesStore } from "./stores/sources";
import { useProfileStore } from "./stores/profile";
import { usePlaybackStore } from "./stores/playback";
import { useLyricsStore } from "./stores/lyrics";
import { useMetadataStore } from "./stores/metadata";
import { useNetworkStore } from "./stores/network";
import { useCacheStore } from "./stores/cache";
import { useLicenseStore } from "./stores/license";
import AppNavigation from "./components/AppNavigation.vue";
import QueuePanel from "./components/QueuePanel.vue";
import PlayerBar from "./components/PlayerBar.vue";
import SetupWizard from "./components/SetupWizard.vue";
import ToastStack from "./components/ToastStack.vue";
import LibraryView from "./views/LibraryView.vue";
import PlayerView from "./views/PlayerView.vue";
import SettingsView from "./views/SettingsView.vue";
import SponsorView from "./views/SponsorView.vue";
import TracksView from "./views/TracksView.vue";
import FavoritesView from "./views/FavoritesView.vue";
import StatsView from "./views/StatsView.vue";
import GroupedLibraryView from "./views/GroupedLibraryView.vue";
import SourcesView from "./views/SourcesView.vue";
import PlaylistsView from "./views/PlaylistsView.vue";
import PlaylistEditorView from "./views/PlaylistEditorView.vue";
import LyricsEditorView from "./views/LyricsEditorView.vue";
import TrackEditorView from "./views/TrackEditorView.vue";
import TrackInfoView from "./views/TrackInfoView.vue";
import SourceConfigView from "./views/SourceConfigView.vue";

const AmllPlayerView = defineAsyncComponent(() => import("./views/AmllPlayerView.vue"));

const { t, locale } = useI18n();
const player = usePlayerStore();
const sources = useSourcesStore();
const profile = useProfileStore();
const playback = usePlaybackStore();
const lyrics = useLyricsStore();
const metadata = useMetadataStore();
const network = useNetworkStore();
const cache = useCacheStore();
const license = useLicenseStore();
const { loadingLibrary, refreshing, friendlyError, loadCachedLibrary, loadRemoteLibrary, refreshLibrary, downloadTrackMetadata } = useLibrary();
const view = ref<View>(getViewFromHash());
const baseView = ref<View>(view.value === "player" || view.value === "track-info" ? "library" : view.value);
const theme = ref<Theme>(localStorage.getItem("welkin-theme") === "dark" ? "dark" : "light");
const accent = ref<Accent>(readAccent());
const sidebarCollapsed = ref(localStorage.getItem("welkin-sidebar-collapsed") === "1");
const downloadingTrackId = ref<number | null>(null);
const booted = ref(false);
const playlistFilter = ref<string | null>(null);
const artistFilter = ref<string | null>(null);
const albumFilter = ref<string | null>(null);
const sourceFilter = ref<string | null>(null);
const sourceConfigReturn = ref<View>("sources");
const editingPlaylist = ref<string | null>(null);
const editingLyricsTrack = ref<Track | null>(null);
const editingTrack = ref<Track | null>(null);
const viewingTrack = ref<Track | null>(null);
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

// Reload the library shortly after a source is added, removed or reconfigured,
// so configuring a source in Settings takes effect without a manual refresh.
let sourceReloadTimer = 0;
watch(() => sources.revision, () => {
  if (!booted.value || profile.firstRun || !sources.hasSources) return;
  window.clearTimeout(sourceReloadTimer);
  sourceReloadTimer = window.setTimeout(() => void loadRemoteLibrary({ silent: true }), 400);
});
watchEffect(() => { document.documentElement.classList.toggle("dark", theme.value === "dark"); document.documentElement.dataset.accent = accent.value; localStorage.setItem("welkin-theme", theme.value); localStorage.setItem("welkin-accent", accent.value); });
watchEffect(() => { document.documentElement.lang = locale.value; localStorage.setItem("welkin-locale", locale.value); });

/** Minimum time (ms) the inline splash stays visible so its reveal can play and a fast boot doesn't flash it. */
const SPLASH_MIN_MS = 3000;
/** Dismisses the static splash screen inlined in index.html once the app is ready. */
function dismissSplash() {
  const splash = document.getElementById("splash");
  if (!splash) return;
  const elapsed = Date.now() - (window.__welkinSplashStart ?? Date.now());
  window.setTimeout(() => {
    splash.classList.add("splash-out");
    // The fade is 420ms; remove the node after it ends, with a timeout as a fallback.
    splash.addEventListener("transitionend", () => splash.remove(), { once: true });
    window.setTimeout(() => splash.remove(), 800);
  }, Math.max(0, SPLASH_MIN_MS - elapsed));
}
// Reveal the app and fade the splash out once bootstrap() has finished hydrating.
watch(booted, (ready) => { if (ready) dismissSplash(); });

watch(() => trackKey(player.currentTrack), (key) => { if (booted.value && key && player.isPlaying) profile.recordPlay(key); });

// The playing track's metadata is wanted immediately, regardless of visibility.
watch(() => player.currentTrack?.id, () => metadata.request(player.currentTrack));

// Resume the last track/position once the playback state and library are ready.
watch([() => playback.ready, () => player.tracks.length], () => {
  if (playbackRestored.value || !playback.ready || player.tracks.length === 0) return;
  playbackRestored.value = true;
  const stored = playback.path;
  if (!stored) return;
  const track = findTrackByKey(stored);
  if (!track) return;
  restoringPlayback = true;
  player.currentTrack = track;
  player.isPlaying = false;
  const position = playback.position;
  if (position > 0) void nextTick(() => seekTo(position));
  window.setTimeout(() => { restoringPlayback = false; }, 1200);
});

/** Resolve a stored playback key to its track. */
function findTrackByKey(key: string): Track | undefined {
  return player.tracks.find((item) => trackKey(item) === key);
}

// Persist the playback position: on track changes, on pause/seek/hide/exit, and
// as a low-frequency safety net while audio keeps playing.
function savePlaybackPosition(force = false) {
  const key = trackKey(player.currentTrack);
  if (!booted.value || restoringPlayback || !key) return;
  lastPositionSavedAt = Date.now();
  playback.record(key, player.position);
  if (force) void playback.flush();
}
function savePlaybackPositionOnLeave() { savePlaybackPosition(true); }
function onVisibilityChange() { if (document.visibilityState === "hidden") savePlaybackPosition(true); }

watch(() => trackKey(player.currentTrack), (key) => {
  if (!booted.value || restoringPlayback || !key) return;
  lastPositionSavedAt = Date.now();
  playback.record(key, 0);
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
function getViewFromHash(): View { const route = window.location.hash.slice(1); return route === "settings" || route === "sponsor" || route === "player" || route === "tracks" || route === "favorites" || route === "stats" || route === "artists" || route === "albums" || route === "sources" || route === "source-config" || route === "playlists" || route === "playlist-new" || route === "lyrics-edit" || route === "track-edit" || route === "track-info" ? route : "library"; }
function setView(nextView: View) { view.value = nextView; window.location.hash = nextView === "library" ? "" : nextView; }
function clearCollectionFilters() { playlistFilter.value = null; artistFilter.value = null; albumFilter.value = null; sourceFilter.value = null; }
function navigate(nextView: View) { clearCollectionFilters(); setView(nextView); }
function syncViewFromHash() { view.value = getViewFromHash(); }
function openUserPlaylist(id: string) { clearCollectionFilters(); playlistFilter.value = id; setView("tracks"); }
function openSource(id: string) { clearCollectionFilters(); sourceFilter.value = id; setView("tracks"); }
function openSourceConfig(from: View) { sourceConfigReturn.value = from; setView("source-config"); }
function closeSourceConfig() { setView(sourceConfigReturn.value); }
function openGroup(kind: "artists" | "albums", key: string) { playlistFilter.value = null; artistFilter.value = kind === "artists" ? key : null; albumFilter.value = kind === "albums" ? key : null; sourceFilter.value = null; setView("tracks"); }
function onOpenGroup(key: string) { openGroup(baseView.value === "albums" ? "albums" : "artists", key); }
/** Clicking an artist name on a track opens that artist's collection. */
function openArtist(artist: string) { const key = primaryArtist(artist); if (key) openGroup("artists", key); }
/** Clicking an album name opens that album's collection. */
function openAlbum(album: string) { const key = albumKey(album); if (key) openGroup("albums", key); }
function createPlaylist() { clearCollectionFilters(); editingPlaylist.value = null; setView("playlist-new"); }
function editPlaylist(id: string) { clearCollectionFilters(); editingPlaylist.value = id; setView("playlist-new"); }
function editLyrics(track: Track) { if (!track.path) return; editingLyricsTrack.value = track; closeQueue(); setView("lyrics-edit"); }
function editInfo(track: Track) { if (!track.path) return; editingTrack.value = track; closeQueue(); setView("track-edit"); }
function showTrackInfo(track: Track) { if (!track.path) return; viewingTrack.value = track; closeQueue(); setView("track-info"); }
function closeTrackInfo() { setView(baseView.value); }
function onPlaylistSaved(id: string) { artistFilter.value = null; albumFilter.value = null; sourceFilter.value = null; playlistFilter.value = id; setView("tracks"); }
function onPlaylistEditorCancel() { navigate("library"); }
function onPlaylistDeleted(id: string) {
  if (playlistFilter.value === id) playlistFilter.value = null;
  setView("playlists");
}
function toggleQueue() {
  if (showQueue.value) closeQueue();
  else openQueue();
}
function openQueue() { showQueue.value = true; }
function closeQueue() { showQueue.value = false; }

async function bootstrap() {
  await sources.hydrate();
  const firstRun = await profile.hydrate();
  const seeded = await playback.hydrate({ path: profile.profile.lastTrack, position: profile.profile.lastPosition });
  if (seeded) profile.clearLegacyPlayback();
  // Ask Rust for the re-verified edition before revealing the app so the splash
  // shows the right one; never let a slow backend stall boot.
  await Promise.race([license.refresh(), new Promise((resolve) => window.setTimeout(resolve, 1500))]);
  booted.value = true;
  void loadCachedLibrary();
  if (!firstRun) {
    // The wizard owns the first fetch; returning users refresh silently.
    void profile.syncRemote();
    if (sources.hasSources) void loadRemoteLibrary({ silent: true });
  }
}

function onWizardFinish() {
  if (sources.hasSources) void loadRemoteLibrary();
  else void loadCachedLibrary();
}

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
/** Re-verify the Pro license from scratch whenever the app regains focus. */
function recheckLicense() { void license.refresh(); }
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

onMounted(() => { window.addEventListener("hashchange", syncViewFromHash); window.addEventListener("keydown", onGlobalKeydown); window.addEventListener("beforeunload", savePlaybackPositionOnLeave); window.addEventListener("pagehide", savePlaybackPositionOnLeave); window.addEventListener("focus", recheckLicense); document.addEventListener("visibilitychange", onVisibilityChange); void initAudio(); void network.start(); void cache.start(); bootDesktopLyrics(); void invoke<string>("ping").catch((error) => console.warn("[welkin] ping failed", describeError(error))); void bootstrap(); });
onUnmounted(() => { savePlaybackPosition(true); stopDesktopLyrics(); window.removeEventListener("hashchange", syncViewFromHash); window.removeEventListener("keydown", onGlobalKeydown); window.removeEventListener("beforeunload", savePlaybackPositionOnLeave); window.removeEventListener("pagehide", savePlaybackPositionOnLeave); window.removeEventListener("focus", recheckLicense); document.removeEventListener("visibilitychange", onVisibilityChange); void profile.flush(); });
</script>

<template>
  <AmllPlayerView v-if="amllMounted && player.currentTrack" :active="amllActive" :return-view="baseView" @navigate="navigate" @closed="amllMounted = false" />
  <Transition name="slide"><PlayerView v-if="view === 'player' && player.currentTrack && !lyrics.useAmll" :return-view="baseView" @navigate="navigate" @queue="toggleQueue" @open-artist="openArtist" @open-album="openAlbum" /></Transition>
  <Transition name="info"><TrackInfoView v-if="view === 'track-info' && viewingTrack" :path="viewingTrack.path ?? null" :source-id="viewingTrack.sourceId" @back="closeTrackInfo" /></Transition>
  <SetupWizard v-if="booted && profile.firstRun" v-model:theme="theme" v-model:accent="accent" @finish="onWizardFinish" />
  <div class="flex h-screen flex-col overflow-hidden bg-bg text-fg" :class="booted ? '' : 'opacity-0'">
    <AppNavigation placement="header" :active-view="baseView" @navigate="navigate" />
    <div class="relative flex min-h-0 flex-1">
      <AppNavigation placement="sidebar" :active-view="baseView" :collapsed="sidebarCollapsed" :active-playlist-id="playlistFilter" :active-artist="artistFilter" :active-album="albumFilter" :active-source="sourceFilter" @navigate="navigate" @toggle="toggleSidebar" @create-playlist="createPlaylist" @open-playlist="openUserPlaylist" />
      <main class="min-w-0 flex-1 overflow-y-auto">
        <Transition name="page" mode="out-in">
          <LibraryView v-if="baseView === 'library'" key="library" :loading="loadingLibrary" :downloading-track-id="downloadingTrackId" @settings="navigate('settings')" @navigate="navigate" @open-playlist="openUserPlaylist" @download-metadata="downloadMetadata" @edit-lyrics="editLyrics" @edit-info="editInfo" @show-info="showTrackInfo" />
          <TracksView v-else-if="baseView === 'tracks'" key="tracks" @open-artist="openArtist" @open-album="openAlbum" :playlist-id="playlistFilter" :artist="artistFilter" :album="albumFilter" :source-id="sourceFilter" :loading="loadingLibrary" :refreshing="refreshing" :downloading-track-id="downloadingTrackId" @refresh="refreshLibrary" @edit="editPlaylist" @download-metadata="downloadMetadata" @edit-lyrics="editLyrics" @edit-info="editInfo" @show-info="showTrackInfo" />
          <SourcesView v-else-if="baseView === 'sources'" key="sources" :loading="loadingLibrary" @open="openSource" @manage="openSourceConfig('sources')" />
          <SourceConfigView v-else-if="baseView === 'source-config'" key="source-config" @back="closeSourceConfig" />
          <GroupedLibraryView v-else-if="baseView === 'artists' || baseView === 'albums'" :key="baseView" :kind="baseView === 'artists' ? 'artists' : 'albums'" :loading="loadingLibrary" @open="onOpenGroup" />
          <FavoritesView v-else-if="baseView === 'favorites'" key="favorites" @open-artist="openArtist" @open-album="openAlbum" :loading="loadingLibrary" :downloading-track-id="downloadingTrackId" @download-metadata="downloadMetadata" @edit-lyrics="editLyrics" @edit-info="editInfo" @show-info="showTrackInfo" />
          <PlaylistsView v-else-if="baseView === 'playlists'" key="playlists" @open-playlist="openUserPlaylist" @edit-playlist="editPlaylist" @create-playlist="createPlaylist" />
          <StatsView v-else-if="baseView === 'stats'" key="stats" :loading="loadingLibrary" :downloading-track-id="downloadingTrackId" @download-metadata="downloadMetadata" @edit-lyrics="editLyrics" @edit-info="editInfo" @show-info="showTrackInfo" />
          <PlaylistEditorView v-else-if="baseView === 'playlist-new'" key="playlist-new" :playlist-id="editingPlaylist" @done="onPlaylistSaved" @deleted="onPlaylistDeleted" @cancel="onPlaylistEditorCancel" />
          <LyricsEditorView v-else-if="baseView === 'lyrics-edit'" key="lyrics-edit" :path="editingLyricsTrack?.path ?? null" :source-id="editingLyricsTrack?.sourceId" @back="navigate('library')" @friendly-error="showError" />
          <TrackEditorView v-else-if="baseView === 'track-edit'" key="track-edit" :path="editingTrack?.path ?? null" :source-id="editingTrack?.sourceId" @back="navigate('library')" @friendly-error="showError" />
          <SponsorView v-else-if="baseView === 'sponsor'" key="sponsor" @back="navigate('library')" />
          <SettingsView v-else key="settings" v-model:theme="theme" v-model:accent="accent" @sponsor="navigate('sponsor')" @sources="openSourceConfig('settings')" @friendly-error="showError" />
        </Transition>
      </main>
      <Transition name="queue-panel"><div v-if="showQueue" class="absolute inset-y-0 right-0 z-20 w-80 border-l border-line bg-surface"><QueuePanel @close="closeQueue" /></div></Transition>
    </div>
    <PlayerBar v-if="player.currentTrack" @open="navigate('player')" @queue="toggleQueue" @open-artist="openArtist" @open-album="openAlbum" />
    <AppNavigation placement="mobile" :active-view="baseView" :active-playlist-id="playlistFilter" :active-artist="artistFilter" :active-album="albumFilter" :active-source="sourceFilter" @navigate="navigate" @create-playlist="createPlaylist" @open-playlist="openUserPlaylist" />
    <ToastStack />
  </div>
</template>
