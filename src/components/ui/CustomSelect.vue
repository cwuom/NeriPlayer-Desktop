<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted, nextTick, useId } from 'vue'

export interface SelectOption {
  value: string
  label: string
}

const props = withDefaults(defineProps<{
  modelValue: string
  options: SelectOption[]
  surface?: 'default' | 'dark'
  id?: string
  label?: string
  disabled?: boolean
}>(), {
  surface: 'default',
  disabled: false,
})

const emit = defineEmits<{
  'update:modelValue': [value: string]
  open: []
}>()

const isOpen = ref(false)
const triggerRef = ref<HTMLElement | null>(null)
const menuRef = ref<HTMLElement | null>(null)
const menuStyle = ref<Record<string, string>>({})
const menuId = `select-${useId()}`
const activeIndex = ref(-1)

const selectedLabel = computed(() => {
  const opt = props.options.find(o => o.value === props.modelValue)
  return opt?.label ?? props.modelValue
})
const isDarkSurface = computed(() => props.surface === 'dark')

function toggle() {
  if (isOpen.value) {
    isOpen.value = false
    return
  }
  openMenu()
}

function openMenu(index = props.options.findIndex(option => option.value === props.modelValue)) {
  if (props.disabled || !props.options.length) return
  activeIndex.value = Math.max(0, index)
  updatePosition()
  isOpen.value = true
  emit('open')
}

function close(restoreFocus = false) {
  isOpen.value = false
  if (restoreFocus) triggerRef.value?.focus({ preventScroll: true })
}

function select(value: string) {
  if (props.disabled || !props.options.some(option => option.value === value)) return
  emit('update:modelValue', value)
  close(true)
}

function handleKeydown(event: KeyboardEvent) {
  if (props.disabled) return
  // 聚焦选择器时让播放快捷键让位，Tab 和未展开时的 Escape 继续交给外层
  if (event.key !== 'Tab' && (event.key !== 'Escape' || isOpen.value)) event.stopPropagation()
  switch (event.key) {
    case 'ArrowDown':
    case 'ArrowUp':
      event.preventDefault()
      if (!isOpen.value) openMenu()
      else activeIndex.value = (activeIndex.value + (event.key === 'ArrowDown' ? 1 : -1) + props.options.length) % props.options.length
      break
    case 'Home':
    case 'End': {
      event.preventDefault()
      const index = event.key === 'Home' ? 0 : props.options.length - 1
      if (!isOpen.value) openMenu(index)
      else activeIndex.value = index
      break
    }
    case 'Enter':
    case ' ':
      event.preventDefault()
      if (!isOpen.value) openMenu()
      else {
        const option = props.options[activeIndex.value]
        if (option) select(option.value)
      }
      break
    case 'Escape':
      if (isOpen.value) {
        event.preventDefault()
        close(true)
      }
      break
    case 'Tab':
      close()
      break
  }
}

function updatePosition() {
  if (!triggerRef.value) return
  const rect = triggerRef.value.getBoundingClientRect()
  const spaceBelow = window.innerHeight - rect.bottom - 12
  const spaceAbove = rect.top - 12
  const menuHeight = Math.min(props.options.length * 40 + 8, 280)
  const width = Math.min(Math.max(rect.width, 200), Math.max(0, window.innerWidth - 16))
  const position = {
    left: `${Math.max(8, Math.min(rect.left, window.innerWidth - width - 8))}px`,
    width: `${width}px`,
  }

  // 底部空间不足时向上弹出
  if (spaceBelow < menuHeight && spaceAbove > spaceBelow) {
    menuStyle.value = {
      ...position,
      bottom: `${window.innerHeight - rect.top + 4}px`,
      maxHeight: `${Math.max(0, Math.min(spaceAbove, 280))}px`,
      transformOrigin: 'bottom center',
    }
  } else {
    menuStyle.value = {
      ...position,
      top: `${rect.bottom + 4}px`,
      maxHeight: `${Math.max(0, Math.min(spaceBelow, 280))}px`,
      transformOrigin: 'top center',
    }
  }
}

