<script setup lang="ts">
import { computed, ref } from "vue";
import { Check, ImagePlus, Music2, Search, Trash2, X } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { usePlayerStore } from "../stores/player";
import { useProfileStore } from "../stores/profile";
import type { Playlist } from "../lib/profile";
import { initial } from "../lib/format";
import PlaylistCover from "../components/PlaylistCover.vue";
import CoverCropper from "../components/CoverCropper.vue";

const props = withDefaults(defineProps<{ playlistId?: string | null }>(), { playlistId: null });
const emit = defineEmits<{ done: [id: string]; cancel: [] }>();
const { t } = useI18n();
const player = usePlayerStore();
const profile = useProfileStore();

const editing = computed(() => props.playlistId ? profile.playlists.find((item) => item.id === props.playlistId) ?? null : null);
const name = ref(editing.value?.name ?? "");
const cover = ref<string | undefined>(editing.value?.cover);
const coverTrack = ref<string | undefined>(editing.value?.coverTrack);
const mode = ref<"upload" | "icon">(coverTrack.value ? "icon" : "upload");
const cropSrc = ref<string | null>(null);
const trackQuery = ref("");

const preview = computed<Playlist>(() => ({ id: "preview", name: name.value || t("playlistEditor.untitled"), tracks: [], cover: cover.value, coverTrack: coverTrack.value }));
const pickerTracks = computed(() => {
  const value = trackQuery.value.trim().toLocaleLowerCase();
  return player.tracks.filter((track) => track.path && (!value || `${track.title} ${track.artist}`.toLocaleLowerCase().includes(value)));
});

function onFile(event: Event) {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0];
  if (!file) return;
  const reader = new FileReader();
  reader.onload = () => { cropSrc.value = String(reader.result); };
  reader.readAsDataURL(file);
  input.value = "";
}
function applyCrop(value: string) { cover.value = value; coverTrack.value = undefined; cropSrc.value = null; }
function chooseTrack(path: string) { coverTrack.value = path; cover.value = undefined; }
function clearCover() { cover.value = undefined; coverTrack.value = undefined; }

function save() {
  const trimmed = name.value.trim();
  if (!trimmed) return;
  if (editing.value) {
    profile.updatePlaylist(editing.value.id, { name: trimmed, cover: cover.value, coverTrack: coverTrack.value });
    emit("done", editing.value.id);
  } else {
    const playlist = profile.createPlaylist(trimmed);
    profile.updatePlaylist(playlist.id, { cover: cover.value, coverTrack: coverTrack.value });
    emit("done", playlist.id);
  }
}
</script>

