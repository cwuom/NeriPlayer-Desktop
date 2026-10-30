<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useAuthStore } from '@/stores/auth'
import M3Dialog from '@/components/ui/M3Dialog.vue'

const props = defineProps<{
  open: boolean
  platform: 'netease' | 'bilibili' | 'youtube'
}>()
const emit = defineEmits<{ 'update:open': [value: boolean] }>()

const { t } = useI18n()
const auth = useAuthStore()
const rawCookies = ref('')
const submitting = ref(false)
let dialogGeneration = 0

const platformName = computed(() => t(`settings.${props.platform}_account`))
const hint = computed(() => t(`settings.cookie_login_${props.platform}_hint`))
const requiredCookies = computed(() => ({
  netease: ['MUSIC_U'],
  bilibili: ['SESSDATA'],
  youtube: ['SAPISID', '__Secure-1PAPISID', '__Secure-3PAPISID'],
}[props.platform]))
const confirmDisabled = computed(() => submitting.value || !rawCookies.value.trim())

watch(() => [props.open, props.platform] as const, ([open, platform], [, previousPlatform]) => {
  if (!open || platform !== previousPlatform) {
    rawCookies.value = ''
    dialogGeneration++
  }
}, { flush: 'sync' })

function updateOpen(open: boolean) {
  if (submitting.value) return
  if (!open) rawCookies.value = ''
  emit('update:open', open)
}

async function submit() {
  if (!props.open || confirmDisabled.value) return
  const generation = dialogGeneration
  const platform = props.platform
  submitting.value = true
  try {
    const accepted = await auth.loginWithCookies(platform, rawCookies.value)
    if (accepted && generation === dialogGeneration && props.open && props.platform === platform) {
      rawCookies.value = ''
      emit('update:open', false)
    }
  } catch {
    // 登录错误由账号 store 提示，保留输入以便修正后重试
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <M3Dialog
    :open="open"
    :title="t('settings.cookie_login')"
    icon="cookie"
    :confirm-text="t(submitting ? 'settings.signing_in' : 'settings.cookie_login_submit')"
    :confirm-disabled="confirmDisabled"
    @update:open="updateOpen"
    @confirm="submit"
  >
    <div class="cookie-login-content" :aria-busy="submitting">
      <div class="cookie-platform">
        <span class="cookie-platform-icon" :style="{ maskImage: `url(/icons/ic_${platform}.svg)` }" aria-hidden="true"></span>
        <span class="cookie-platform-name">{{ platformName }}</span>
      </div>
      <p class="cookie-description">{{ t('settings.cookie_login_desc') }}</p>
      <div class="cookie-format-card">
        <div class="cookie-format-heading">
          <span class="material-symbols-rounded" aria-hidden="true">info</span>
          <span>{{ t('settings.cookie_login_format') }}</span>
        </div>
        <div class="cookie-required-keys">
          <code v-for="name in requiredCookies" :key="name">{{ name }}</code>
        </div>
        <p class="cookie-login-hint">{{ hint }}</p>
      </div>
      <label class="cookie-field">
        <span class="cookie-field-label">{{ t('settings.cookie_login_content') }}</span>
        <span class="cookie-field-surface">
          <textarea
            v-model="rawCookies"
            :aria-label="t('settings.cookie_login_content')"
            :placeholder="t('settings.cookie_login_placeholder')"
            :disabled="submitting"
            rows="6"
            autocomplete="off"
            autocapitalize="off"
            autocorrect="off"
            spellcheck="false"
          />
          <span class="cookie-field-underline" />
        </span>
      </label>
      <p class="cookie-private-note">
        <span class="material-symbols-rounded" aria-hidden="true">lock</span>
        <span>{{ t('settings.cookie_login_private') }}</span>
      </p>
    </div>
  </M3Dialog>
</template>

<style scoped lang="scss">
.cookie-login-content {
  display: flex;
  flex-direction: column;
  gap: 16px;
  min-width: 0;
  white-space: normal;

  p { margin: 0; }
}

.cookie-platform {
  align-self: center;
  display: inline-flex;
  align-items: center;
  gap: 8px;
  max-width: 100%;
  padding: 8px 12px;
  border-radius: var(--radius-full);
  background: var(--md-surface-container);
}

.cookie-platform-icon {
  flex-shrink: 0;
  width: 18px;
  height: 18px;
  background: var(--md-primary);
  mask-size: contain;
  mask-position: center;
  mask-repeat: no-repeat;
}

.cookie-platform-name { font-size: 12px; font-weight: 600; color: var(--md-on-surface); overflow-wrap: anywhere; }
.cookie-description { font-size: 13px; line-height: 1.6; text-align: center; }

.cookie-format-card {
  display: grid;
  gap: 10px;
  padding: 14px 16px;
  border-radius: var(--radius-lg);
  background: color-mix(in srgb, var(--md-primary) 8%, var(--md-surface-container));
}

.cookie-format-heading {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  font-weight: 600;
  color: var(--md-primary);

  .material-symbols-rounded { font-size: 16px; }
}

.cookie-required-keys {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;

  code {
    max-width: 100%;
    padding: 3px 7px;
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--md-primary) 10%, transparent);
    color: var(--md-on-surface);
    font-family: inherit;
    font-size: 11px;
    line-height: 1.5;
    overflow-wrap: anywhere;
  }
}

.cookie-login-hint { font-size: 12px; line-height: 1.6; color: var(--md-on-surface-variant); }
.cookie-field { display: grid; gap: 6px; min-width: 0; }
.cookie-field-label { font-size: 12px; font-weight: 500; transition: color var(--duration-short); }
.cookie-field-surface { display: block; overflow: hidden; border-radius: var(--radius-md) var(--radius-md) 0 0; }

textarea {
  display: block;
  box-sizing: border-box;
  width: 100%;
  min-height: 144px;
  padding: 12px 16px;
  resize: none;
  appearance: none;
  border: none;
  outline: none;
  background: var(--md-surface-container);
  color: var(--md-on-surface);
  font-family: var(--font-family);
  font-size: 13px;
  line-height: 1.6;
  caret-color: var(--md-primary);

  &::placeholder { color: var(--md-on-surface-variant); opacity: 0.6; }
  &:disabled { opacity: 0.38; }
}

.cookie-field-underline {
  display: block;
  height: 2px;
  background: var(--md-on-surface-variant);
  opacity: 0.5;
  transition: background var(--duration-short), opacity var(--duration-short);
}

.cookie-field:focus-within {
  .cookie-field-label { color: var(--md-primary); }
  .cookie-field-underline { background: var(--md-primary); opacity: 1; }
}

.cookie-private-note {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--md-on-surface-variant);

  .material-symbols-rounded { font-size: 15px; }
}
</style>
