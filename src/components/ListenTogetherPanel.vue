<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { useListenTogetherStore } from '@/stores/listenTogether'
import { useI18n } from 'vue-i18n'

const lt = useListenTogetherStore()
const { t } = useI18n()

const joinRoomId = ref('')
const nowTick = ref(Date.now())
let nowTimer: ReturnType<typeof setInterval> | null = null
const CONTROLLER_GRACE_PERIOD_MS = 10 * 60 * 1000

const statusColor = computed(() => {
  switch (lt.connectionState) {
    case 'connected': return 'var(--md-primary)'
    case 'connecting': return 'var(--md-tertiary)'
    default: return 'var(--md-outline)'
  }
})

const statusText = computed(() => {
  switch (lt.connectionState) {
    case 'connected': return t('listen_together.connected')
    case 'connecting': return lt.roomId ? t('listen_together.reconnecting') : t('listen_together.connecting')
    default: return t('listen_together.disconnected')
  }
})

const roomStatusText = computed(() => {
  switch (lt.roomState?.roomStatus) {
    case 'controller_offline': return t('listen_together.room_status_controller_offline')
    case 'closed': return t('listen_together.room_status_closed')
    default: return t('listen_together.room_status_active')
  }
})

const roomStatusTone = computed(() => {
  switch (lt.roomState?.roomStatus) {
    case 'controller_offline': return 'warning'
    case 'closed': return 'danger'
    default: return 'success'
  }
})

const controllerOfflineHint = computed(() => {
  const offlineSince = lt.roomState?.controllerOfflineSince
  if (!offlineSince || lt.roomState?.roomStatus !== 'controller_offline') return ''

  const remainingMs = Math.max(0, offlineSince + CONTROLLER_GRACE_PERIOD_MS - nowTick.value)
  const remainingMinutes = Math.max(1, Math.ceil(remainingMs / 60_000))
  return t('listen_together.controller_offline_detail', { minutes: remainingMinutes })
})

const latestSyncLabel = computed(() => {
  const eventType = lt.lastSyncEventType
  switch (eventType) {
    case 'PLAY': return t('listen_together.sync_play')
    case 'PAUSE': return t('listen_together.sync_pause')
    case 'SEEK': return t('listen_together.sync_seek')
    case 'SET_TRACK': return t('listen_together.sync_track')
    case 'UPDATE_SETTINGS': return t('listen_together.sync_settings')
    case 'HEARTBEAT': return t('listen_together.sync_heartbeat')
    case 'ROOM_RESUMED':
    case 'RECONNECTED':
      return t('listen_together.sync_reconnected')
    case 'ROOM_SUSPENDED':
      return t('listen_together.sync_suspended')
    case 'INITIAL_STATE':
    case 'WELCOME':
    case 'STATE_SYNC':
      return t('listen_together.sync_state')
    default:
      return eventType || t('listen_together.sync_unknown')
  }
})

const latestSyncTime = computed(() => formatRelativeTime(lt.lastSyncAt))
const latestRoomUpdateTime = computed(() => formatRelativeTime(lt.roomState?.updatedAt))
const controllerHeartbeatTime = computed(() => formatRelativeTime(lt.roomState?.controllerHeartbeatAt))

const connectedBanner = computed(() => {
  if (lt.roomState?.roomStatus === 'controller_offline') {
    return {
      tone: 'warning',
      icon: 'wifi_off',
      title: t('listen_together.controller_offline'),
      desc: controllerOfflineHint.value,
    }
  }

  if (lt.lastSyncEventType === 'ROOM_RESUMED' || lt.lastSyncEventType === 'RECONNECTED') {
    return {
      tone: 'success',
      icon: 'sync',
      title: t('listen_together.sync_reconnected'),
      desc: lt.lastReconnectAt ? formatRelativeTime(lt.lastReconnectAt) : '',
    }
  }

  return null
})

function handleCreate() {
  lt.createRoom()
}

function handleJoin() {
  if (!joinRoomId.value.trim()) return
  lt.joinRoom(joinRoomId.value.trim())
}

function handleLeave() {
  lt.leaveRoom()
}

