<script setup lang="ts">
import { computed } from "vue";
import { ListMusic } from "@lucide/vue";
import { usePlayerStore } from "../stores/player";
import { useProfileStore } from "../stores/profile";
import type { Playlist } from "../lib/profile";
import { initial } from "../lib/format";

const props = withDefaults(defineProps<{ playlist: Playlist; iconSize?: number; overrideSrc?: string | null }>(), { iconSize: 20, overrideSrc: null });
const player = usePlayerStore();
const profile = useProfileStore();
const track = computed(() => props.playlist.coverTrack ? player.tracks.find((item) => item.path === props.playlist.coverTrack) : undefined);
const src = computed(() => props.overrideSrc ?? profile.playlistCoverUrl(props.playlist));
const hasImage = computed(() => !!src.value || !!track.value?.cover);
const background = computed(() => track.value?.color ?? "var(--accent)");
const foreground = computed(() => track.value ? "text-white/90" : "text-accent-fg");
</script>

<template>
  <span class="relative grid shrink-0 place-items-center overflow-hidden" :class="hasImage ? '' : foreground" :style="hasImage ? {} : { backgroundColor: background }">
    <img v-if="src" :src="src" alt="" decoding="async" class="h-full w-full object-cover" />
    <img v-else-if="track?.cover" :src="track.cover" alt="" loading="lazy" decoding="async" class="h-full w-full object-cover" />
    <span v-else-if="track" class="font-black leading-none" :style="{ fontSize: `${iconSize * 1.35}px` }">{{ initial(track) }}</span>
    <ListMusic v-else :size="iconSize" :stroke-width="1.8" />
  </span>
</template>
