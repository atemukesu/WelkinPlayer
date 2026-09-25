<script setup lang="ts">
import { computed } from "vue";
import { Ellipsis, Pause, Play, Trash2 } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { usePlayerStore } from "../stores/player";
import type { Track } from "../stores/player";
import { initial, pad } from "../lib/format";
import { trackViewMode } from "../lib/ui";
import TrackCard from "./TrackCard.vue";

withDefaults(defineProps<{ tracks: Track[]; emptyKey: string; removable?: boolean }>(), { removable: false });
const emit = defineEmits<{ play: [track: Track]; menu: [event: MouseEvent, track: Track]; details: [track: Track]; remove: [track: Track] }>();
const { t } = useI18n();
const player = usePlayerStore();
const currentId = computed(() => player.currentTrack?.id);
const playing = computed(() => player.isPlaying);
</script>

<template>
  <div>
    <template v-if="trackViewMode === 'grid'">
      <div v-if="tracks.length === 0" class="py-10 text-center text-sm text-muted">{{ t(emptyKey) }}</div>
      <div v-else class="grid grid-cols-2 gap-4 sm:grid-cols-3 xl:grid-cols-4">
        <TrackCard
          v-for="track in tracks"
          :key="track.id"
          :track="track"
          :active="track.id === currentId"
          :playing="playing"
          :removable="removable"
          @play="emit('play', $event)"
          @menu="(event: MouseEvent, track: Track) => emit('menu', event, track)"
          @remove="emit('remove', $event)"
        />
      </div>
    </template>

    <template v-else>
      <div class="hidden grid-cols-[36px_44px_minmax(0,1fr)_140px_56px_44px_44px] items-center gap-4 border-t border-line px-3 py-2 text-[10px] font-semibold uppercase tracking-[0.2em] text-dim md:grid"><span>#</span><span></span><span>{{ t("library.colTrack") }}</span><span>{{ t("library.colAlbum") }}</span><span class="text-right">{{ t("library.colTime") }}</span><span></span><span></span></div>
      <div class="border-b border-line">
        <div
          v-for="(track, index) in tracks"
          :key="track.id"
          role="button"
          tabindex="0"
          class="grid w-full cursor-pointer grid-cols-[36px_44px_minmax(0,1fr)_44px_40px] items-center gap-4 border-t border-line px-3 py-2 text-left md:grid-cols-[36px_44px_minmax(0,1fr)_140px_56px_44px_44px]"
          :class="track.id === currentId ? 'ak-select relative z-10 border-transparent' : 'ak-hover'"
          @click="emit('play', track)"
          @keydown.enter="emit('play', track)"
          @keydown.space.prevent="emit('play', track)"
          @contextmenu.prevent="emit('menu', $event, track)"
        >
          <span class="font-mono text-[13px] tabular-nums text-dim">{{ pad(index + 1) }}</span>
          <span class="grid h-11 w-11 place-items-center overflow-hidden text-lg font-black text-white/90" :style="{ backgroundColor: track.color }"><img v-if="track.cover" :src="track.cover" alt="" class="h-full w-full object-cover" /><template v-else>{{ initial(track) }}</template></span>
          <span class="grid min-w-0 gap-0.5"><strong class="truncate text-sm font-semibold tracking-wide" :class="track.id === currentId ? 'text-accent' : 'text-fg'">{{ track.title }}</strong><small class="truncate text-xs text-muted">{{ track.artist }}</small></span>
          <span class="hidden truncate text-xs text-muted md:block">{{ track.album }}</span>
          <span class="hidden text-right font-mono text-[13px] tabular-nums text-muted md:block">{{ track.duration }}</span>
          <span class="grid place-items-center text-dim"><span v-if="track.id === currentId && playing" class="text-accent"><span class="ak-eq"><i></i><i></i><i></i></span></span><Pause v-else-if="track.id === currentId" :size="16" class="text-accent" /><Play v-else :size="16" /></span>
          <button v-if="removable" type="button" class="grid h-8 w-8 place-items-center text-dim transition-colors hover:text-red-500" :title="t('library.playlists.removeTrack')" @click.stop="emit('remove', track)"><Trash2 :size="15" /></button>
          <button v-else type="button" class="grid h-8 w-8 place-items-center text-dim transition-colors hover:text-fg" :title="t('controls.expand')" @click.stop="emit('details', track)"><Ellipsis :size="18" /></button>
        </div>
        <p v-if="tracks.length === 0" class="py-10 text-center text-sm text-muted">{{ t(emptyKey) }}</p>
      </div>
    </template>
  </div>
</template>
