<script setup lang="ts">
// 更多选项面板（对齐 Android MoreOptionsSheet）。样式在 NowPlaying.global.scss，Teleport 到 body 后同样生效
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { usePlayerStore } from '@/stores/player'
import { LYRIC_FONT_SCALE_MAX, LYRIC_FONT_SCALE_MIN, LYRIC_FONT_SCALE_STEP, useSettingsStore } from '@/stores/settings'
import { useToastStore } from '@/stores/toast'
import { useCurrentLyricsStore } from '@/stores/currentLyrics'
import { AUDIO_EFFECT_PRESETS } from '@/modules/audioEffects/model'
import { biliSubtitleTarget } from '@/modules/lyrics/biliSubtitle'
import { formatLyricOffsetMs, LYRIC_OFFSET_STEP_MS } from '@/modules/lyrics/lyricOffset'
import { resolveBiliVideoSkipBvid } from '@/modules/playback/biliVideoSkip'
import { closeDesktopLyricsWindow, desktopLyricsOpen, openDesktopLyricsWindow } from '@/modules/desktopLyrics/bridge'
import { normalizeDesktopLyricsStyle } from '@/modules/desktopLyrics/style'
import { createLogger } from '@/utils/logger'
import { summarizeLogError } from '@/utils/logSanitizer'
import AudioEffectsPanel from '../AudioEffectsPanel.vue'
import BiliSubtitleSettings from '../BiliSubtitleSettings.vue'
import BilibiliCoverImage from '../BilibiliCoverImage.vue'
import LyricsMatchPanel from '../LyricsMatchPanel.vue'
import EditableRangeValue from '../ui/EditableRangeValue.vue'
import NowPlayingFontScaleView from './NowPlayingFontScaleView.vue'
import NowPlayingTrackDetail from './NowPlayingTrackDetail.vue'
import { platformLabel } from './nowPlayingHelpers'
import { useNowPlayingSheetContext } from './sheetContext'
import { useCurrentTrackDownload } from './useCurrentTrackDownload'
import { useLyricOffsetControls } from './useLyricOffsetControls'
import { useLyricsEditing } from './useLyricsEditing'
import { useTrackInfoEditing } from './useTrackInfoEditing'
import { useTrackNavigation } from './useTrackNavigation'

const log = createLogger('now-playing')

const player = usePlayerStore()
const settings = useSettingsStore()
const toast = useToastStore()
const currentLyrics = useCurrentLyricsStore()
const { t } = useI18n()

const context = useNowPlayingSheetContext()
const {
  showMoreSheet, hideMoreSheet, moreSheetView, moreSheetTransition, goToSubView, goBackToMain,
  albumName, commentTarget, quality, shareSong, openQualitySwitcher, openBiliVideoSkip,
  openListenTogetherFromMore, openComments, openAudioEffectsSettings,
} = context
const { currentSource, switchableQualities, canSwitchCurrentAudioQuality, currentQualityKey, setQualityKey, currentAudioQualityLabel } = quality

const {
  lyricsEditorText, lyricsTranslationEditorText, lyricsRomanizationEditorText, lyricsEditorTab,
  openLyricsEditor, leaveLyricsEditor, applyLyricsFromEditor,
  lyricMatchTarget, lyricMatchSession, openLyricMatch, leaveLyricMatch, onLyricMatchPicked,
} = useLyricsEditing(context)
const {
  searchQuery, searchResults, isSearching, infoSearchPlatform, infoApplyCandidate, applyInfoFields, infoSearched,
  fieldPickerRef, openInfoSearch, selectInfoSearchPlatform, doSearch, applySearchResult, confirmApplySearchResult,
  editTitle, editArtist, editCoverUrl, openEditInfo, saveEditInfo, restoreInfo,
} = useTrackInfoEditing(context)
const {
  downloadActionIcon, downloadActionLabel, downloadActionDesc, downloadActionDisabled, handleDownloadAction,
} = useCurrentTrackDownload(showMoreSheet)
const { canViewNeteaseAlbum, primaryArtistName, canViewNeteaseArtist, openCurrentAlbum, openCurrentArtist } =
  useTrackNavigation(context)
const {
  currentLyricDefaultOffsetMs, currentLyricUserOffsetMs, currentLyricTotalOffsetMs, lyricOffsetSliderMin,
  lyricOffsetSliderMax, nudgeLyricOffset, resetLyricOffsetToDefault, currentLyricSourceLabel, lyricOffsetSummary,
} = useLyricOffsetControls()

