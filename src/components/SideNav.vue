<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watchEffect } from 'vue'
import { useRouter, useRoute } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { useSettingsStore } from '@/stores/settings'
import { readSidebarCollapsed, SIDEBAR_AUTO_RAIL_WIDTH, writeSidebarCollapsed } from '@/modules/layout/sidebar'

const router = useRouter()
const route = useRoute()
const { t } = useI18n()
const settings = useSettingsStore()

interface NavItem {
  path: string
  /** 路由 meta.section 或路由名，用来判定高亮 */
  match: string
  icon: string
  key: string
}

const primaryItems: NavItem[] = [
  { path: '/', match: 'home', icon: 'home', key: 'nav.home' },
  { path: '/explore', match: 'explore', icon: 'explore', key: 'nav.explore' },
  { path: '/library', match: 'library', icon: 'library_music', key: 'nav.library' },
]

const mineItems: NavItem[] = [
  { path: '/recent', match: 'recent', icon: 'history', key: 'nav.recent' },
  { path: '/downloads', match: 'downloads', icon: 'download', key: 'nav.downloads' },
  { path: '/stats', match: 'playback-stats', icon: 'insights', key: 'nav.stats' },
]

const footerItems = computed<NavItem[]>(() => {
  const items: NavItem[] = [{ path: '/settings', match: 'settings', icon: 'settings', key: 'nav.settings' }]
  if (settings.devModeEnabled) items.unshift({ path: '/debug', match: 'debug', icon: 'bug_report', key: 'settings.debug' })
  return items
})

// 「我的」分组下的页面同时属于音乐库板块：命中具体子页时只点亮子页，音乐库入口不再重复高亮
const mineNames = new Set(mineItems.map(item => item.match))
function isActive(item: NavItem): boolean {
  const name = String(route.name ?? '')
  if (mineNames.has(item.match)) return name === item.match
  if (mineNames.has(name)) return false
  return ((route.meta.section as string) ?? name) === item.match
}

const userCollapsed = ref(readSidebarCollapsed())
const viewportWidth = ref(window.innerWidth)
function onResize() { viewportWidth.value = window.innerWidth }
onMounted(() => window.addEventListener('resize', onResize, { passive: true }))
onBeforeUnmount(() => window.removeEventListener('resize', onResize))

// 窄窗口自动收成图标栏，让内容区保留足够宽度；宽窗口则尊重用户的折叠选择
const autoRail = computed(() => viewportWidth.value < SIDEBAR_AUTO_RAIL_WIDTH)
const collapsed = computed(() => autoRail.value || userCollapsed.value)

watchEffect(() => {
  document.documentElement.classList.toggle('sidebar-collapsed', collapsed.value)
})

function toggleCollapsed() {
  userCollapsed.value = !userCollapsed.value
  writeSidebarCollapsed(userCollapsed.value)
}

function go(item: NavItem) {
  if (route.path !== item.path) void router.push(item.path)
}
</script>

<template>
  <nav class="side-nav" :class="{ collapsed }" :aria-label="t('nav.home')">
    <div class="nav-scroll">
      <ul class="nav-group" role="list">
        <li v-for="item in primaryItems" :key="item.path">
          <a
            class="nav-item"
            :class="{ active: isActive(item) }"
            :href="item.path"
            :title="collapsed ? t(item.key) : undefined"
            :aria-current="isActive(item) ? 'page' : undefined"
            @click.prevent="go(item)"
          >
            <span class="material-symbols-rounded nav-icon" :class="{ filled: isActive(item) }">{{ item.icon }}</span>
            <span class="nav-label">{{ t(item.key) }}</span>
          </a>
        </li>
      </ul>

      <div class="nav-section-title" aria-hidden="true">{{ t('nav.section_mine') }}</div>
      <div class="nav-divider" aria-hidden="true" />
      <ul class="nav-group" role="list">
        <li v-for="item in mineItems" :key="item.path">
          <a
            class="nav-item"
            :class="{ active: isActive(item) }"
            :href="item.path"
            :title="collapsed ? t(item.key) : undefined"
            :aria-current="isActive(item) ? 'page' : undefined"
            @click.prevent="go(item)"
          >
            <span class="material-symbols-rounded nav-icon" :class="{ filled: isActive(item) }">{{ item.icon }}</span>
            <span class="nav-label">{{ t(item.key) }}</span>
          </a>
        </li>
      </ul>
    </div>

    <ul class="nav-group nav-footer" role="list">
      <li v-for="item in footerItems" :key="item.path">
        <a
          class="nav-item"
          :class="{ active: isActive(item) }"
          :href="item.path"
          :title="collapsed ? t(item.key) : undefined"
          :aria-current="isActive(item) ? 'page' : undefined"
          @click.prevent="go(item)"
        >
          <span class="material-symbols-rounded nav-icon" :class="{ filled: isActive(item) }">{{ item.icon }}</span>
          <span class="nav-label">{{ t(item.key) }}</span>
        </a>
      </li>
      <li v-if="!autoRail">
        <button
          class="nav-item nav-toggle"
          type="button"
          :title="t(collapsed ? 'nav.expand_sidebar' : 'nav.collapse_sidebar')"
          :aria-label="t(collapsed ? 'nav.expand_sidebar' : 'nav.collapse_sidebar')"
          :aria-expanded="!collapsed"
          @click="toggleCollapsed"
        >
          <span class="material-symbols-rounded nav-icon">{{ collapsed ? 'left_panel_open' : 'left_panel_close' }}</span>
          <span class="nav-label">{{ t('nav.collapse_sidebar') }}</span>
        </button>
      </li>
    </ul>
  </nav>