function handleClickOutside(e: MouseEvent) {
  if (!isOpen.value) return
  const target = e.target as HTMLElement
  if (triggerRef.value?.contains(target)) return
  if (menuRef.value?.contains(target)) return
  close()
}

function handleScroll(event: Event) {
  if (isOpen.value && !menuRef.value?.contains(event.target as Node)) close()
}

function handleResize() {
  if (isOpen.value) updatePosition()
}

async function revealActiveOption() {
  await nextTick()
  if (isOpen.value) menuRef.value?.querySelector(`[data-option-index="${activeIndex.value}"]`)?.scrollIntoView({ block: 'nearest' })
}

onMounted(() => {
  document.addEventListener('pointerdown', handleClickOutside, true)
  window.addEventListener('scroll', handleScroll, true)
  window.addEventListener('resize', handleResize)
})

onUnmounted(() => {
  document.removeEventListener('pointerdown', handleClickOutside, true)
  window.removeEventListener('scroll', handleScroll, true)
  window.removeEventListener('resize', handleResize)
})

watch(isOpen, async (open) => {
  if (open) {
    await nextTick()
    if (!isOpen.value) return
    updatePosition()
    void revealActiveOption()
  }
})
watch(activeIndex, revealActiveOption)
watch(() => props.disabled, disabled => { if (disabled) close() })
watch(() => props.modelValue, value => {
  if (isOpen.value) activeIndex.value = props.options.findIndex(option => option.value === value)
})
watch(() => props.options.map(option => option.value), (values, previous) => {
  if (!isOpen.value) return
  if (!values.length) { close(); return }
  const index = values.indexOf(previous[activeIndex.value])
  activeIndex.value = Math.max(0, index)
  void nextTick(() => { if (isOpen.value) updatePosition() })
})
</script>

<template>
  <div class="custom-select" :class="{ 'custom-select--dark': isDarkSurface }">
    <button ref="triggerRef" :id="id" class="custom-select-trigger" @click="toggle" @keydown="handleKeydown" @blur="close()" type="button"
      role="combobox" aria-haspopup="listbox" :aria-label="label" :aria-expanded="isOpen" :aria-controls="menuId"
      :aria-activedescendant="isOpen && activeIndex >= 0 ? `${menuId}-${activeIndex}` : undefined" :disabled="disabled" :title="selectedLabel">
      <span class="custom-select-label">{{ selectedLabel }}</span>
      <span class="material-symbols-rounded custom-select-arrow" :class="{ open: isOpen }" aria-hidden="true">expand_more</span>
    </button>

    <Teleport to="body">
      <Transition name="cs-menu">
        <div
          v-if="isOpen"
          ref="menuRef"
          class="custom-select-menu"
          :class="{ 'custom-select-menu--dark': isDarkSurface }"
          :style="menuStyle"
          :id="menuId"
          role="listbox"
          :aria-label="label"
        >
          <button
            v-for="(opt, index) in options"
            :key="opt.value"
            class="custom-select-option"
            :class="{ active: opt.value === modelValue, highlighted: index === activeIndex }"
            :id="`${menuId}-${index}`"
            :data-option-index="index"
            role="option"
            :aria-selected="opt.value === modelValue"
            tabindex="-1"
            @click="select(opt.value)"
            @mousedown.prevent
            type="button"
          >
            <span class="custom-select-option-label">{{ opt.label }}</span>
            <span class="material-symbols-rounded custom-select-check" :class="{ selected: opt.value === modelValue }" aria-hidden="true">check</span>
          </button>
        </div>
      </Transition>
    </Teleport>
  </div>
</template>

<style scoped lang="scss">
.custom-select {
  position: relative;
  display: inline-flex;
  min-width: 0;
}