const canManageBiliVideoSkip = computed(() => !!player.currentTrack && !!resolveBiliVideoSkipBvid(player.currentTrack))

async function toggleDesktopLyrics() {
  try {
    if (desktopLyricsOpen.value) {
      await closeDesktopLyricsWindow()
    } else {
      await openDesktopLyricsWindow()
      hideMoreSheet()
    }
  } catch (error) {
    log.warn('desktop lyrics window failed:', summarizeLogError(error))
    toast.error(t('player.desktop_lyrics_failed'))
  }
}

function toggleDesktopLyricsLock() {
  settings.desktopLyrics = normalizeDesktopLyricsStyle({ ...settings.desktopLyrics, locked: !settings.desktopLyrics.locked })
}

function nudgeLyricFontScale(steps: number) {
  const next = Math.round((settings.lyricFontScale + steps * LYRIC_FONT_SCALE_STEP) / LYRIC_FONT_SCALE_STEP) * LYRIC_FONT_SCALE_STEP
  settings.lyricFontScale = Math.min(LYRIC_FONT_SCALE_MAX, Math.max(LYRIC_FONT_SCALE_MIN, Number(next.toFixed(2))))
}

// 「更多」里音频效果一行直接写出当前生效的设置，都是默认时显示说明
const audioEffectsSummary = computed(() => {
  const parts: string[] = []
  if (player.playbackSpeed !== 1) parts.push(`${player.playbackSpeed.toFixed(2)}x`)
  if (player.vinylEffectEnabled) parts.push(t('player.vinyl_effect'))
  else if (player.playbackPitch !== 1) parts.push(`${t('player.pitch')} ${player.playbackPitch.toFixed(2)}x`)
  if (player.loudnessGainMb !== 0) parts.push(`+${(player.loudnessGainMb / 100).toFixed(1)}dB`)
  if (player.audioEffectsProfile.enabled) {
    const id = player.audioEffectsProfile.presetId
    parts.push(player.audioEffects.userPresets.find(preset => preset.id === id)?.name
      ?? (AUDIO_EFFECT_PRESETS.some(preset => preset.id === id) ? t(`dsp.presets.${id}`) : t('dsp.manual')))
  }
  return parts.length ? parts.join(' · ') : t('player.audio_effects_desc')
})

function openLyricsFill() {
  openLyricMatch('track')
}

const isQualitySwitching = ref(false)

async function switchQuality(key: string) {
  if (isQualitySwitching.value || !canSwitchCurrentAudioQuality.value || player.isLoadingAudio) return
  const source = currentSource.value
  const previousKey = currentQualityKey(source)
  if (!previousKey || previousKey === key) {
    showMoreSheet.value = false
    return
  }

  isQualitySwitching.value = true
  setQualityKey(source, key)
  try {
    showMoreSheet.value = false
    await player.replayWithQuality()
  } catch (e) {
    setQualityKey(source, previousKey)
    toast.error(String(e))
  } finally {
    isQualitySwitching.value = false
  }
}
</script>