<template>
  <div class="mx-auto w-full max-w-4xl px-6 py-6 lg:px-8 lg:py-8">
    <header class="border-b border-line pb-6">
      <p class="flex items-center gap-2 text-[13px] font-semibold uppercase tracking-[0.35em] text-accent"><span class="h-2 w-2 bg-accent"></span>{{ t("playlistEditor.eyebrow") }}</p>
      <h1 class="mt-4 text-4xl font-black leading-none tracking-tight sm:text-5xl">{{ editing ? t("playlistEditor.editTitle") : t("playlistEditor.createTitle") }}</h1>
    </header>

    <div class="mt-8 grid gap-8 lg:grid-cols-[240px_minmax(0,1fr)]">
      <div class="flex flex-col items-center gap-4">
        <PlaylistCover :playlist="preview" class="ak-frame h-40 w-40 border border-line text-5xl" :icon-size="48" />
        <label class="grid w-full gap-2 text-[13px] font-semibold text-dim">
          {{ t("playlistEditor.name") }}
          <input v-model="name" maxlength="48" :placeholder="t('playlistEditor.namePlaceholder')" class="h-10 border border-line bg-bg px-3 text-sm text-fg outline-none focus:border-accent" @keydown.enter="save" />
        </label>
      </div>

      <div class="min-w-0">
        <h2 class="text-sm font-bold uppercase tracking-[0.2em]">{{ t("playlistEditor.cover") }}</h2>
        <div class="mt-3 inline-flex border border-line">
          <button type="button" class="flex h-9 items-center gap-2 px-4 text-[12px] font-semibold uppercase tracking-[0.15em]" :class="mode === 'upload' ? 'bg-accent text-accent-fg' : 'text-muted hover:text-fg'" @click="mode = 'upload'"><ImagePlus :size="14" />{{ t("playlistEditor.upload") }}</button>
          <button type="button" class="flex h-9 items-center gap-2 border-l border-line px-4 text-[12px] font-semibold uppercase tracking-[0.15em]" :class="mode === 'icon' ? 'bg-accent text-accent-fg' : 'text-muted hover:text-fg'" @click="mode = 'icon'"><Music2 :size="14" />{{ t("playlistEditor.chooseIcon") }}</button>
        </div>

        <div v-if="mode === 'upload'" class="mt-4 grid gap-3">
          <label class="ak-frame grid cursor-pointer place-items-center gap-2 border border-dashed border-line bg-surface/50 px-6 py-10 text-center transition-colors hover:border-accent">
            <ImagePlus :size="26" class="text-dim" />
            <span class="text-sm font-semibold">{{ t("playlistEditor.chooseFile") }}</span>
            <span class="text-[12px] text-dim">{{ t("playlistEditor.uploadHint") }}</span>
            <input type="file" accept="image/*" class="hidden" @change="onFile" />
          </label>
          <button v-if="cover" type="button" class="flex items-center gap-2 justify-self-start text-[12px] font-semibold text-dim transition-colors hover:text-red-500" @click="clearCover"><Trash2 :size="14" />{{ t("playlistEditor.remove") }}</button>
        </div>

        <div v-else class="mt-4 grid gap-3">
          <p class="text-[12px] text-dim">{{ t("playlistEditor.pickTrack") }}</p>
          <label class="flex h-10 items-center gap-2 border border-line bg-surface px-3"><Search :size="15" class="text-dim" /><input v-model="trackQuery" class="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-dim" :placeholder="t('library.search')" /></label>
          <p v-if="pickerTracks.length === 0" class="py-8 text-center text-sm text-muted">{{ t("playlistEditor.noTracks") }}</p>
          <div v-else class="grid max-h-72 grid-cols-3 gap-3 overflow-y-auto pr-1 sm:grid-cols-4">
            <button v-for="track in pickerTracks" :key="track.id" type="button" class="group grid gap-2 text-left" @click="chooseTrack(track.path as string)">
              <span class="relative grid aspect-square place-items-center overflow-hidden text-2xl font-black text-white/90 ring-offset-2 ring-offset-surface transition-all" :class="coverTrack === track.path ? 'ring-2 ring-accent' : 'group-hover:ring-2 group-hover:ring-line-strong'" :style="{ backgroundColor: track.color }">
                <img v-if="track.cover" :src="track.cover" alt="" class="h-full w-full object-cover" />
                <template v-else>{{ initial(track) }}</template>
                <span v-if="coverTrack === track.path" class="absolute bottom-0 right-0 grid h-6 w-6 place-items-center bg-accent text-accent-fg"><Check :size="14" /></span>
              </span>
              <span class="truncate text-[11px] font-semibold">{{ track.title }}</span>
            </button>
          </div>
        </div>
      </div>
    </div>

    <footer class="mt-10 flex justify-end gap-3 border-t border-line pt-6">
      <button type="button" class="ak-clip-tr h-11 border border-line px-5 text-[13px] font-semibold" @click="emit('cancel')"><span class="flex items-center gap-2"><X :size="15" />{{ t("playlistEditor.cancel") }}</span></button>
      <button type="button" class="ak-clip-tr h-11 bg-accent px-6 text-[13px] font-bold text-accent-fg disabled:opacity-50" :disabled="!name.trim()" @click="save">{{ editing ? t("playlistEditor.save") : t("playlistEditor.create") }}</button>
    </footer>

    <CoverCropper v-if="cropSrc" :src="cropSrc" @confirm="applyCrop" @cancel="cropSrc = null" />
  </div>
</template>
