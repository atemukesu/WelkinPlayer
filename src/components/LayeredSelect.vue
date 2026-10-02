<!--
 Copyright 2026 Atemukesu
 SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts" generic="T extends string">
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { ChevronDown } from "@lucide/vue";

defineProps<{ options: { value: T; label: string }[]; label: string }>();
const model = defineModel<T>({ required: true });
const open = ref(false);
const root = ref<HTMLElement>();
const menu = ref<HTMLElement>();
const position = ref({ x: 0, y: 0 });

// Keep the menu fully inside the viewport, measuring its real rendered size.
function clampToViewport() {
  const rootEl = root.value;
  const menuEl = menu.value;
  if (!rootEl || !menuEl) return;
  // Extra margin covers the offset shadow and angled corners.
  const margin = 8;
  const gap = 8;
  const rect = rootEl.getBoundingClientRect();
  const width = Math.min(rect.width, window.innerWidth - margin * 2);
  // Apply the width before measuring so long labels wrap like they will on screen.
  menuEl.style.width = `${width}px`;
  const height = menuEl.offsetHeight;

  let x = rect.left;
  if (x + width + margin > window.innerWidth) x = window.innerWidth - width - margin;
  if (x < margin) x = margin;

  let y = rect.bottom + gap;
  if (y + height + margin > window.innerHeight) {
    const above = rect.top - gap - height;
    // Prefer flipping above the trigger; otherwise pin to the bottom edge.
    y = above >= margin ? above : Math.max(margin, window.innerHeight - height - margin);
  }
  position.value = { x, y };
}

function scheduleClamp() {
  void nextTick(clampToViewport);
}

function close(event?: FocusEvent) {
  if (!event) {
    open.value = false;
    return;
  }
  const next = event.relatedTarget as Node | null;
  if (!root.value?.contains(next) && !menu.value?.contains(next)) open.value = false;
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
  <div ref="root" class="layered-select" @focusout="close">
    <button type="button" class="layered-select__trigger" :aria-expanded="open" :aria-label="label" @click="open = !open">
      <span class="layered-select__label">{{ options.find((option) => option.value === model)?.label }}</span>
      <ChevronDown class="layered-select__chevron" :class="open ? 'rotate-180' : ''" :size="16" :stroke-width="1.8" />
    </button>
    <Teleport to="body">
      <Transition name="layered-menu">
        <div v-if="open" ref="menu" class="layered-select__menu" role="listbox" :aria-label="label" :style="{ left: `${position.x}px`, top: `${position.y}px` }">
          <span class="layered-select__backdrop layered-select__backdrop--gray" aria-hidden="true"></span>
          <span class="layered-select__backdrop layered-select__backdrop--white" aria-hidden="true"></span>
          <div class="layered-select__content">
            <button v-for="option in options" :key="option.value" type="button" class="layered-select__option" :class="model === option.value ? 'layered-select__option--selected' : ''" :aria-selected="model === option.value" @click="model = option.value; open = false">
              {{ option.label }}
            </button>
          </div>
        </div>
      </Transition>
    </Teleport>
  </div>
</template>

<style scoped>
.layered-select { position: relative; min-width: 0; }
.layered-select__trigger { display: flex; height: 2.5rem; width: 100%; align-items: center; gap: 0.5rem; justify-content: space-between; border: 1px solid var(--line); background: var(--bg); padding: 0 0.75rem; color: var(--fg); font-size: 0.875rem; text-align: left; }
.layered-select__trigger:hover { border-color: var(--accent); }
.layered-select__label { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.layered-select__chevron { flex: none; transition: transform 240ms cubic-bezier(0.2, 0.8, 0.2, 1); }
.layered-select__menu { position: fixed; z-index: 90; isolation: isolate; }
.layered-select__backdrop { position: absolute; inset: 0; pointer-events: none; clip-path: polygon(0 0, calc(100% - 10px) 0, 100% 10px, 100% 100%, 10px 100%, 0 calc(100% - 10px)); -webkit-mask-image: linear-gradient(#000, #000); mask-image: linear-gradient(#000, #000); -webkit-mask-repeat: no-repeat; mask-repeat: no-repeat; animation: ak-mask-h 320ms cubic-bezier(0.2, 0.8, 0.2, 1) both; }
.layered-select__backdrop--gray { z-index: 0; background: #9ca3af; transform: translate(6px, 6px); animation-delay: 90ms; }
.layered-select__backdrop--white { z-index: 1; background: #fff; }
.layered-select__content { position: relative; z-index: 2; max-height: min(22rem, 50vh); overflow-y: auto; overscroll-behavior: contain; clip-path: polygon(0 0, calc(100% - 10px) 0, 100% 10px, 100% 100%, 10px 100%, 0 calc(100% - 10px)); -webkit-mask-image: linear-gradient(#000, #000); mask-image: linear-gradient(#000, #000); -webkit-mask-repeat: no-repeat; mask-repeat: no-repeat; animation: ak-mask-h 320ms cubic-bezier(0.2, 0.8, 0.2, 1) both; }
.layered-select__content::-webkit-scrollbar { width: 10px; }
.layered-select__content::-webkit-scrollbar-track { background: #fff; }
.layered-select__content::-webkit-scrollbar-thumb { background: #d1d5db; }
.layered-select__content::-webkit-scrollbar-thumb:hover { background: #9ca3af; }
.layered-select__option { display: block; width: 100%; padding: 0.6rem 0.75rem; color: #000; font-size: 0.8125rem; font-weight: 600; text-align: left; text-transform: uppercase; letter-spacing: 0.1em; overflow-wrap: anywhere; transition: background-color 180ms cubic-bezier(0.2, 0.8, 0.2, 1); }
.layered-select__option:hover, .layered-select__option--selected { background: #d1d5db; color: #000; }
.layered-select__option:first-child { clip-path: polygon(0 0, calc(100% - 10px) 0, 100% 10px, 100% 100%, 0 100%); }
.layered-select__option:last-child { clip-path: polygon(0 0, 100% 0, 100% 100%, 10px 100%, 0 calc(100% - 10px)); }
.layered-menu-enter-active, .layered-menu-leave-active { transition: opacity 180ms ease, transform 180ms cubic-bezier(0.2, 0.8, 0.2, 1); }
.layered-menu-enter-from, .layered-menu-leave-to { opacity: 0; transform: translate(-5px, -5px); }
</style>