<template>
  <!-- 更多选项面板（对齐 Android MoreOptionsSheet） -->
  <Teleport to="body">
    <Transition name="more-sheet">
    <div v-if="player.hasPlaybackSession && showMoreSheet" class="np-more-overlay" @click="showMoreSheet = false">
      <div class="np-more-sheet" @click.stop>

        <Transition :name="moreSheetTransition" mode="out-in">
        <div :key="moreSheetView" class="np-more-sheet-content">

        <!-- 主菜单：分组与顺序对齐 Android MoreOptionsMainContent；常用的偏移、字号直接在行内加减 -->
        <template v-if="moreSheetView === 'main'">
          <h4 class="np-more-title">{{ t('player.more_options') }}</h4>

          <!-- 歌曲信息 -->
          <div class="np-more-group">
            <button class="np-more-list-item" @click="openInfoSearch">
              <span class="material-symbols-rounded">info</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('player.get_info') }}</span>
              </div>
              <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
            </button>
            <button class="np-more-list-item" @click="openEditInfo">
              <span class="material-symbols-rounded">edit</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('player.edit_info') }}</span>
              </div>
              <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
            </button>
            <button v-if="commentTarget" class="np-more-list-item" @click="openComments()">
              <span class="material-symbols-rounded">chat_bubble</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('player.comments') }}</span>
              </div>
              <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
            </button>
          </div>

          <!-- 播放：音质、音频效果、下载 -->
          <div class="np-more-group">
            <button v-if="canSwitchCurrentAudioQuality" class="np-more-list-item" :disabled="player.isLoadingAudio" @click="openQualitySwitcher()">
              <span class="material-symbols-rounded">music_note</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('player.quality_switch') }}</span>
                <span class="np-more-list-desc">{{ currentAudioQualityLabel() || '—' }}</span>
              </div>
              <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
            </button>
            <button class="np-more-list-item" @click="goToSubView('effects')">
              <span class="material-symbols-rounded">tune</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('player.audio_effects') }}</span>
                <span class="np-more-list-desc">{{ audioEffectsSummary }}</span>
              </div>
              <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
            </button>
            <button
              v-if="currentSource !== 'local'"
              class="np-more-list-item"
              :disabled="downloadActionDisabled"
              @click="handleDownloadAction"
            >
              <span class="material-symbols-rounded">{{ downloadActionIcon }}</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ downloadActionLabel }}</span>
                <span v-if="downloadActionDesc" class="np-more-list-desc">{{ downloadActionDesc }}</span>
              </div>
            </button>
          </div>

          <!-- 歌词 -->
          <div class="np-more-group">
            <button v-if="player.currentTrack && biliSubtitleTarget(player.currentTrack)" class="np-more-list-item" @click="goToSubView('bili-subtitles')">
              <span class="material-symbols-rounded">subtitles</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('bili_subtitle.title') }}</span>
                <span v-if="currentLyrics.subtitle?.track" class="np-more-list-desc">{{ currentLyrics.subtitle.track.label }}</span>
              </div>
              <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
            </button>
            <div class="np-more-list-item np-more-list-item--stepper">
              <button class="np-more-stepper-main" @click="goToSubView('offset')">
                <span class="material-symbols-rounded">timer</span>
                <div class="np-more-list-info">
                  <span class="np-more-list-headline">{{ t('player.lyric_offset') }}</span>
                  <span class="np-more-list-desc" :class="{ 'np-more-list-desc--custom': currentLyricUserOffsetMs !== 0 }">{{ lyricOffsetSummary }}</span>
                </div>
              </button>
              <div class="np-more-stepper">
                <button :title="t('player.lyric_offset_later')" :aria-label="t('player.lyric_offset_later')" @click="nudgeLyricOffset(-1)">
                  <span class="material-symbols-rounded">remove</span>
                </button>
                <button :title="t('player.lyric_offset_earlier')" :aria-label="t('player.lyric_offset_earlier')" @click="nudgeLyricOffset(1)">
                  <span class="material-symbols-rounded">add</span>
                </button>
              </div>
            </div>
            <div class="np-more-list-item np-more-list-item--stepper">
              <button class="np-more-stepper-main" @click="goToSubView('fontsize')">
                <span class="material-symbols-rounded">format_size</span>
                <div class="np-more-list-info">
                  <span class="np-more-list-headline">{{ t('player.font_scale') }}</span>
                  <span class="np-more-list-desc">{{ Math.round(settings.lyricFontScale * 100) }}%</span>
                </div>
              </button>
              <div class="np-more-stepper">
                <button
                  :disabled="settings.lyricFontScale <= LYRIC_FONT_SCALE_MIN"
                  :title="t('player.font_scale_smaller')" :aria-label="t('player.font_scale_smaller')"
                  @click="nudgeLyricFontScale(-1)"
                >
                  <span class="material-symbols-rounded">text_decrease</span>
                </button>
                <button
                  :disabled="settings.lyricFontScale >= LYRIC_FONT_SCALE_MAX"
                  :title="t('player.font_scale_larger')" :aria-label="t('player.font_scale_larger')"
                  @click="nudgeLyricFontScale(1)"
                >
                  <span class="material-symbols-rounded">text_increase</span>
                </button>
              </div>
            </div>
            <button class="np-more-list-item" :class="{ active: desktopLyricsOpen }" @click="toggleDesktopLyrics">
              <span class="material-symbols-rounded">picture_in_picture_alt</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('player.desktop_lyrics') }}</span>
              </div>
              <span v-if="desktopLyricsOpen" class="material-symbols-rounded np-more-list-check">check</span>
            </button>
            <button v-if="desktopLyricsOpen" class="np-more-list-item" @click="toggleDesktopLyricsLock">
              <span class="material-symbols-rounded">{{ settings.desktopLyrics.locked ? 'lock' : 'lock_open' }}</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t(settings.desktopLyrics.locked ? 'desktop_lyrics.unlock' : 'desktop_lyrics.lock') }}</span>
              </div>
            </button>
            <button class="np-more-list-item" @click="openLyricsFill">
              <span class="material-symbols-rounded">lyrics</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('player.lyrics_fill') }}</span>
                <span class="np-more-list-desc">{{ t('player.lyrics_fill_desc') }}</span>
              </div>
              <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
            </button>
            <button class="np-more-list-item" @click="openLyricsEditor()">
              <span class="material-symbols-rounded">edit_note</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('player.lyrics_editor') }}</span>
              </div>
              <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
            </button>
          </div>

          <!-- 浏览与分享 -->
          <div class="np-more-group">
            <button v-if="canManageBiliVideoSkip" class="np-more-list-item" @click="openBiliVideoSkip">
              <span class="material-symbols-rounded">skip_next</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('player.bili_skip_manage') }}</span>
              </div>
              <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
            </button>
            <button v-if="canViewNeteaseAlbum" class="np-more-list-item" @click="openCurrentAlbum">
              <span class="material-symbols-rounded">library_music</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('player.view_album', { name: albumName }) }}</span>
              </div>
            </button>
            <button v-if="canViewNeteaseArtist" class="np-more-list-item" @click="openCurrentArtist">
              <span class="material-symbols-rounded">artist</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('player.view_artist', { name: primaryArtistName }) }}</span>
              </div>
            </button>
            <button class="np-more-list-item" @click="shareSong">
              <span class="material-symbols-rounded">share</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('player.share') }}</span>
              </div>
            </button>
            <button class="np-more-list-item" @click="openListenTogetherFromMore">
              <span class="material-symbols-rounded">headphones</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('listen_together.title') }}</span>
                <span class="np-more-list-desc">{{ t('listen_together.entry_desc') }}</span>
              </div>
              <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
            </button>
            <button class="np-more-list-item" @click="goToSubView('track-detail')">
              <span class="material-symbols-rounded">article</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('player.track_detail') }}</span>
                <span class="np-more-list-desc">{{ t('player.track_detail_desc') }}</span>
              </div>
              <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
            </button>
          </div>
        </template>

        <template v-else-if="moreSheetView === 'bili-subtitles'">
          <div class="np-more-sub-header">
            <button class="np-more-back" :aria-label="t('common.back')" @click="goBackToMain()"><span class="material-symbols-rounded">arrow_back</span></button>
            <h4 class="np-more-title">{{ t('bili_subtitle.title') }}</h4>
          </div>
          <BiliSubtitleSettings :track="player.currentTrack" />
        </template>

        <!-- 子视图：歌词偏移（显示与编辑的都是实际生效的偏移） -->
        <template v-else-if="moreSheetView === 'offset'">
          <div class="np-more-sub-header">
            <button class="np-more-back" @click="goBackToMain()">
              <span class="material-symbols-rounded">arrow_back</span>
            </button>
            <h4 class="np-more-title">{{ t('player.lyric_offset') }}</h4>
          </div>
          <div class="np-more-item">
            <div class="np-more-hint">
              {{ currentLyricSourceLabel
                ? t('player.lyric_offset_default_of', { source: currentLyricSourceLabel, value: formatLyricOffsetMs(currentLyricDefaultOffsetMs) })
                : t('player.lyric_offset_default', { value: formatLyricOffsetMs(currentLyricDefaultOffsetMs) }) }}
            </div>
            <div class="np-more-row">
              <button class="np-more-step-btn" :title="t('player.lyric_offset_later')" :aria-label="t('player.lyric_offset_later')" @click="nudgeLyricOffset(-1)">
                <span class="material-symbols-rounded">remove</span>
              </button>
              <input type="range"
                :min="lyricOffsetSliderMin" :max="lyricOffsetSliderMax" :step="LYRIC_OFFSET_STEP_MS"
                :value="currentLyricTotalOffsetMs"
                class="np-more-slider"
                :aria-label="t('player.lyric_offset')"
                @input="currentLyricTotalOffsetMs = parseInt(($event.target as HTMLInputElement).value)"
              />
              <button class="np-more-step-btn" :title="t('player.lyric_offset_earlier')" :aria-label="t('player.lyric_offset_earlier')" @click="nudgeLyricOffset(1)">
                <span class="material-symbols-rounded">add</span>
              </button>
              <EditableRangeValue
                v-model="currentLyricTotalOffsetMs"
                class="np-offset-value"
                :class="{ positive: currentLyricTotalOffsetMs > 0, negative: currentLyricTotalOffsetMs < 0 }"
                :min="lyricOffsetSliderMin"
                :max="lyricOffsetSliderMax"
                :step="LYRIC_OFFSET_STEP_MS"
                :display-value="formatLyricOffsetMs(currentLyricTotalOffsetMs)"
                input-suffix="ms"
                :aria-label="t('player.lyric_offset')"
              />
            </div>
            <p class="np-more-hint np-more-hint--below">{{ t('player.lyric_offset_hint') }}</p>
            <button v-if="currentLyricUserOffsetMs !== 0" class="np-more-text-btn" @click="resetLyricOffsetToDefault">
              <span class="material-symbols-rounded">restart_alt</span>
              {{ t('player.lyric_offset_reset', { value: formatLyricOffsetMs(currentLyricDefaultOffsetMs) }) }}
            </button>
          </div>
        </template>

        <!-- 子视图：字号 -->
        <template v-else-if="moreSheetView === 'fontsize'">
          <NowPlayingFontScaleView @back="goBackToMain()" />
        </template>

        <!-- 子视图：音频效果，与工具栏的音效弹层是同一个面板 -->
        <template v-else-if="moreSheetView === 'effects'">
          <div class="np-more-sub-header">
            <button class="np-more-back" @click="goBackToMain()">
              <span class="material-symbols-rounded">arrow_back</span>
            </button>
            <h4 class="np-more-title">{{ t('player.audio_effects') }}</h4>
          </div>
          <AudioEffectsPanel class="np-more-effects" @details="openAudioEffectsSettings" />
        </template>

        <!-- 子视图：获取歌曲信息 -->
        <template v-else-if="moreSheetView === 'search'">
          <div class="np-more-sub-header">
            <button class="np-more-back" @click="goBackToMain()">
              <span class="material-symbols-rounded">arrow_back</span>
            </button>
            <h4 class="np-more-title">{{ t('player.get_info') }}</h4>
          </div>
          <div class="np-more-search-bar">
            <input
              v-model="searchQuery"
              class="np-more-input"
              :placeholder="t('player.search_song')"
              @keydown.enter="doSearch"
            />
            <button class="np-more-search-btn" @click="doSearch" :disabled="isSearching">
              <span class="material-symbols-rounded">search</span>
            </button>
          </div>
          <div class="np-more-segmented platform">
            <button
              :class="{ active: infoSearchPlatform === 'netease' }"
              @click="selectInfoSearchPlatform('netease')"
            >
              {{ t('player.source_netease') }}
            </button>
            <button
              :class="{ active: infoSearchPlatform === 'qq' }"
              @click="selectInfoSearchPlatform('qq')"
            >
              {{ t('player.source_qq') }}
            </button>
            <button
              :class="{ active: infoSearchPlatform === 'bilibili' }"
              @click="selectInfoSearchPlatform('bilibili')"
            >
              Bilibili
            </button>
            <button
              :class="{ active: infoSearchPlatform === 'youtube' }"
              @click="selectInfoSearchPlatform('youtube')"
            >
              YouTube
            </button>
          </div>
          <div v-if="isSearching" class="np-more-status">{{ t('player.searching') }}</div>
          <div v-else-if="infoSearched && searchResults.length === 0" class="np-more-status">{{ t('player.no_results') }}</div>
          <div class="np-more-search-results" :class="{ compact: !!infoApplyCandidate }">
            <button
              v-for="(r, ri) in searchResults"
              :key="ri"
              class="np-more-search-item"
              :class="{ active: infoApplyCandidate === r }"
              @click="applySearchResult(r)"
            >
              <BilibiliCoverImage :src="r.cover_url" class="np-more-search-cover"><span class="np-more-search-cover np-more-search-cover-fallback material-symbols-rounded filled">music_note</span></BilibiliCoverImage>
              <div class="np-more-search-info">
                <span class="np-more-search-title">{{ r.title }}</span>
                <span class="np-more-search-artist">{{ r.artist }}</span>
              </div>
              <span class="np-more-search-source">{{ platformLabel(r.source, t) }}</span>
            </button>
          </div>
          <div v-if="infoApplyCandidate" ref="fieldPickerRef" class="np-more-field-picker">
            <div class="np-more-candidate-preview">
              <BilibiliCoverImage
                v-if="infoApplyCandidate.cover_url || infoApplyCandidate.coverUrl"
                :src="infoApplyCandidate.cover_url || infoApplyCandidate.coverUrl"
                class="np-more-candidate-cover"
              />
              <div class="np-more-search-info">
                <span class="np-more-search-title">{{ infoApplyCandidate.title }}</span>
                <span class="np-more-search-artist">{{ infoApplyCandidate.artist }}</span>
              </div>
            </div>
            <div class="np-more-field-title">{{ t('player.fill_fields_title') }}</div>
            <div class="np-more-field-options">
              <label class="np-more-chip">
                <input v-model="applyInfoFields.title" type="checkbox" />
                <span>{{ t('player.song_title') }}</span>
              </label>
              <label class="np-more-chip">
                <input v-model="applyInfoFields.artist" type="checkbox" />
                <span>{{ t('player.artist_name') }}</span>
              </label>
              <label class="np-more-chip">
                <input v-model="applyInfoFields.cover" type="checkbox" />
                <span>{{ t('player.fill_field_cover') }}</span>
              </label>
              <label class="np-more-chip">
                <input v-model="applyInfoFields.lyrics" type="checkbox" />
                <span>{{ t('player.fill_field_lyrics') }}</span>
              </label>
            </div>
            <div class="np-more-form-actions compact">
              <button class="np-more-form-btn primary" @click="confirmApplySearchResult">
                <span class="material-symbols-rounded">check</span>
                {{ t('player.apply_selection') }}
              </button>
              <button class="np-more-form-btn" @click="infoApplyCandidate = null">
                {{ t('common.cancel') }}
              </button>
            </div>
          </div>
        </template>

        <!-- 子视图：编辑歌曲信息 -->
        <template v-else-if="moreSheetView === 'editinfo'">
          <div class="np-more-sub-header">
            <button class="np-more-back" @click="goBackToMain()">
              <span class="material-symbols-rounded">arrow_back</span>
            </button>
            <h4 class="np-more-title">{{ t('player.edit_info') }}</h4>
          </div>
          <div class="np-more-form">
            <label class="np-more-form-label">{{ t('player.song_title') }}</label>
            <input v-model="editTitle" class="np-more-input" />

            <label class="np-more-form-label">{{ t('player.artist_name') }}</label>
            <input v-model="editArtist" class="np-more-input" />

            <label class="np-more-form-label">{{ t('player.cover_url_label') }}</label>
            <input v-model="editCoverUrl" class="np-more-input" />

            <!-- 对齐 Android：歌曲信息里直接进歌词编辑 -->
            <div class="np-more-form-actions">
              <button class="np-more-form-btn" @click="openLyricsEditor('editinfo')">
                <span class="material-symbols-rounded">edit_note</span>
                {{ t('player.edit_lyrics') }}
              </button>
            </div>

            <div class="np-more-form-actions">
              <button class="np-more-form-btn primary" @click="saveEditInfo">
                <span class="material-symbols-rounded">check</span>
                {{ t('common.save') }}
              </button>
              <button v-if="player.hasOriginalTrackInfo()" class="np-more-form-btn" @click="restoreInfo">
                <span class="material-symbols-rounded">restore</span>
                {{ t('player.restore_original') }}
              </button>
            </div>
          </div>
        </template>

        <!-- 子视图：歌曲详情 -->
        <template v-else-if="moreSheetView === 'track-detail'">
          <NowPlayingTrackDetail />
        </template>

        <!-- 子视图：音质切换 -->
        <template v-else-if="moreSheetView === 'quality'">
          <div class="np-more-sub-header">
            <button class="np-more-back" @click="goBackToMain()">
              <span class="material-symbols-rounded">arrow_back</span>
            </button>
            <h4 class="np-more-title">{{ t('player.quality_switch') }}</h4>
          </div>
          <div v-if="canSwitchCurrentAudioQuality" class="np-more-quality-list">
            <button
              v-for="q in switchableQualities"
              :key="q.key"
              class="np-more-quality-item"
              :class="{ active: currentQualityKey() === q.key }"
              :disabled="isQualitySwitching"
              @click="switchQuality(q.key)"
            >
              <span>{{ t(q.label) }}</span>
              <span
                v-if="currentQualityKey() === q.key"
                class="material-symbols-rounded"
                style="font-size: 18px"
              >check</span>
            </button>
            <div v-if="isQualitySwitching" class="np-more-status">{{ t('player.quality_changing') }}</div>
          </div>
          <div v-else class="np-more-status">{{ t('player.not_available') }}</div>
        </template>

        <!-- 子视图：歌词编辑器（对齐 Android LyricsEditorSheet） -->
        <template v-else-if="moreSheetView === 'lyrics-editor'">
          <div class="np-more-sub-header">
            <button class="np-more-back" @click="leaveLyricsEditor()">
              <span class="material-symbols-rounded">arrow_back</span>
            </button>
            <h4 class="np-more-title">{{ t('player.lyrics_editor') }}</h4>
            <button class="np-more-header-action" @click="openLyricMatch('editor')">
              <span class="material-symbols-rounded">manage_search</span>
              {{ t('player.lyrics_match_action') }}
            </button>
          </div>
          <div class="np-lyrics-editor">
            <div class="np-more-segmented">
              <button
                :class="{ active: lyricsEditorTab === 'original' }"
                @click="lyricsEditorTab = 'original'"
              >
                {{ t('player.lyrics_editor_original') }}
              </button>
              <button
                :class="{ active: lyricsEditorTab === 'translation' }"
                @click="lyricsEditorTab = 'translation'"
              >
                {{ t('player.lyrics_editor_translation') }}
              </button>
              <button
                :class="{ active: lyricsEditorTab === 'romanization' }"
                @click="lyricsEditorTab = 'romanization'"
              >
                {{ t('player.lyrics_editor_romanization') }}
              </button>
            </div>
            <textarea
              v-if="lyricsEditorTab === 'original'"
              v-model="lyricsEditorText"
              class="np-lyrics-textarea"
              :placeholder="t('player.lyrics_editor_placeholder')"
              spellcheck="false"
            />
            <textarea
              v-else-if="lyricsEditorTab === 'translation'"
              v-model="lyricsTranslationEditorText"
              class="np-lyrics-textarea"
              :placeholder="t('player.lyrics_translation_placeholder')"
              spellcheck="false"
            />
            <textarea
              v-else
              v-model="lyricsRomanizationEditorText"
              class="np-lyrics-textarea"
              :placeholder="t('player.lyrics_romanization_placeholder')"
              spellcheck="false"
            />
            <div class="np-more-form-actions">
              <button class="np-more-form-btn primary" @click="applyLyricsFromEditor">
                <span class="material-symbols-rounded">check</span>
                {{ t('player.lyrics_apply') }}
              </button>
              <button class="np-more-form-btn" @click="lyricsEditorText = ''; lyricsTranslationEditorText = ''; lyricsRomanizationEditorText = ''; applyLyricsFromEditor()">
                <span class="material-symbols-rounded">clear_all</span>
                {{ t('player.lyrics_clear') }}
              </button>
            </div>
          </div>
        </template>

        <!-- 子视图：歌词匹配（对齐 Android 歌词编辑器「匹配」；从菜单进来时直接应用） -->
        <template v-else-if="moreSheetView === 'lyrics-fill'">
          <div class="np-more-sub-header">
            <button class="np-more-back" @click="leaveLyricMatch()">
              <span class="material-symbols-rounded">arrow_back</span>
            </button>
            <h4 class="np-more-title">{{ t(lyricMatchTarget === 'editor' ? 'player.lyrics_match' : 'player.lyrics_fill') }}</h4>
          </div>
          <LyricsMatchPanel
            v-if="player.currentTrack"
            :key="`${lyricMatchSession}:${player.currentTrack.id}`"
            :title="player.currentTrack.title"
            :artist="player.currentTrack.artist"
            :album="player.currentTrack.album"
            :duration-ms="player.currentTrack.durationMs || 0"
            :playback-source="currentSource"
            :prefer-word-timed="settings.preferWordTimedLyrics"
            :apply-label="t(lyricMatchTarget === 'editor' ? 'player.lyrics_match_fill_editor' : 'player.lyrics_match_apply')"
            @pick="onLyricMatchPicked"
          />
        </template>

        </div>
        </Transition>

      </div>
    </div>
    </Transition>
  </Teleport>
</template>
