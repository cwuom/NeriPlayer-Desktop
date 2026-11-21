import { ref, type Ref } from 'vue'
import i18n from '@/i18n'
import { useListenTogetherStore } from '@/stores/listenTogether'
import { useToastStore } from '@/stores/toast'
import type { TrackInfo } from '@/types/track'
import type { PlaybackCommandSource } from './types'

let _remoteSyncGuardUntil = 0

/** 播放命令的来源记录：一起听据此区分用户操作与房间同步，并在房间内拦截本地控制 */
export function createCommandSource(currentTrack: Ref<TrackInfo | null>) {
  const lastCommandSource = ref<PlaybackCommandSource>('local')
  const lastCommandSequence = ref(0)

  function markCommandSource(source: PlaybackCommandSource) {
    lastCommandSource.value = source
    lastCommandSequence.value++
    if (source === 'remote_sync') {
      _remoteSyncGuardUntil = Date.now() + 3000
    } else if (source === 'local') {
      _remoteSyncGuardUntil = 0
    }
  }

  /**
   * 一起听里听众没有控制权（房主离线或关闭了成员控制）时，用户操作在执行前就拦下并提示，
   * 不再先改本地、再被服务端拒绝而和房间分叉（对齐 Android shouldBlockLocalRoomControl）
   */
  function blockedByListenTogether(commandSource: PlaybackCommandSource): boolean {
    if (commandSource !== 'local') return false
    const restriction = useListenTogetherStore().localControlRestriction
    if (!restriction) return false
    useToastStore().error(i18n.global.t(restriction === 'controller_offline'
      ? 'listen_together.control_blocked_controller_offline'
      : 'listen_together.control_blocked_member_control'))
    return true
  }

  /** 播放器替用户做的后续动作（恢复、失败跳过）：跟随房间的同步仍按同步处理，其余都不算用户操作 */
  function internalFollowUpSource(commandSource: PlaybackCommandSource): PlaybackCommandSource {
    return commandSource === 'remote_sync' ? 'remote_sync' : 'local_safety'
  }

  /** 一起听期间不能切到本地文件：其它成员没法播放它（对齐 Android shouldBlockLocalSongSwitch） */
  function blocksLocalSongInRoom(track: TrackInfo, commandSource: PlaybackCommandSource): boolean {
    if (commandSource !== 'local' || !useListenTogetherStore().roomId) return false
    if (track.source !== 'local' && !track.id.startsWith('local:')) return false
    // 重新加载当前这首（例如继续播放时需要重载）不是切歌
    if (track.id === currentTrack.value?.id) return false
    useToastStore().error(i18n.global.t('listen_together.local_playback_blocked'))
    return true
  }

  function isRemoteSyncGuardActive() {
    return Date.now() < _remoteSyncGuardUntil
  }

  return {
    lastCommandSource, lastCommandSequence,
    markCommandSource, blockedByListenTogether, internalFollowUpSource, blocksLocalSongInRoom, isRemoteSyncGuardActive,
  }
}