// 剪贴板检测
async function checkClipboard() {
  const invite = await lt.checkClipboardInvite()
  if (invite) {
    joinRoomId.value = invite.roomId
    if (invite.baseUrl) {
      lt.baseUrl = invite.baseUrl
    }
  }
}

function formatRelativeTime(timestamp?: number | null) {
  if (!timestamp) return t('listen_together.not_available')

  const diff = Math.max(0, nowTick.value - timestamp)
  const minutes = Math.floor(diff / 60_000)
  const hours = Math.floor(diff / 3_600_000)
  const days = Math.floor(diff / 86_400_000)

  if (minutes < 1) return t('recent.just_now')
  if (hours < 1) return t('recent.minutes_ago', { count: minutes })
  if (days < 1) return t('recent.hours_ago', { count: hours })
  return t('recent.days_ago', { count: days })
}

onMounted(() => {
  nowTimer = setInterval(() => {
    nowTick.value = Date.now()
  }, 15_000)
  checkClipboard()
  window.addEventListener('focus', checkClipboard)
})

onUnmounted(() => {
  if (nowTimer) clearInterval(nowTimer)
  window.removeEventListener('focus', checkClipboard)
})
</script>

<template>
  <div class="lt-panel">
    <header class="lt-header">
      <div class="lt-title">
        <span class="material-symbols-rounded">group</span>
        <h3>{{ t('listen_together.title') }}</h3>
      </div>
      <div class="lt-status">
        <span class="lt-status-dot" :style="{ background: statusColor }" />
        <span class="lt-status-text">{{ statusText }}</span>
      </div>
      <div class="lt-header-spacer" />
      <button
        type="button"
        class="lt-close"
        :title="t('common.close')"
        :aria-label="t('common.close')"
        @click="$emit('close')"
      >
        <span class="material-symbols-rounded">close</span>
      </button>
    </header>

    <!-- 未连接状态 -->
    <div v-if="!lt.roomId && lt.connectionState !== 'connecting'" class="lt-body">
      <div class="lt-field">
        <label>{{ t('listen_together.nickname') }}</label>
        <input v-model="lt.nickname" type="text" class="lt-input" maxlength="20" />
      </div>

      <div class="lt-field">
        <label>{{ t('listen_together.room_id') }}</label>
        <input
          v-model="joinRoomId"
          type="text"
          class="lt-input"
          :placeholder="t('listen_together.room_id_placeholder')"
        />
      </div>

      <div class="lt-actions">
        <button type="button" class="lt-btn primary" data-lt-action="create-room" @click="handleCreate">
          <span class="material-symbols-rounded">add</span>
          {{ t('listen_together.create_room') }}
        </button>
        <button
          type="button"
          class="lt-btn"
          data-lt-action="join-room"
          :disabled="!joinRoomId.trim()"
          @click="handleJoin"
        >
          <span class="material-symbols-rounded">login</span>
          {{ t('listen_together.join_room') }}
        </button>
      </div>

      <div v-if="lt.sessionError" class="lt-error">{{ lt.sessionError }}</div>
    </div>

    <!-- 连接中 -->
    <div v-else-if="lt.connectionState !== 'connected'" class="lt-body lt-center">
      <span
        class="material-symbols-rounded"
        :class="{ spinning: lt.connectionState === 'connecting' }"
      >{{ lt.connectionState === 'connecting' ? 'progress_activity' : 'sync_problem' }}</span>
      <span>{{ lt.roomId ? t('listen_together.reconnecting') : t('listen_together.connecting') }}</span>
      <small class="lt-center-desc">
        {{ lt.roomId ? t('listen_together.reconnecting_desc') : t('listen_together.connecting_desc') }}
      </small>
      <button
        v-if="lt.roomId"
        type="button"
        class="lt-leave-button"
        data-lt-action="leave-room"
        @click="handleLeave"
      >
        <span class="material-symbols-rounded">logout</span>
        {{ t('listen_together.leave_room') }}
      </button>
    </div>

    <!-- 已连接状态 -->
    <div v-else class="lt-body lt-connected-body">
      <section class="lt-room-overview" data-lt-section="room-status">
        <div class="lt-room-info">
          <div class="lt-room-status-row">
            <div class="lt-room-status-title">
              <span class="material-symbols-rounded">sensors</span>
              <div>
                <span class="lt-label">{{ t('listen_together.room_status') }}</span>
                <strong>{{ roomStatusText }}</strong>
              </div>
            </div>
            <strong class="lt-status-chip" :class="roomStatusTone">{{ roomStatusText }}</strong>
          </div>
          <div class="lt-room-id">
            <div class="lt-room-code">
              <span class="lt-label">{{ t('listen_together.room_id') }}</span>
              <span class="lt-value">{{ lt.roomId }}</span>
            </div>
            <button
              type="button"
              class="lt-icon-btn"
              :title="t('listen_together.copy_invite')"
              :aria-label="t('listen_together.copy_invite')"
              data-lt-action="copy-invite" @click="lt.copyInviteLink()"
            >
              <span class="material-symbols-rounded">content_copy</span>
            </button>
            <span class="lt-role-badge" :class="lt.role || ''">
              {{ lt.isController ? t('listen_together.role_controller') : t('listen_together.role_listener') }}
            </span>
          </div>
        </div>

      <div v-if="connectedBanner" class="lt-banner" :class="connectedBanner.tone">
        <span class="material-symbols-rounded">{{ connectedBanner.icon }}</span>
        <div class="lt-banner-content">
          <strong>{{ connectedBanner.title }}</strong>
          <small v-if="connectedBanner.desc">{{ connectedBanner.desc }}</small>
        </div>
      </div>

        <details class="lt-diagnostics">
          <summary>
            <span class="material-symbols-rounded">sync</span>
            <span>{{ t('listen_together.sync_status') }}</span>
            <strong>{{ latestSyncLabel }}</strong>
            <span class="material-symbols-rounded lt-expand-icon">expand_more</span>
          </summary>
          <dl class="lt-diagnostics-list">
            <div>
              <dt>{{ t('listen_together.recent_sync') }}</dt>
              <dd>{{ latestSyncTime }}</dd>
            </div>
            <div>
              <dt>{{ t('listen_together.last_room_update') }}</dt>
              <dd>{{ latestRoomUpdateTime }}</dd>
            </div>
            <div>
              <dt>{{ t('listen_together.controller_heartbeat') }}</dt>
              <dd>{{ controllerHeartbeatTime }}</dd>
            </div>
            <div>
              <dt>{{ t('listen_together.room_version') }}</dt>
              <dd>#{{ lt.roomState?.version ?? 0 }}</dd>
            </div>
          </dl>
        </details>
      </section>

      <!-- 成员列表 -->
      <section class="lt-section lt-members-section" data-lt-section="members">
        <div class="lt-section-heading">
          <span class="material-symbols-rounded">groups</span>
          <h4>{{ t('listen_together.members') }}</h4>
          <span class="lt-count-badge">{{ lt.members.length }}</span>
        </div>
        <ul class="lt-member-list">
          <li v-for="m in lt.members" :key="m.userUuid" class="lt-member">
            <span class="material-symbols-rounded lt-member-avatar">
              {{ m.role === 'controller' ? 'shield' : 'person' }}
            </span>
            <span class="lt-member-name">{{ m.nickname || m.userUuid.slice(0, 8) }}</span>
            <span class="lt-member-role" :class="m.role">
              {{ m.role === 'controller' ? t('listen_together.role_controller') : t('listen_together.role_listener') }}
            </span>
          </li>
        </ul>
      </section>

      <!-- 房间设置（仅房主） -->
      <section data-lt-section="controls" v-if="lt.isController" class="lt-section lt-controls-section">
        <div class="lt-section-heading">
          <span class="material-symbols-rounded">tune</span>
          <h4>{{ t('listen_together.settings') }}</h4>
        </div>
        <label class="lt-setting-row">
          <span class="material-symbols-rounded lt-setting-icon">touch_app</span>
          <span class="lt-setting-label">{{ t('listen_together.allow_member_control') }}</span>
          <input
            type="checkbox"
            :checked="lt.roomSettings.allowMemberControl"
            @change="lt.updateRoomSettings({ allowMemberControl: ($event.target as HTMLInputElement).checked })"
          />
        </label>
        <label class="lt-setting-row">
          <span class="material-symbols-rounded lt-setting-icon">pause_circle</span>
          <span class="lt-setting-label">{{ t('listen_together.auto_pause_on_change') }}</span>
          <input
            type="checkbox"
            :checked="lt.roomSettings.autoPauseOnMemberChange"
            @change="lt.updateRoomSettings({ autoPauseOnMemberChange: ($event.target as HTMLInputElement).checked })"
          />
        </label>
        <label class="lt-setting-row">
          <span class="material-symbols-rounded lt-setting-icon">link</span>
          <span class="lt-setting-label">{{ t('listen_together.share_audio_links') }}</span>
          <input
            type="checkbox"
            :checked="lt.roomSettings.shareAudioLinks"
            @change="lt.updateRoomSettings({ shareAudioLinks: ($event.target as HTMLInputElement).checked })"
          />
        </label>
      </section>

      <button type="button" class="lt-leave-button" data-lt-action="leave-room" @click="handleLeave">
        <span class="material-symbols-rounded">logout</span>
        {{ t('listen_together.leave_room') }}
      </button>
    </div>
  </div>