</template>

<style scoped lang="scss">
.side-nav {
  width: var(--sidebar-width, 228px);
  height: 100%;
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
  /* 侧栏从窗口顶边开始，顶栏品牌与窗口按钮叠在它上面 */
  padding: calc(var(--titlebar-height, 36px) + 10px) 12px 12px;
  background: var(--clean-sidebar, var(--md-surface-container));
  border-right: 1px solid var(--hairline);
  transition: width 260ms var(--ease-emphasized), padding 260ms var(--ease-emphasized);
  overflow: hidden;

  &.collapsed { padding-left: 10px; padding-right: 10px; }
}

.nav-scroll {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  overflow-x: hidden;
  scrollbar-width: none;
  &::-webkit-scrollbar { display: none; }
}

.nav-group {
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.nav-footer {
  padding-top: 10px;
  border-top: 1px solid var(--hairline);
}

.nav-section-title {
  margin: 18px 12px 6px;
  font-size: var(--fs-11);
  font-weight: 600;
  letter-spacing: 0.02em;
  color: var(--md-on-surface-variant);
  white-space: nowrap;
  overflow: hidden;
  transition: opacity 160ms var(--ease-standard);

  .collapsed & { opacity: 0; height: 0; margin: 0; }
}

/* 折叠后用分隔线代替分组标题，保持分组关系 */
.nav-divider {
  display: none;
  height: 1px;
  margin: 10px 8px;
  background: var(--hairline);

  .collapsed & { display: block; }
}

.nav-item {
  position: relative;
  display: flex;
  align-items: center;
  gap: 12px;
  width: 100%;
  height: 38px;
  padding: 0 12px;
  border-radius: var(--radius-sm);
  color: var(--md-on-surface-variant);
  font-size: var(--fs-14);
  font-weight: 500;
  text-align: left;
  white-space: nowrap;
  cursor: pointer;
  transition: background var(--duration-short) var(--ease-standard),
              color var(--duration-short) var(--ease-standard),
              padding 260ms var(--ease-emphasized);

  .collapsed & {
    justify-content: center;
    gap: 0;
    padding: 0;
  }

  &:hover {
    background: var(--state-hover);
    color: var(--md-on-surface);
  }

  &:active { background: var(--state-pressed); }

  &.active {
    background: var(--md-secondary-container);
    color: var(--md-on-secondary-container);
    font-weight: 600;

    .nav-icon { color: var(--md-primary); }
  }

  &:focus-visible {
    outline: 2px solid var(--md-primary);
    outline-offset: -2px;
  }
}

.nav-icon {
  font-size: 22px;
  flex-shrink: 0;
  transition: color var(--duration-short) var(--ease-standard);
}

.nav-label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  transition: opacity 180ms var(--ease-standard);

  .collapsed & {
    max-width: 0;
    opacity: 0;
    pointer-events: none;
  }
}

@media (prefers-reduced-motion: reduce) {
  .side-nav, .nav-item, .nav-label { transition: none; }
}
</style>
