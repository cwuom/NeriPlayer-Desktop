<script setup lang="ts">
// 曲目行共用右键菜单，播放与多选仍由所在列表处理
// 「播放」「多选」与列表上下文有关，交给父组件；其余动作在这里完成
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { invoke } from '@tauri-apps/api/core'
import ContextMenu from '@/components/ui/ContextMenu.vue'
import AddToPlaylistDialog from '@/components/AddToPlaylistDialog.vue'
import BiliVideoSkipDialog from '@/components/BiliVideoSkipDialog.vue'
import { resolveBiliVideoSkipBvid } from '@/modules/playback/biliVideoSkip'
import { usePlayerStore, type TrackInfo } from '@/stores/player'
import { useToastStore } from '@/stores/toast'
import { useDownloadStore } from '@/stores/download'
import { useTrackDownloadMenu } from '@/composables/useTrackDownloadMenu'
import {
  createContextMenuItem,
  createContextMenuSeparator,
  type ContextMenuActionItem,
  type ContextMenuItem,
} from '@/utils/contextMenu'
import { isLocalTrack, localTrackFilePath } from '@/modules/library/localTrack'
import { writeClipboardText } from '@/utils/clipboard'
import { createLogger } from '@/utils/logger'

const props = withDefaults(defineProps<{
  /** 显示「播放」，点击后发出 play */
  playable?: boolean
  /** 显示「多选」，点击后发出 select */
  selectable?: boolean
  /** 显示「加入歌单」（歌单详情页里通常已有同类入口时可关掉） */
  playlistAction?: boolean
  /** 追加在末尾的页面专属动作，点击后发出 action */
  extraItems?: (track: TrackInfo) => ContextMenuItem[]
}>(), {
  playable: true,
  selectable: false,
  playlistAction: true,
  extraItems: undefined,
})

const emit = defineEmits<{
  play: [track: TrackInfo]
  select: [track: TrackInfo]
  action: [id: string, track: TrackInfo]
}>()

const { t } = useI18n()
const player = usePlayerStore()
const toast = useToastStore()
const downloads = useDownloadStore()
const log = createLogger('track-context-menu')

/** 本地文件或已下载的曲目在磁盘上的位置 */
function filePathOf(track: TrackInfo): string {
  return localTrackFilePath(track) || downloads.getDownloadedTrack(track.id)?.filePath || ''
}

const menu = ref<{ open: boolean; x: number; y: number; track: TrackInfo | null }>({
  open: false, x: 0, y: 0, track: null,
})
const playlistDialogOpen = ref(false)
const playlistTarget = ref<TrackInfo | null>(null)
const videoSkipDialogOpen = ref(false)
const videoSkipTarget = ref<TrackInfo | null>(null)

function close() {
  menu.value.open = false
}

const { downloadMenuItem, downloadFromMenu } = useTrackDownloadMenu(() => menu.value.track, close)

const items = computed<ContextMenuItem[]>(() => {
  const track = menu.value.track
  if (!track) return []
  const list: ContextMenuItem[] = []
  if (props.playable) list.push(createContextMenuItem(t('player.play_track'), { id: 'play', icon: 'play_arrow' }))
  if (props.selectable) list.push(createContextMenuItem(t('common.multi_select'), { id: 'select', icon: 'checklist' }))
  if (list.length) list.push(createContextMenuSeparator('track-primary'))
  list.push(
    createContextMenuItem(t('player.play_next'), { id: 'play-next', icon: 'queue_play_next' }),
    createContextMenuItem(t('player.add_to_queue'), { id: 'add-to-queue', icon: 'add_to_queue' }),
  )
  if (props.playlistAction) {
    list.push(createContextMenuItem(t('player.add_to_playlist'), { id: 'add-to-playlist', icon: 'playlist_add' }))
  }
  if (!isLocalTrack(track)) list.push(downloadMenuItem.value)
  if (resolveBiliVideoSkipBvid(track)) {
    list.push(createContextMenuItem(t('player.bili_skip_manage'), { id: 'bili-video-skip', icon: 'skip_next' }))
  }
  list.push(
    createContextMenuSeparator('track-copy'),
    createContextMenuItem(t('player.copy_title'), { id: 'copy-title', icon: 'content_copy' }),
    createContextMenuItem(t('player.copy_track_info'), { id: 'copy-info', icon: 'copy_all', disabled: !track.artist.trim() }),
  )
  if (filePathOf(track)) {
    list.push(createContextMenuItem(t('library.show_in_folder'), { id: 'reveal', icon: 'folder_open' }))
  }
  const extra = props.extraItems?.(track) ?? []
  if (extra.length) list.push(createContextMenuSeparator('track-extra'), ...extra)
  return list
})

function openAt(x: number, y: number, track: TrackInfo) {
  menu.value = { open: true, x, y, track }
}

/** 右键在光标处弹出；「更多」按钮点击时贴着按钮弹出 */
function open(event: MouseEvent, track: TrackInfo) {
  event.preventDefault()
  event.stopPropagation()
  if (event.type === 'contextmenu' || !(event.currentTarget instanceof HTMLElement)) {
    openAt(event.clientX, event.clientY, track)
    return
  }
  const rect = event.currentTarget.getBoundingClientRect()
  openAt(rect.left, rect.bottom + 4, track)
}

async function copy(text: string) {
  try {
    await writeClipboardText(text)
    toast.success(t('player.copied'))
  } catch {
    toast.error(t('player.copy_failed'))
  }
}

async function reveal(path: string) {
  try {
    await invoke('reveal_file', { path })
  } catch (error) {
    log.warn('reveal local file failed:', error)
    toast.error(t('download.reveal_failed'))
  }
}

function handleClick(item: ContextMenuActionItem) {
  const track = menu.value.track
  if (!track || !item.id) return
  switch (item.id) {
    case 'play': emit('play', track); break
    case 'select': emit('select', track); break
    case 'play-next': player.addToQueueNext(track); break
    case 'add-to-queue': player.addToQueueEnd(track); break
    case 'add-to-playlist':
      playlistTarget.value = track
      playlistDialogOpen.value = true
      break
    case 'download': void downloadFromMenu(); break
    case 'bili-video-skip':
      if (!resolveBiliVideoSkipBvid(track)) return
      videoSkipTarget.value = { ...track, syncPayload: track.syncPayload ? { ...track.syncPayload } : undefined }
      videoSkipDialogOpen.value = true
      close()
      break
    case 'copy-title': void copy(track.title); break
    case 'copy-info': void copy(`${track.title} - ${track.artist}`); break
    case 'reveal': {
      const path = filePathOf(track)
      if (path) void reveal(path)
      break
    }
    default: emit('action', item.id, track)
  }
}

defineExpose({ open, openAt, close })
</script>

<template>
  <ContextMenu
    v-model:open="menu.open"
    :x="menu.x"
    :y="menu.y"
    :items="items"
    @click="handleClick"
  />
  <AddToPlaylistDialog v-model:open="playlistDialogOpen" :track="playlistTarget" />
  <BiliVideoSkipDialog v-model:open="videoSkipDialogOpen" :track="videoSkipTarget" />
</template>