</template>

<style scoped lang="scss">
.lt-panel {
  position: fixed;
  right: 12px;
  bottom: 88px;
  width: min(392px, calc(100vw - 24px));
  max-height: calc(100vh - 136px);
  background: var(--md-surface-container);
  border: 1px solid var(--md-outline-variant);
  border-radius: var(--radius-lg);
  box-shadow: 0 16px 44px rgba(0, 0, 0, 0.2);
  z-index: 200;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.lt-header {
  display: flex;
  align-items: center;
  min-height: 52px;
  padding: 10px 12px 10px 16px;
  gap: 8px;
  border-bottom: 1px solid var(--md-surface-container-highest);
}

.lt-title {
  display: flex;
  align-items: center;
  gap: 8px;

  h3 {
    font-size: 15px;
    font-weight: 600;
    margin: 0;
  }

  .material-symbols-rounded {
    color: var(--md-primary);
    font-size: 20px;
  }
}

.lt-status {
  display: flex;
  align-items: center;
  min-height: 24px;
  gap: 6px;
  padding: 0 8px;
  border-radius: var(--radius-full);
  background: var(--md-surface-container-high);
}

.lt-status-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
}

.lt-status-text {
  font-size: 11px;
  color: var(--md-on-surface-variant);
}

.lt-header-spacer {
  flex: 1;
}

