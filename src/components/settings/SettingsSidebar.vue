<script setup lang="ts">
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import type { SettingsNavGroup, SettingsSectionId } from './useSettingsSections'
import type { SettingsSearchResult } from './useSettingsSearch'

// 设置页左列：搜索框、搜索结果与分区导航。状态归设置页所有，这里只负责呈现和键盘交互
const props = defineProps<{
  settingsSearchResults: SettingsSearchResult[]
  settingsNavGroups: SettingsNavGroup[]
  activeSettingsSection: SettingsSectionId
}>()
const emit = defineEmits<{
  select: [id: SettingsSectionId]
  openResult: [result: SettingsSearchResult | undefined]
}>()
const settingsQuery = defineModel<string>('settingsQuery', { required: true })

const { t } = useI18n()
const settingsSearchInput = ref<HTMLInputElement | null>(null)

function selectSettingsSection(id: SettingsSectionId) {
  emit('select', id)
}

function openSettingsSearchResult(result: SettingsSearchResult | undefined) {
  emit('openResult', result)
}

function sectionIcon(id: SettingsSectionId): string {
  return props.settingsNavGroups.flatMap(group => group.items).find(item => item.id === id)?.icon ?? 'settings'
}

function sectionTitle(id: SettingsSectionId): string {
  return props.settingsNavGroups.flatMap(group => group.items).find(item => item.id === id)?.label ?? ''
}

function onSettingsSearchKeydown(event: KeyboardEvent) {
  if (event.key === 'Enter') {
    event.preventDefault()
    openSettingsSearchResult(props.settingsSearchResults[0])
  } else if (event.key === 'Escape' && settingsQuery.value) {
    event.preventDefault()
    event.stopPropagation()
    settingsQuery.value = ''
  }
}
</script>

<template>
  <aside class="settings-sidebar">
    <h1 class="page-title">{{ t('settings.title') }}</h1>
    <label class="settings-search">
      <span class="material-symbols-rounded" aria-hidden="true">search</span>
      <input
        ref="settingsSearchInput"
        v-model="settingsQuery"
        type="search"
        :placeholder="t('settings.search_hint')"
        :aria-label="t('settings.search_hint')"
        @keydown="onSettingsSearchKeydown"
      />
      <button
        v-if="settingsQuery"
        type="button"
        class="settings-search-clear"
        :aria-label="t('common.clear')"
        @click="settingsQuery = ''; settingsSearchInput?.focus()"
      >
        <span class="material-symbols-rounded">close</span>
      </button>
    </label>
    <Transition name="fade" mode="out-in">
    <div v-if="settingsQuery.trim()" key="search" class="settings-search-results settings-nav-group" role="listbox" :aria-label="t('settings.search_hint')">
      <TransitionGroup name="settings-result">
        <button
          v-for="result in settingsSearchResults"
          :key="result.key"
          type="button"
          role="option"
          class="settings-nav-item settings-search-result"
          @click="openSettingsSearchResult(result)"
        >
          <span class="settings-nav-icon material-symbols-rounded">{{ result.isSection ? sectionIcon(result.section) : 'tune' }}</span>
          <span class="settings-nav-copy">
            <strong>{{ result.title }}</strong>
            <small>{{ result.isSection ? result.desc : sectionTitle(result.section) }}</small>
          </span>
          <span class="material-symbols-rounded settings-nav-arrow">chevron_right</span>
        </button>
      </TransitionGroup>
      <p v-if="!settingsSearchResults.length" class="settings-search-empty">{{ t('settings.search_empty') }}</p>
    </div>
    <nav v-else key="nav" class="settings-nav" :aria-label="t('settings.title')">
      <div v-for="(group, groupIndex) in settingsNavGroups" :key="groupIndex" class="settings-nav-group">
        <button
          v-for="item in group.items"
          :key="item.id"
          class="settings-nav-item"
          :class="{ active: activeSettingsSection === item.id }"
          type="button"
          :aria-current="activeSettingsSection === item.id ? 'page' : undefined"
          @click="selectSettingsSection(item.id)"
        >
          <span class="settings-nav-icon material-symbols-rounded">{{ item.icon }}</span>
          <span class="settings-nav-copy">
            <strong>{{ item.label }}</strong>
            <small>{{ item.description }}</small>
          </span>
          <span class="material-symbols-rounded settings-nav-arrow">chevron_right</span>
        </button>
      </div>
    </nav>
    </Transition>
  </aside>
</template>

<style scoped lang="scss">
/* 左列标题固定不滚动，仅分组列表滚动，与右列头部固定的结构对称，
   两列顶部基线不再随各自滚动位置错位 */
.settings-sidebar {
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  padding-top: 18px;
}

