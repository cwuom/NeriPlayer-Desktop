<script setup lang="ts">
import { ref } from 'vue'

const model = defineModel<string>({ default: '' })
const props = withDefaults(defineProps<{
  label?: string
  placeholder?: string
  maxlength?: number
  type?: 'text' | 'password'
  disabled?: boolean
  autocomplete?: string
}>(), { type: 'text', disabled: false })

const isFocused = ref(false)
const inputRef = ref<HTMLInputElement>()
const emit = defineEmits<{ enter: [] }>()

function focus() {
  if (!props.disabled) inputRef.value?.focus()
}

defineExpose({ focus })
</script>

<template>
  <div class="m3-input" :class="{ focused: isFocused, filled: !!model, disabled }" @click="focus">
    <label v-if="label" class="input-label">{{ label }}</label>
    <input
      ref="inputRef"
      v-model="model"
      :type="type"
      :disabled="disabled"
      :autocomplete="autocomplete"
      :aria-label="label"
      :placeholder="placeholder"
      :maxlength="maxlength"
      @focus="isFocused = true"
      @blur="isFocused = false"
      @keydown.enter="!disabled && emit('enter')"
    />
    <div class="input-underline" />
  </div>
</template>

<style scoped lang="scss">
/* Clean 输入框：安静的墨色填充，聚焦时变白底并套强调色光环（替代 M3 下划线） */
.m3-input {
  position: relative;
  padding: 8px 0 0;

  &.disabled { opacity: 0.4; }
}

.input-label {
  display: block;
  font-size: var(--fs-12);
  font-weight: 600;
  color: var(--md-on-surface-variant);
  margin: 0 2px 6px;
  transition: color 200ms;

  .focused & { color: var(--md-on-surface); }
}

input {
  display: block;
  width: 100%;
  height: 42px;
  padding: 0 14px;
  font-size: var(--fs-14);
  font-family: inherit;
  color: var(--md-on-surface);
  background: var(--md-surface-container-high);
  border: 1px solid transparent;
  border-radius: var(--radius-sm);
  outline: none;
  caret-color: var(--md-primary);
  transition: background 150ms, border-color 150ms, box-shadow 150ms;

  &:hover { background: color-mix(in srgb, var(--md-on-surface) 4%, var(--md-surface-container-high)); }

  &::placeholder {
    color: var(--md-on-surface-variant);
    opacity: 0.7;
  }

  .focused & {
    background: var(--md-surface-container-lowest);
    border-color: var(--md-primary);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--md-primary) 18%, transparent);
  }
}

/* 下划线保留节点以兼容结构，Clean 下不再显示 */
.input-underline { display: none; }
</style>
