<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { VList, Virtualizer } from "virtua/vue";
import { Check, Pause, Play, Trash2 } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { usePlayerStore } from "../stores/player";
import type { Track } from "../stores/player";
import { initial, pad } from "../lib/format";
import { trackViewMode } from "../lib/ui";
import TrackCard from "./TrackCard.vue";

const props = withDefaults(defineProps<{ tracks: Track[]; emptyKey: string; removable?: boolean; selectable?: boolean; selected?: Set<string>; scroller?: HTMLElement | null }>(), { removable: false, selectable: false, scroller: null });
const emit = defineEmits<{ play: [track: Track]; menu: [event: MouseEvent, track: Track]; remove: [track: Track]; "update:selected": [value: Set<string>] }>();
const { t } = useI18n();
const player = usePlayerStore();
const currentId = computed(() => player.currentTrack?.id);
const playing = computed(() => player.isPlaying);

// The trailing action column only exists when a remove button is shown; without
// it the row keeps the space for the play indicator as the last column.
const showRemove = computed(() => props.removable && !props.selectable);
// Narrow screens drop the index column entirely, so the row condenses to a
// three/four column track row; the index reappears with album/time at md.
const listGrid = computed(() => {
  const mobile = props.selectable
    ? "grid-cols-[36px_44px_minmax(0,1fr)_44px]"
    : (showRemove.value ? "grid-cols-[44px_minmax(0,1fr)_44px_40px]" : "grid-cols-[44px_minmax(0,1fr)_44px]");
  const wide = showRemove.value
    ? "md:grid-cols-[36px_44px_minmax(0,1fr)_140px_56px_44px_44px]"
    : "md:grid-cols-[36px_44px_minmax(0,1fr)_140px_56px_44px]";
  return `${mobile} ${wide}`;
});
const listHeaderGrid = computed(() => showRemove.value
  ? "grid-cols-[36px_44px_minmax(0,1fr)_140px_56px_44px_44px]"
  : "grid-cols-[36px_44px_minmax(0,1fr)_140px_56px_44px]");

const GAP = 16;
const SCROLLBAR = 8;
const LIST_ROW_HEIGHT = 61;

const rootEl = ref<HTMLElement | null>(null);
const virtualHostEl = ref<HTMLElement | null>(null);
const width = ref(0);
const startMargin = ref(0);
let observer: ResizeObserver | undefined;
let scrollerObserver: ResizeObserver | undefined;

function measure() { width.value = rootEl.value?.clientWidth ?? 0; }

// When the page itself scrolls, the virtualizer is mounted after the page
// header/toolbar, so it has to be told how much content sits above it for its
// scroll maths to line up. Measured from the DOM so wrapping/resizing is safe.
function measureStartMargin() {
  const scroller = props.scroller;
  const host = virtualHostEl.value;
  if (!scroller || !host) { startMargin.value = 0; return; }
  const value = Math.round(host.getBoundingClientRect().top - scroller.getBoundingClientRect().top + scroller.scrollTop);
  if (value >= 0) startMargin.value = value;
}

const external = computed(() => !!props.scroller);
const listComponent = computed(() => (external.value ? Virtualizer : VList));
const listProps = computed(() => (external.value ? { scrollRef: props.scroller ?? undefined, startMargin: startMargin.value } : {}));

function observeScroller() {
  scrollerObserver?.disconnect();
  const scroller = props.scroller;
  if (typeof ResizeObserver === "undefined" || !scroller) return;
  scrollerObserver = new ResizeObserver(() => measureStartMargin());
  scrollerObserver.observe(scroller);
  // Content above the list (banners, wrapped headings) can grow/shrink without
  // changing the scroller's own box, so watch the scroll content too.
  const content = scroller.firstElementChild;
  if (content instanceof HTMLElement) scrollerObserver.observe(content);
}

