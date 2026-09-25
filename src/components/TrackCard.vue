<script setup lang="ts">
import { Play, Trash2 } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import type { Track } from "../stores/player";
import { initial } from "../lib/format";

withDefaults(defineProps<{ track: Track; active?: boolean; playing?: boolean; removable?: boolean }>(), { removable: false });
const emit = defineEmits<{ play: [track: Track]; menu: [event: MouseEvent, track: Track]; remove: [track: Track] }>();
const { t } = useI18n();
</script>

<template>
  <article
    class="group relative cursor-pointer border border-line bg-surface p-3 transition-all hover:-translate-y-0.5 hover:border-accent"
    :class="active ? 'border-accent' : ''"
    @click="emit('play', track)"
    @contextmenu.prevent="emit('menu', $event, track)"
  >
    <div class="relative aspect-square w-full overflow-hidden" :style="{ backgroundColor: track.color }">
      <img v-if="track.cover" :src="track.cover" alt="" class="absolute inset-0 h-full w-full object-cover" />
      <span v-else class="absolute inset-0 grid place-items-center text-5xl font-black text-white/90">{{ initial(track) }}</span>
      <button class="ak-clip-tr absolute bottom-0 right-0 grid h-10 w-10 place-items-center bg-fg text-bg opacity-0 transition-opacity group-hover:opacity-100" :title="t('controls.play')" @click.stop="emit('play', track)">
        <Play :size="16" />
      </button>
      <button v-if="removable" type="button" class="absolute right-0 top-0 grid h-8 w-8 place-items-center bg-red-500 text-white opacity-0 transition-opacity group-hover:opacity-100" :title="t('library.playlists.removeTrack')" @click.stop="emit('remove', track)">
        <Trash2 :size="15" />
      </button>
      <span v-if="active && playing" class="absolute left-2 top-2 grid h-6 w-6 place-items-center bg-accent text-accent-fg"><span class="ak-eq"><i></i><i></i><i></i></span></span>
    </div>
    <p class="mt-3 truncate text-sm font-semibold tracking-wide" :class="active ? 'text-accent' : ''">{{ track.title }}</p>
    <p class="truncate text-xs text-muted">{{ track.artist }}</p>
  </article>
</template>
