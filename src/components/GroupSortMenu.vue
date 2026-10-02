<!--
 Copyright 2026 Atemukesu
 SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { ArrowDown, ArrowUp, ArrowUpDown, Check } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { useGroupSort, type GroupSortContext, type GroupSortKey } from "../lib/sort";

const props = defineProps<{ context: GroupSortContext }>();
const { t } = useI18n();
const sortKey = computed(() => useGroupSort(props.context).key.value);
const sortDir = computed(() => useGroupSort(props.context).dir.value);
const open = ref(false);
const root = ref<HTMLElement>();
const menu = ref<HTMLElement>();
const position = ref({ x: 0, y: 0 });

const options: { value: GroupSortKey; labelKey: string }[] = [
  { value: "default", labelKey: "library.sortDefault" },
  { value: "name", labelKey: "library.sortName" },
  { value: "count", labelKey: "library.sortCount" },
];

// Keep the menu fully inside the viewport, measuring its real rendered size.
function clampToViewport() {
  const rootEl = root.value;
  const menuEl = menu.value;
  if (!rootEl || !menuEl) return;
  // Extra margin covers the offset shadow and angled corners.
  const margin = 8;
  const gap = 8;
  const rect = rootEl.getBoundingClientRect();
  const width = Math.min(menuEl.offsetWidth || 160, window.innerWidth - margin * 2);
  const height = menuEl.offsetHeight;
  // The trigger sits on the right side of a toolbar, so anchor the menu's right edge to it.
  let x = rect.right - width;
  if (x + width + margin > window.innerWidth) x = window.innerWidth - width - margin;
  if (x < margin) x = margin;

  let y = rect.bottom + gap;
  if (y + height + margin > window.innerHeight) {
    const above = rect.top - gap - height;
    y = above >= margin ? above : Math.max(margin, window.innerHeight - height - margin);
  }
  position.value = { x, y };
}

function scheduleClamp() {
  void nextTick(clampToViewport);
}

function close() {
  open.value = false;
}

function choose(value: GroupSortKey) {
  useGroupSort(props.context).key.value = value;
  close();
}

function toggleDir() {
  const dir = useGroupSort(props.context).dir;
  dir.value = dir.value === "asc" ? "desc" : "asc";
}

function onPointerDown(event: PointerEvent) {
  if (!open.value) return;
  const target = event.target as Node | null;
  if (root.value?.contains(target) || menu.value?.contains(target)) return;
  open.value = false;
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === "Escape") open.value = false;
}

function onResize() {
  if (open.value) scheduleClamp();
}

watch(open, (isOpen) => { if (isOpen) scheduleClamp(); });
onMounted(() => {
  document.addEventListener("pointerdown", onPointerDown);
  window.addEventListener("keydown", onKeydown);
  window.addEventListener("resize", onResize);
  // Capture covers scrolls happening in any ancestor scroll container.
  window.addEventListener("scroll", onResize, true);
});
onBeforeUnmount(() => {
  document.removeEventListener("pointerdown", onPointerDown);
  window.removeEventListener("keydown", onKeydown);
  window.removeEventListener("resize", onResize);
  window.removeEventListener("scroll", onResize, true);
  open.value = false;
});
</script>