onMounted(() => {
  measure();
  if (typeof ResizeObserver !== "undefined" && rootEl.value) {
    observer = new ResizeObserver(measure);
    observer.observe(rootEl.value);
  }
  observeScroller();
  void nextTick(measureStartMargin);
});
watch(() => props.scroller, () => { observeScroller(); void nextTick(measureStartMargin); });
watch([() => props.tracks.length, trackViewMode], () => { void nextTick(measureStartMargin); });
onBeforeUnmount(() => { observer?.disconnect(); scrollerObserver?.disconnect(); });

const columns = computed(() => Math.max(2, Math.min(4, Math.round(width.value / 200))));
const colWidth = computed(() => {
  const cols = columns.value;
  return Math.max(120, (Math.max(0, width.value - SCROLLBAR) - GAP * (cols - 1)) / cols);
});
const gridRowIndexes = computed(() => Array.from({ length: Math.ceil(props.tracks.length / columns.value) }, (_, index) => index));
const gridRowHeight = computed(() => Math.round(colWidth.value + 88));
function gridTrack(row: number, col: number): Track { return props.tracks[row * columns.value + col]; }
function gridInRange(row: number, col: number): boolean { return row * columns.value + col < props.tracks.length; }

const selection = computed(() => props.selected ?? new Set<string>());
const selectableTracks = computed(() => props.tracks.filter((track) => track.path));
const allSelected = computed(() => selectableTracks.value.length > 0 && selectableTracks.value.every((track) => selection.value.has(track.path as string)));
const selectedCount = computed(() => props.tracks.filter(isSelected).length);
let anchorPath: string | null = null;

function isSelected(track: Track): boolean {
  return !!track.path && selection.value.has(track.path);
}
function commit(next: Set<string>) { emit("update:selected", next); }
function toggleAll() {
  if (allSelected.value) commit(new Set());
  else commit(new Set(selectableTracks.value.map((track) => track.path as string)));
}
function selectRange(track: Track) {
  const paths = selectableTracks.value.map((item) => item.path as string);
  const to = paths.indexOf(track.path as string);
  if (to < 0) return;
  const from = anchorPath ? paths.indexOf(anchorPath) : to;
  if (from < 0) { commit(new Set([track.path as string])); return; }
  commit(new Set(paths.slice(Math.min(from, to), Math.max(from, to) + 1)));
}
function selectOnly(track: Track) {
  if (track.path) commit(new Set([track.path]));
}
function toggleOne(track: Track) {
  if (!track.path) return;
  const next = new Set(selection.value);
  if (next.has(track.path)) next.delete(track.path);
  else next.add(track.path);
  commit(next);
}
function activate(track: Track, event?: MouseEvent) {
  if (!props.selectable) { emit("play", track); return; }
  if (!track.path) return;
  if (event?.shiftKey) { selectRange(track); return; }
  anchorPath = track.path;
  if (event?.ctrlKey || event?.metaKey) selectOnly(track);
  else toggleOne(track);
}
</script>