.custom-select-trigger {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 8px 12px;
  border-radius: var(--radius-md);
  background: var(--md-surface-container-high);
  color: var(--md-on-surface);
  font-size: 13px;
  font-weight: 500;
  font-family: inherit;
  cursor: pointer;
  border: 1px solid var(--md-outline-variant);
  transition: background 150ms, border-color 150ms;
  min-width: 0;
  width: 100%;

  &:hover {
    background: var(--md-surface-container-highest);
    border-color: var(--md-outline);
  }

  &:focus-visible {
    outline: 2px solid var(--md-primary);
    outline-offset: 2px;
  }

  &:disabled {
    opacity: 0.5;
    cursor: default;
  }
}

.custom-select-label {
  flex: 1;
  text-align: left;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.custom-select-arrow {
  font-size: 18px !important;
  flex-shrink: 0;
  opacity: 0.6;
  transition: transform 200ms ease;

  &.open {
    transform: rotate(180deg);
  }
}

.custom-select--dark {
  .custom-select-trigger {
    background: rgba(255, 255, 255, 0.08);
    color: rgba(255, 255, 255, 0.88);
    border-color: rgba(255, 255, 255, 0.1);

    &:hover {
      background: rgba(255, 255, 255, 0.11);
      border-color: rgba(255, 255, 255, 0.18);
    }
  }

  .custom-select-arrow {
    opacity: 0.68;
  }
}
</style>

<style lang="scss">
/* 全局样式（Teleport 到 body 的菜单） */
.custom-select-menu {
  box-sizing: border-box;
  position: fixed;
  z-index: 500;
  background: var(--md-surface-container-high);
  border: 1px solid var(--md-outline-variant);
  border-radius: var(--radius-md);
  padding: 4px;
  overflow-y: auto;
  overscroll-behavior: contain;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.28), 0 2px 8px rgba(0, 0, 0, 0.12);
  transform-origin: top center;
}

.custom-select-option {
  display: flex;
  align-items: center;
  gap: 12px;
  box-sizing: border-box;
  min-height: 40px;
  width: 100%;
  padding: 8px 14px;
  border: none;
  background: none;
  color: var(--md-on-surface);
  font-size: 13px;
  font-weight: 500;
  font-family: inherit;
  text-align: left;
  cursor: pointer;
  border-radius: var(--radius-sm);
  transition: background 120ms;

  &:hover, &.highlighted {
    background: var(--md-surface-container-highest);
  }

  &.active {
    color: var(--md-primary);
    background: color-mix(in srgb, var(--md-primary) 10%, transparent);
  }
}

.custom-select-option-label {
  flex: 1;
  min-width: 0;
  overflow-wrap: anywhere;
  line-height: 1.4;
}

.custom-select-check {
  flex-shrink: 0;
  font-size: 18px !important;
  visibility: hidden;

  &.selected { visibility: visible; }
}

.custom-select-menu--dark {
  background: rgba(22, 21, 27, 0.98);
  border-color: rgba(255, 255, 255, 0.1);
  box-shadow: 0 16px 42px rgba(0, 0, 0, 0.46), 0 0 0 1px rgba(255, 255, 255, 0.03) inset;
  backdrop-filter: blur(20px);

  .custom-select-option {
    color: rgba(255, 255, 255, 0.78);

    &:hover, &.highlighted {
      background: rgba(255, 255, 255, 0.1);
      color: rgba(255, 255, 255, 0.94);
    }

    &.active {
      color: rgba(185, 225, 255, 0.96);
      background: rgba(120, 190, 255, 0.16);
    }
  }
}

/* 入场/退场动画 */
.cs-menu-enter-active {
  transition: opacity 120ms ease, transform 120ms cubic-bezier(0.05, 0.7, 0.1, 1);
}
.cs-menu-leave-active {
  transition: opacity 80ms ease, transform 80ms ease;
}
.cs-menu-enter-from {
  opacity: 0;
  transform: scaleY(0.92);
}
.cs-menu-leave-to {
  opacity: 0;
  transform: scaleY(0.96);
}
</style>