<template>
  <div ref="root" class="group-sort-menu">
    <button type="button" class="grid h-10 w-10 shrink-0 place-items-center border transition-colors" :class="sortKey !== 'default' ? 'border-accent text-accent' : 'border-line text-dim hover:border-accent hover:text-accent'" :title="t('library.sort')" :aria-expanded="open" @click="open = !open"><ArrowUpDown :size="16" :stroke-width="2" /></button>
    <Teleport to="body">
      <Transition name="group-sort-menu-pop">
        <div v-if="open" ref="menu" class="group-sort-menu__pop" role="listbox" :aria-label="t('library.sort')" :style="{ left: `${position.x}px`, top: `${position.y}px` }">
          <span class="group-sort-menu__backdrop group-sort-menu__backdrop--gray" aria-hidden="true"></span>
          <span class="group-sort-menu__backdrop group-sort-menu__backdrop--white" aria-hidden="true"></span>
          <div class="group-sort-menu__content">
            <button v-for="option in options" :key="option.value" type="button" class="group-sort-menu__item" :class="sortKey === option.value ? 'group-sort-menu__item--selected' : ''" :aria-selected="sortKey === option.value" @click="choose(option.value)">
              <span class="group-sort-menu__label">{{ t(option.labelKey) }}</span>
              <Check v-if="sortKey === option.value" :size="14" class="group-sort-menu__check" />
            </button>
          </div>
        </div>
      </Transition>
    </Teleport>
    <button v-if="sortKey !== 'default'" type="button" class="grid h-10 w-10 shrink-0 place-items-center border border-line text-dim transition-colors hover:border-accent hover:text-accent" :title="sortDir === 'asc' ? t('library.sortAsc') : t('library.sortDesc')" @click="toggleDir"><ArrowUp v-if="sortDir === 'asc'" :size="16" :stroke-width="2" /><ArrowDown v-else :size="16" :stroke-width="2" /></button>
  </div>
</template>

<style scoped>
.group-sort-menu { position: relative; display: flex; align-items: center; gap: 0.75rem; }
.group-sort-menu__pop { position: fixed; z-index: 90; isolation: isolate; min-width: 10rem; }
.group-sort-menu__backdrop { position: absolute; inset: 0; pointer-events: none; clip-path: polygon(0 0, calc(100% - 10px) 0, 100% 10px, 100% 100%, 10px 100%, 0 calc(100% - 10px)); -webkit-mask-image: linear-gradient(#000, #000); mask-image: linear-gradient(#000, #000); -webkit-mask-repeat: no-repeat; mask-repeat: no-repeat; animation: ak-mask-h 320ms cubic-bezier(0.2, 0.8, 0.2, 1) both; }
.group-sort-menu__backdrop--gray { z-index: 0; background: #9ca3af; transform: translate(6px, 6px); animation-delay: 90ms; }
.group-sort-menu__backdrop--white { z-index: 1; background: #fff; }
.group-sort-menu__content { position: relative; z-index: 2; max-height: min(22rem, 50vh); overflow-y: auto; overscroll-behavior: contain; clip-path: polygon(0 0, calc(100% - 10px) 0, 100% 10px, 100% 100%, 10px 100%, 0 calc(100% - 10px)); -webkit-mask-image: linear-gradient(#000, #000); mask-image: linear-gradient(#000, #000); -webkit-mask-repeat: no-repeat; mask-repeat: no-repeat; animation: ak-mask-h 320ms cubic-bezier(0.2, 0.8, 0.2, 1) both; }
.group-sort-menu__item { display: flex; width: 100%; align-items: center; justify-content: space-between; gap: 0.75rem; padding: 0.6rem 0.75rem; color: #000; font-size: 0.8125rem; font-weight: 600; text-align: left; text-transform: uppercase; letter-spacing: 0.1em; transition: background-color 180ms cubic-bezier(0.2, 0.8, 0.2, 1); }
.group-sort-menu__item:hover, .group-sort-menu__item--selected { background: #d1d5db; color: #000; }
.group-sort-menu__item:first-child { clip-path: polygon(0 0, calc(100% - 10px) 0, 100% 10px, 100% 100%, 0 100%); }
.group-sort-menu__item:last-child { clip-path: polygon(0 0, 100% 0, 100% 100%, 10px 100%, 0 calc(100% - 10px)); }
.group-sort-menu__label { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.group-sort-menu__check { flex: none; }
.group-sort-menu-pop-enter-active, .group-sort-menu-pop-leave-active { transition: opacity 180ms ease, transform 180ms cubic-bezier(0.2, 0.8, 0.2, 1); }
.group-sort-menu-pop-enter-from, .group-sort-menu-pop-leave-to { opacity: 0; transform: translate(-5px, -5px); }
</style>