<template>
  <div ref="rootEl" :class="external ? 'w-full' : 'flex h-full flex-col'">
    <div v-if="selectable" class="flex shrink-0 items-center gap-3 border-b border-line px-3 py-2">
      <button type="button" class="grid h-5 w-5 place-items-center border-2 transition-colors" :class="allSelected ? 'border-accent bg-accent text-accent-fg' : 'border-line-strong text-transparent hover:border-accent'" :title="allSelected ? t('library.deselectAll') : t('library.selectAll')" @click="toggleAll"><Check :size="12" :stroke-width="3" /></button>
      <span class="text-[11px] font-semibold uppercase tracking-[0.2em] text-dim">{{ selectedCount ? t("library.selected", { count: selectedCount }) : t("library.selectAll") }}</span>
    </div>
    <div :class="external ? undefined : 'min-h-0 flex-1'">
    <template v-if="trackViewMode === 'grid'">
      <div v-if="tracks.length" ref="virtualHostEl" :class="external ? 'w-full' : 'h-full'">
      <component :is="listComponent" :data="gridRowIndexes" :item-size="gridRowHeight" v-bind="listProps">
        <template #default="{ item: row }">
          <div class="grid gap-4 px-2 pb-4" :style="{ gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))` }">
            <template v-for="col in columns" :key="col">
              <TrackCard
                v-if="gridInRange(row, col - 1)"
                :track="gridTrack(row, col - 1)"
                :active="gridTrack(row, col - 1).id === currentId"
                :playing="playing"
                :removable="removable"
                :selectable="selectable"
                :selected="isSelected(gridTrack(row, col - 1))"
                @play="emit('play', $event)"
                @menu="(event: MouseEvent, track: Track) => emit('menu', event, track)"
                @remove="emit('remove', $event)"
                @toggle="activate"
              />
              <span v-else></span>
            </template>
          </div>
        </template>
      </component>
      </div>
      <div v-else class="py-10 text-center text-sm text-muted">{{ t(emptyKey) }}</div>
    </template>

    <template v-else>
      <div :class="external ? undefined : 'flex h-full flex-col'">
        <div class="hidden shrink-0 items-center gap-4 px-3 py-2 text-[10px] font-semibold uppercase tracking-[0.2em] text-dim md:grid" :class="listHeaderGrid"><span>#</span><span></span><span>{{ t("library.colTrack") }}</span><span>{{ t("library.colAlbum") }}</span><span class="text-right">{{ t("library.colTime") }}</span><span></span><span v-if="showRemove"></span></div>
        <div v-if="tracks.length" ref="virtualHostEl" :class="external ? 'w-full' : 'min-h-0 flex-1'">
        <component :is="listComponent" :data="tracks" :item-size="LIST_ROW_HEIGHT" v-bind="listProps">
          <template #default="{ item: track, index }">
            <div
              role="button"
              tabindex="0"
              :key="track.id"
              class="grid w-full cursor-pointer items-center gap-4 border-t border-line px-3 py-2 text-left"
              :class="[listGrid, selectable ? (isSelected(track) ? 'bg-accent/10' : 'ak-hover') : (track.id === currentId ? 'ak-select relative z-10 border-transparent' : 'ak-hover')]"
              @click="activate(track, $event)"
              @keydown.enter="activate(track)"
              @keydown.space.prevent="activate(track)"
              @contextmenu.prevent="emit('menu', $event, track)"
            >
              <span v-if="selectable" class="grid h-5 w-5 place-items-center border-2" :class="isSelected(track) ? 'border-accent bg-accent text-accent-fg' : 'border-line-strong text-transparent'"><Check :size="12" :stroke-width="3" /></span>
              <span v-else class="hidden font-mono text-[13px] tabular-nums text-dim md:block">{{ pad(index + 1) }}</span>
              <span class="grid h-11 w-11 place-items-center overflow-hidden text-lg font-black text-white/90" :style="{ backgroundColor: track.color }"><img v-if="track.cover" :src="track.cover" alt="" loading="lazy" decoding="async" class="h-full w-full object-cover" /><template v-else>{{ initial(track) }}</template></span>
              <span class="grid min-w-0 gap-0.5"><strong class="truncate text-sm font-semibold tracking-wide" :class="track.id === currentId ? 'text-accent' : 'text-fg'">{{ track.title }}</strong><small class="truncate text-xs text-muted">{{ track.artist }}</small></span>
              <span class="hidden truncate text-xs text-muted md:block">{{ track.album }}</span>
              <span class="hidden text-right font-mono text-[13px] tabular-nums text-muted md:block">{{ track.duration }}</span>
              <span class="grid place-items-center text-dim"><span v-if="track.id === currentId && playing" class="text-accent"><span class="ak-eq"><i></i><i></i><i></i></span></span><Pause v-else-if="track.id === currentId" :size="16" class="text-accent" /><Play v-else :size="16" /></span>
              <button v-if="showRemove" type="button" class="grid h-8 w-8 place-items-center text-dim transition-colors hover:text-red-500" :title="t('library.playlists.removeTrack')" @click.stop="emit('remove', track)"><Trash2 :size="15" /></button>
            </div>
          </template>
        </component>
        </div>
        <p v-else class="shrink-0 py-10 text-center text-sm text-muted">{{ t(emptyKey) }}</p>
      </div>
    </template>
    </div>
  </div>
</template>
