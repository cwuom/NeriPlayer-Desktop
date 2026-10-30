<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import M3Dialog from '@/components/ui/M3Dialog.vue'
import M3Input from '@/components/ui/M3Input.vue'
import { useSyncStore } from '@/stores/sync'

const props = defineProps<{ mode: 'export' | 'import' | null }>()
const emit = defineEmits<{ 'update:mode': [value: null]; imported: [] }>()
const { t } = useI18n()
const sync = useSyncStore()
const encrypted = ref(false)
const includeLoginState = ref(false)
const password = ref('')
const passwordConfirmation = ref('')
const filePath = ref('')
const stage = ref<'select' | 'password' | 'confirm'>('select')
const busy = ref(false)
const error = ref('')
let revision = 0

const exporting = computed(() => props.mode === 'export')
const passwordValid = computed(() => password.value.length > 0 && password.value.length <= 1024)
const confirmDisabled = computed(() => busy.value || (exporting.value
  ? encrypted.value && (!passwordValid.value || password.value !== passwordConfirmation.value)
  : stage.value === 'password' && !passwordValid.value))
const title = computed(() => exporting.value ? t('settings.export_config')
  : stage.value === 'password' ? t('settings.config_password_required') : t('settings.config_android_warning_title'))

function close() {
  if (busy.value) return
  password.value = ''
  passwordConfirmation.value = ''
  emit('update:mode', null)
}

watch(() => props.mode, mode => {
  revision += 1
  password.value = ''
  passwordConfirmation.value = ''
  filePath.value = ''
  error.value = ''
  stage.value = 'select'
  encrypted.value = false
  includeLoginState.value = false
  busy.value = false
  if (mode === 'import') void runImport()
}, { immediate: true })

async function runImport() {
  if (busy.value) return
  const current = revision
  busy.value = true
  error.value = ''
  try {
    const result = await sync.importConfig({
      filePath: filePath.value || undefined,
      password: password.value || undefined,
      confirmedAndroid: stage.value === 'confirm',
    })
    if (current !== revision) return
    busy.value = false
    if (result?.success) {
      emit('imported')
      close()
    } else if (result?.requiresPassword) {
      filePath.value = result.filePath
      stage.value = 'password'
    } else if (result?.requiresConfirmation) {
      filePath.value = result.filePath
      stage.value = 'confirm'
    } else if (result?.error && stage.value !== 'select') {
      error.value = result.error
    } else {
      close()
    }
  } finally {
    if (current === revision) busy.value = false
  }
}

async function confirm() {
  if (!props.mode || confirmDisabled.value) return
  if (!exporting.value) {
    await runImport()
    return
  }
  busy.value = true
  try {
    const result = await sync.exportConfig({
      password: encrypted.value ? password.value : undefined,
      includeLoginState: includeLoginState.value,
    })
    busy.value = false
    if (result?.success || !result?.error) close()
    else error.value = result.error
  } finally { busy.value = false }
}
</script>

<template>
  <M3Dialog
    :open="mode !== null && (exporting || stage !== 'select')"
    :title="title"
    :icon="exporting ? 'lock' : stage === 'confirm' ? 'warning' : 'key'"
    :confirm-text="busy ? t('common.loading') : t('common.confirm')"
    :confirm-disabled="confirmDisabled"
    @update:open="close"
    @confirm="confirm"
  >
    <div class="config-transfer">
      <template v-if="exporting">
        <label class="config-option">
          <span class="config-option-copy">
            <span class="config-option-title">{{ t('settings.config_encrypt') }}</span>
            <span class="config-option-description">{{ t('settings.config_encrypt_desc') }}</span>
          </span>
          <span class="m3-switch">
            <input type="checkbox" role="switch" :aria-label="t('settings.config_encrypt')" :checked="encrypted" :disabled="busy" @change="encrypted = ($event.target as HTMLInputElement).checked" />
            <span class="track"><span class="thumb"><span v-if="encrypted" class="material-symbols-rounded">check</span></span></span>
          </span>
        </label>
        <label class="config-option">
          <span class="config-option-copy">
            <span class="config-option-title">{{ t('settings.config_include_login') }}</span>
            <span class="config-option-description">{{ t('settings.config_include_login_desc') }}</span>
          </span>
          <span class="m3-switch">
            <input type="checkbox" role="switch" :aria-label="t('settings.config_include_login')" :checked="includeLoginState" :disabled="busy" @change="includeLoginState = ($event.target as HTMLInputElement).checked" />
            <span class="track"><span class="thumb"><span v-if="includeLoginState" class="material-symbols-rounded">check</span></span></span>
          </span>
        </label>
        <div v-if="includeLoginState" class="config-notice">
          <span class="material-symbols-rounded" aria-hidden="true">privacy_tip</span>
          <p>{{ t('settings.export_config_warning_desc') }}</p>
        </div>
      </template>
      <p v-else-if="stage === 'confirm'" class="config-warning">{{ t('settings.config_android_warning_desc') }}</p>
      <p v-else>{{ t('settings.config_password_required_desc') }}</p>
      <template v-if="exporting ? encrypted : stage === 'password'">
        <M3Input
          v-model="password" type="password" :label="t('settings.config_password')"
          :autocomplete="exporting ? 'new-password' : 'off'" :disabled="busy" :maxlength="1024"
          @enter="confirm"
        />
        <M3Input
          v-if="exporting" v-model="passwordConfirmation" type="password" :label="t('settings.config_password_confirm')"
          autocomplete="new-password" :disabled="busy" :maxlength="1024"
          @enter="confirm"
        />
        <p v-if="exporting && passwordConfirmation && password !== passwordConfirmation" role="alert" class="config-warning">{{ t('settings.config_password_mismatch') }}</p>
      </template>
      <p v-if="error" role="alert" class="config-warning">{{ error }}</p>
    </div>
  </M3Dialog>
</template>

<style scoped lang="scss">
.config-transfer {
  display: grid;
  gap: 12px;
  white-space: normal;
}

.config-transfer p { margin: 0; font-size: 13px; line-height: 1.6; }

.config-option {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 16px;
  border-radius: var(--radius-lg);
  background: var(--md-surface-container);
  cursor: pointer;
  transition: background var(--duration-short) var(--ease-standard);

  &:hover { background: var(--md-surface-container-highest); }
  &:has(input:disabled) { cursor: default; }
}

.config-option-copy { display: grid; gap: 4px; min-width: 0; }
.config-option-title { font-size: 14px; font-weight: 600; color: var(--md-on-surface); }
.config-option-description { font-size: 12px; line-height: 1.5; color: var(--md-on-surface-variant); }

.config-notice {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 14px 16px;
  border-radius: var(--radius-lg);
  background: color-mix(in srgb, var(--md-error-container) 24%, var(--md-surface-container));
  color: var(--md-error);

  > .material-symbols-rounded { flex-shrink: 0; font-size: 20px; margin-top: 1px; }
}
.config-warning { color: var(--md-error); }
</style>
