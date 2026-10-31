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
.m3-input {
  position: relative;
  padding: 8px 0 0;

  &.disabled { opacity: 0.38; }
}

.input-label {
  display: block;
  font-size: 12px;
  font-weight: 500;
  color: var(--md-on-surface-variant);
  margin-bottom: 4px;
  transition: color 200ms;

  .focused & { color: var(--md-primary); }
}

input {
  display: block;
  width: 100%;
  height: 48px;
  padding: 0 16px;
  font-size: 16px;
  font-family: inherit;
  color: var(--md-on-surface);
  background: var(--md-surface-container);
  border: none;
  border-radius: 12px 12px 0 0;
  outline: none;
  caret-color: var(--md-primary);

  &::placeholder {
    color: var(--md-on-surface-variant);
    opacity: 0.6;
  }
}

.input-underline {
  height: 2px;
  background: var(--md-on-surface-variant);
  opacity: 0.5;
  border-radius: 0 0 1px 1px;
  transition: background 200ms, opacity 200ms, height 200ms;

  .focused & {
    height: 2px;
    background: var(--md-primary);
    opacity: 1;
  }
}
</style>