.lt-close {
  width: 32px;
  height: 32px;
  border-radius: var(--radius-full);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--md-on-surface-variant);
  transition: background 150ms, color 150ms;
  &:hover {
    background: var(--md-surface-variant);
    color: var(--md-on-surface);
  }
  .material-symbols-rounded { font-size: 18px; }
}

.lt-body {
  flex: 1;
  overflow-y: auto;
  padding: 16px;
  display: flex;
  flex-direction: column;
  gap: 16px;
  scrollbar-width: thin;
}

.lt-connected-body {
  gap: 0;
}

.lt-center {
  align-items: center;
  justify-content: center;
  gap: 8px;
  color: var(--md-on-surface-variant);
}

.lt-center-desc {
  font-size: 12px;
  color: var(--md-on-surface-variant);
}

.lt-field {
  display: flex;
  flex-direction: column;
  gap: 4px;

  label {
    font-size: 12px;
    font-weight: 500;
    color: var(--md-on-surface-variant);
  }
}

.lt-input {
  padding: 8px 12px;
  border-radius: 8px;
  border: 1px solid var(--md-outline-variant);
  background: var(--md-surface-container-low);
  color: var(--md-on-surface);
  font-size: 13px;
  outline: none;
  transition: border-color 150ms;

  &:focus { border-color: var(--md-primary); }
}

.lt-actions {
  display: flex;
  gap: 8px;
}

