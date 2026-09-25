<script setup lang="ts" generic="T extends string">
import { onBeforeUnmount, ref } from "vue";
import { ChevronDown } from "@lucide/vue";

defineProps<{ options: { value: T; label: string }[]; label: string }>();
const model = defineModel<T>({ required: true });
const open = ref(false);
const root = ref<HTMLElement>();

function close(event?: FocusEvent) {
  if (!event || !root.value?.contains(event.relatedTarget as Node | null)) open.value = false;
}

onBeforeUnmount(() => { open.value = false; });
</script>

<template>
  <div ref="root" class="layered-select" @focusout="close">
    <button type="button" class="layered-select__trigger" :aria-expanded="open" :aria-label="label" @click="open = !open">
      <span>{{ options.find((option) => option.value === model)?.label }}</span>
      <ChevronDown class="layered-select__chevron" :class="open ? 'rotate-180' : ''" :size="16" :stroke-width="1.8" />
    </button>
    <Transition name="layered-menu">
      <div v-if="open" class="layered-select__menu" role="listbox" :aria-label="label">
        <span class="layered-select__backdrop layered-select__backdrop--gray" aria-hidden="true"></span>
        <span class="layered-select__backdrop layered-select__backdrop--white" aria-hidden="true"></span>
        <div class="layered-select__content">
          <button v-for="option in options" :key="option.value" type="button" class="layered-select__option" :class="model === option.value ? 'layered-select__option--selected' : ''" :aria-selected="model === option.value" @click="model = option.value; open = false">
            {{ option.label }}
          </button>
        </div>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.layered-select { position: relative; }
.layered-select__trigger { display: flex; height: 2.5rem; width: 100%; align-items: center; justify-content: space-between; border: 1px solid var(--line); background: var(--bg); padding: 0 0.75rem; color: var(--fg); font-size: 0.875rem; text-align: left; }
.layered-select__trigger:hover { border-color: var(--accent); }
.layered-select__chevron { transition: transform 240ms cubic-bezier(0.2, 0.8, 0.2, 1); }
.layered-select__menu { position: absolute; z-index: 30; top: calc(100% + 0.5rem); width: 100%; isolation: isolate; }
.layered-select__backdrop { position: absolute; inset: 0; pointer-events: none; clip-path: polygon(0 0, calc(100% - 10px) 0, 100% 10px, 100% 100%, 10px 100%, 0 calc(100% - 10px)); -webkit-mask-image: linear-gradient(#000, #000); mask-image: linear-gradient(#000, #000); -webkit-mask-repeat: no-repeat; mask-repeat: no-repeat; animation: ak-mask-h 320ms cubic-bezier(0.2, 0.8, 0.2, 1) both; }
.layered-select__backdrop--gray { z-index: 0; background: #9ca3af; transform: translate(6px, 6px); animation-delay: 90ms; }
.layered-select__backdrop--white { z-index: 1; background: #fff; }
.layered-select__content { position: relative; z-index: 2; -webkit-mask-image: linear-gradient(#000, #000); mask-image: linear-gradient(#000, #000); -webkit-mask-repeat: no-repeat; mask-repeat: no-repeat; animation: ak-mask-h 320ms cubic-bezier(0.2, 0.8, 0.2, 1) both; }
.layered-select__option { display: block; width: 100%; padding: 0.6rem 0.75rem; color: #000; font-size: 0.8125rem; font-weight: 600; text-align: left; text-transform: uppercase; letter-spacing: 0.1em; transition: background-color 180ms cubic-bezier(0.2, 0.8, 0.2, 1); }
.layered-select__option:hover, .layered-select__option--selected { background: #d1d5db; color: #000; }
.layered-select__option:first-child { clip-path: polygon(0 0, calc(100% - 10px) 0, 100% 10px, 100% 100%, 0 100%); }
.layered-select__option:last-child { clip-path: polygon(0 0, 100% 0, 100% 100%, 10px 100%, 0 calc(100% - 10px)); }
.layered-menu-enter-active, .layered-menu-leave-active { transition: opacity 180ms ease, transform 180ms cubic-bezier(0.2, 0.8, 0.2, 1); }
.layered-menu-enter-from, .layered-menu-leave-to { opacity: 0; transform: translate(-5px, -5px); }
</style>