.settings-nav {
  display: flex;
  flex-direction: column;
  gap: 10px;
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  overflow-x: hidden;
  overscroll-behavior: contain;
  padding-right: 4px;

  scrollbar-width: none;
  &::-webkit-scrollbar { display: none; }
}

.settings-nav-group {
  padding: 4px;
  border-radius: var(--radius-lg);
  background: color-mix(in srgb, var(--md-surface-container) 78%, transparent);
}

.settings-search {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 42px;
  margin: -12px 4px 12px 0;
  padding: 0 6px 0 12px;
  border: 1px solid transparent;
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--md-surface-container-high) 82%, transparent);
  color: var(--md-on-surface-variant);
  transition: border-color 160ms var(--ease-standard), background 160ms var(--ease-standard);

  &:focus-within {
    border-color: var(--md-primary);
    background: var(--md-surface-container-high);
  }

  > .material-symbols-rounded { font-size: 20px; }

  input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: none;
    background: transparent;
    color: var(--md-on-surface);
    font: inherit;
    font-size: 14px;
    &::placeholder { color: var(--md-on-surface-variant); }
    &::-webkit-search-cancel-button { display: none; }
  }
}

.settings-search-clear {
  width: 30px;
  height: 30px;
  display: grid;
  place-items: center;
  border-radius: 50%;
  color: var(--md-on-surface-variant);
  .material-symbols-rounded { font-size: 18px; }
  &:hover { background: var(--md-surface-container-highest); color: var(--md-on-surface); }
}

.settings-search-results {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  overscroll-behavior: contain;
  margin-right: 4px;
  scrollbar-width: none;
  &::-webkit-scrollbar { display: none; }
}

.settings-search-result { min-height: 54px; }

.settings-search-empty {
  padding: 24px 12px;
  text-align: center;
  font-size: 13px;
  color: var(--md-on-surface-variant);
}

.settings-result-enter-active { transition: opacity 180ms var(--ease-standard), transform 220ms var(--ease-emphasized-decel); }
.settings-result-enter-from { opacity: 0; transform: translateY(6px); }
.settings-result-move { transition: transform 220ms var(--ease-emphasized-decel); }

.settings-nav-item {
  width: 100%;
  min-height: 60px;
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 12px;
  border: 1px solid transparent;
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--md-on-surface);
  text-align: left;
  cursor: pointer;
  transition: background var(--duration-short) var(--ease-standard),
              color var(--duration-short) var(--ease-standard);

  &:hover:not(.active) { background: var(--md-surface-container-high); }

  &.active {
    background: var(--md-primary-container);
    color: var(--md-on-primary-container);
  }
}

.settings-nav-icon {
  flex-shrink: 0;
  font-size: 21px;
  color: var(--md-primary);
}

.settings-nav-copy {
  min-width: 0;
  flex: 1;

  strong,
  small {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  strong {
    font-size: 13px;
    font-weight: 600;
    line-height: 1.25;
  }

  small {
    margin-top: 3px;
    color: var(--md-on-surface-variant);
    font-size: 11px;
    line-height: 1.35;
    white-space: nowrap;
  }
}

.settings-nav-item.active .settings-nav-icon,
.settings-nav-item.active .settings-nav-copy small {
  color: var(--md-on-primary-container);
}

.settings-nav-arrow {
  flex-shrink: 0;
  margin-left: auto;
  color: var(--md-on-surface-variant);
  font-size: 18px;
  opacity: 0.7;
}

.settings-nav-item.active .settings-nav-arrow {
  color: var(--md-on-primary-container);
}

.page-title {
  font-size: 28px;
  font-weight: 700;
  letter-spacing: -0.5px;
  /* 与右侧详情头部等高（h2 28px×1.2 + 8px 间距 + 描述 13px×1.5 ≈ 61px），
     保证左右两列列表顶部基线对齐 */
  min-height: 61px;
  margin: 0 0 24px;
}

@media (max-width: 760px) {
  .settings-sidebar {
    position: static;
    min-height: 0;
    overflow: visible;
    padding: 0;
  }

  .page-title {
    margin-bottom: 14px;
    font-size: 24px;
    /* 窄屏为单列布局，无需与右侧头部等高 */
    min-height: auto;
  }

  .settings-nav {
    flex-direction: row;
    gap: 8px;
    overflow-x: auto;
    padding: 0 2px 8px;
    scrollbar-width: none;

    &::-webkit-scrollbar { display: none; }
  }

  .settings-nav-group {
    display: contents;
  }

  .settings-nav-item {
    flex: 0 0 210px;
    min-height: 56px;
  }

  .settings-nav-copy small { display: none; }
}

@media (max-width: 480px) {
  .settings-nav-item {
    flex-basis: 184px;
    padding-inline: 10px;
  }
}
</style>