.lt-btn {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 8px 12px;
  border-radius: 10px;
  font-size: 13px;
  font-weight: 500;
  background: var(--md-surface-container-high);
  color: var(--md-on-surface);
  transition: background 150ms, transform 100ms;

  .material-symbols-rounded { font-size: 18px; }

  &:hover { background: var(--md-surface-container-highest); }
  &:active { transform: scale(0.97); }
  &:disabled { opacity: 0.4; pointer-events: none; }

  &.primary {
    background: var(--md-primary);
    color: var(--md-on-primary);
    &:hover { opacity: 0.9; }
  }

  &.danger {
    background: var(--md-error-container);
    color: var(--md-on-error-container);
    &:hover { opacity: 0.9; }
  }
}

.lt-error {
  font-size: 12px;
  color: var(--md-error);
  padding: 8px;
  background: var(--md-error-container);
  border-radius: 8px;
}

.lt-room-info {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.lt-room-overview {
  padding-bottom: 16px;
  border-bottom: 1px solid var(--md-outline-variant);
}

.lt-room-status-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}

.lt-room-status-title {
  display: flex;
  align-items: center;
  min-width: 0;
  gap: 10px;

  > .material-symbols-rounded {
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    flex: 0 0 36px;
    border-radius: var(--radius-md);
    background: var(--md-primary-container);
    color: var(--md-on-primary-container);
    font-size: 21px;
  }

  > div {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 2px;
  }

  strong {
    font-size: 15px;
    line-height: 1.25;
  }
}

.lt-room-id {
  display: flex;
  align-items: center;
  gap: 8px;
}

.lt-label {
  font-size: 11px;
  color: var(--md-on-surface-variant);
}

.lt-room-code {
  display: flex;
  min-width: 0;
  flex: 1;
  flex-direction: column;
  gap: 2px;
}

.lt-value {
  overflow: hidden;
  font-size: 16px;
  font-weight: 650;
  font-family: monospace;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.lt-icon-btn {
  width: 32px;
  height: 32px;
  border-radius: var(--radius-full);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--md-on-surface-variant);
  transition: background 150ms, color 150ms;
  &:hover {
    background: var(--md-surface-variant);
    color: var(--md-on-surface);
  }
  .material-symbols-rounded { font-size: 16px; }
}

.lt-role-badge {
  display: inline-flex;
  align-items: center;
  flex: 0 0 auto;
  font-size: 11px;
  font-weight: 500;
  padding: 4px 9px;
  border-radius: var(--radius-full);
  background: var(--md-secondary-container);
  color: var(--md-on-secondary-container);

  &.controller {
    background: var(--md-primary-container);
    color: var(--md-on-primary-container);
  }
}

.lt-section {
  padding: 16px 0;
  border-bottom: 1px solid var(--md-outline-variant);

  h4 {
    font-size: 13px;
    font-weight: 600;
    margin: 0 0 6px;
  }
}

.lt-diagnostics {
  margin-top: 2px;
  border-top: 1px solid var(--md-outline-variant);

  summary {
    display: flex;
    align-items: center;
    min-height: 38px;
    gap: 8px;
    color: var(--md-on-surface-variant);
    cursor: pointer;
    list-style: none;

    &::-webkit-details-marker { display: none; }

    > .material-symbols-rounded:first-child { font-size: 17px; }

    > span:not(.material-symbols-rounded) { font-size: 12px; }

    strong {
      overflow: hidden;
      margin-left: auto;
      color: var(--md-on-surface);
      font-size: 12px;
      font-weight: 600;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
  }

  &[open] .lt-expand-icon { transform: rotate(180deg); }
}

.lt-expand-icon {
  font-size: 18px;
  transition: transform 160ms ease;
}

.lt-diagnostics-list {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 8px 16px;
  margin: 0;
  padding: 4px 0 2px 25px;

  div { min-width: 0; }

  dt,
  dd {
    overflow: hidden;
    margin: 0;
    font-size: 11px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  dt { color: var(--md-on-surface-variant); }
  dd { margin-top: 2px; color: var(--md-on-surface); font-weight: 550; }
}

.lt-section-heading {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 10px;

  > .material-symbols-rounded {
    color: var(--md-on-surface-variant);
    font-size: 18px;
  }

  h4 { margin: 0; }
}

.lt-count-badge {
  display: inline-grid;
  place-items: center;
  min-width: 22px;
  height: 20px;
  margin-left: auto;
  padding: 0 6px;
  border-radius: var(--radius-full);
  background: var(--md-surface-container-highest);
  color: var(--md-on-surface-variant);
  font-size: 11px;
  font-weight: 600;
}

.lt-banner {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  margin-top: 12px;
  padding: 10px 12px;
  border-radius: var(--radius-sm);

  .material-symbols-rounded {
    font-size: 18px;
    margin-top: 1px;
  }

  &.warning {
    background: color-mix(in srgb, var(--md-tertiary-container) 88%, transparent);
    color: var(--md-on-tertiary-container);
  }

  &.success {
    background: color-mix(in srgb, var(--md-primary-container) 88%, transparent);
    color: var(--md-on-primary-container);
  }
}

.lt-banner-content {
  display: flex;
  flex-direction: column;
  gap: 2px;

  strong {
    font-size: 13px;
  }

  small {
    font-size: 11px;
    opacity: 0.8;
  }
}

.lt-status-chip {
  display: inline-flex;
  align-items: center;
  flex: 0 0 auto;
  width: fit-content;
  min-height: 24px;
  padding: 0 9px;
  border-radius: var(--radius-full);
  font-size: 11px;
  font-weight: 600;

  &.success {
    background: var(--md-primary-container);
    color: var(--md-on-primary-container);
  }

  &.warning {
    background: var(--md-tertiary-container);
    color: var(--md-on-tertiary-container);
  }

  &.danger {
    background: var(--md-error-container);
    color: var(--md-on-error-container);
  }
}

.lt-member-list {
  display: flex;
  flex-direction: column;
  list-style: none;
  padding: 0;
  margin: 0;
}

.lt-member {
  display: flex;
  align-items: center;
  min-height: 42px;
  gap: 10px;
  padding: 4px 0;
  font-size: 13px;

  & + & { border-top: 1px solid var(--md-surface-container-highest); }
}

.lt-member-avatar {
  display: grid;
  place-items: center;
  width: 30px;
  height: 30px;
  flex: 0 0 30px;
  border-radius: var(--radius-full);
  background: var(--md-surface-container-highest);
  font-size: 17px;
  color: var(--md-on-surface-variant);
}

.lt-member-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  font-weight: 550;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.lt-member-role {
  flex: 0 0 auto;
  font-size: 11px;
  color: var(--md-on-surface-variant);

  &.controller { color: var(--md-primary); }
}

.lt-controls-section {
  display: flex;
  flex-direction: column;
  gap: 2px;

  .lt-section-heading { margin-bottom: 6px; }
}

.lt-setting-row {
  display: flex;
  align-items: center;
  min-height: 40px;
  gap: 10px;
  padding: 0 2px;
  cursor: pointer;
}

.lt-setting-icon {
  color: var(--md-on-surface-variant);
  font-size: 18px;
}

.lt-setting-label {
  flex: 1;
  min-width: 0;
  font-size: 12px;
}

.lt-setting-row input[type='checkbox'] {
  width: 18px;
  height: 18px;
  flex: 0 0 auto;
  accent-color: var(--md-primary);
  cursor: pointer;
}

.lt-leave-button {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 100%;
  min-height: 40px;
  gap: 8px;
  margin-top: 14px;
  border-radius: var(--radius-sm);
  color: var(--md-error);
  font-size: 13px;
  font-weight: 600;
  transition: background 150ms;

  &:hover { background: var(--md-error-container); }

  .material-symbols-rounded { font-size: 18px; }
}

.spinning {
  animation: spin 1s linear infinite;
}

@keyframes spin { to { transform: rotate(360deg); } }

@media (max-width: 520px) {
  .lt-panel {
    right: 8px;
    bottom: 84px;
    width: calc(100vw - 16px);
    max-height: calc(100vh - 132px);
  }

  .lt-actions { flex-direction: column; }
}
</style>
