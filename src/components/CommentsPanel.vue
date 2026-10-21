<script setup lang="ts">
import { writeClipboardText } from '@/utils/clipboard'
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { invoke } from '@tauri-apps/api/core'
import type { TrackInfo } from '@/stores/player'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'
import { resolveCommentTarget, type CommentTarget } from '@/modules/playback/commentTarget'
import { useEscapeClose } from '@/composables/useEscapeClose'
import type { ContextMenuActionItem } from '@/utils/contextMenu'
import BilibiliCoverImage from '@/components/BilibiliCoverImage.vue'
import ContextMenu from '@/components/ui/ContextMenu.vue'

/// 评论（对齐 Android CommentSheet）：网易云歌曲评论与 B 站视频评论，YouTube / 本地没有评论入口
const props = defineProps<{ open: boolean; track: TrackInfo | null }>()
const emit = defineEmits<{ 'update:open': [open: boolean] }>()
const { t, locale } = useI18n()
const auth = useAuthStore()
const toast = useToastStore()

interface CommentQuote { user_name: string; content: string | null }
interface CommentEmote { placeholder: string; url: string }
interface CommentImage { url: string; width: number; height: number }

interface CommentItem {
  id: string
  user_name: string
  avatar_url: string | null
  user_level: number | null
  content: string
  time_ms: number
  like_count: number
  liked: boolean
  reply_count: number | null
  quotes: CommentQuote[]
  preview_replies: CommentItem[]
  emotes: CommentEmote[]
  images: CommentImage[]
}

interface CommentPage {
  items: CommentItem[]
  total: number
  has_more: boolean
  cursor: string | null
}

type FailureReason = 'permission' | 'not_found' | 'closed' | 'server' | 'verification' | 'api'

interface CommentActionResult {
  ok: boolean
  code: number | null
  reason: FailureReason | null
}

type CommentSort = 'hot' | 'newest' | 'recommended'

interface ReplyThread {
  items: CommentItem[]
  cursor: string | null
  hasMore: boolean
  loading: boolean
  expanded: boolean
  failed: boolean
}

interface ReplyTarget {
  commentId: string
  rootId: string
  userName: string
}

type SendStatus =
  | { kind: 'success' }
  | { kind: 'uncertain' }
  | { kind: 'failure'; reason: FailureReason | null; code: number | null }

const PAGE_SIZE = 20
/** 各平台单条评论长度上限（对齐 Android commentLengthLimit） */
const LENGTH_LIMIT: Record<CommentTarget['platform'], number> = { netease: 140, bilibili: 1000 }

const target = computed(() => resolveCommentTarget(props.track))
const sorts = computed<CommentSort[]>(() => target.value?.platform === 'netease' ? ['hot', 'newest', 'recommended'] : ['hot', 'newest'])
const sort = ref<CommentSort>('hot')
/** 切换排序时新结果出来前保留旧列表（对齐 Android pendingSort），失败可继续浏览 */
const pendingSort = ref<CommentSort | null>(null)
const items = ref<CommentItem[]>([])
const total = ref(0)
const hasMore = ref(false)
const loading = ref(false)
const refreshing = ref(false)
const loadingMore = ref(false)
const loadMoreFailed = ref(false)
const failed = ref(false)
const threads = ref<Record<string, ReplyThread>>({})
const likingIds = ref<Set<string>>(new Set())
let page = 1
let cursor: string | null = null
let requestSeq = 0
let loadedKey = ''

const draft = ref('')
const replyTarget = ref<ReplyTarget | null>(null)
const sending = ref(false)
const sendStatus = ref<SendStatus | null>(null)
const draftInput = ref<HTMLTextAreaElement | null>(null)
let draftTargetKey = ''

const bodyEl = ref<HTMLElement | null>(null)

function targetKey(): string {
  return target.value ? `${target.value.platform}:${target.value.target}` : ''
}

const lengthLimit = computed(() => target.value ? LENGTH_LIMIT[target.value.platform] : 0)
const draftLength = computed(() => draft.value.length)
const overLimit = computed(() => draftLength.value > lengthLimit.value)
const loggedIn = computed(() => {
  if (!target.value) return false
  return target.value.platform === 'netease' ? auth.netease.loggedIn : auth.bilibili.loggedIn
})
const busy = computed(() => loading.value || refreshing.value || loadingMore.value || pendingSort.value !== null)
const canSend = computed(() =>
  !!target.value && loggedIn.value && !sending.value && !busy.value && !failed.value
  && draft.value.trim().length > 0 && !overLimit.value,
)
const likeEnabled = computed(() => !busy.value && !sending.value)

const isOpen = () => props.open
useEscapeClose(isOpen, () => emit('update:open', false))

// ---------- 加载 ----------

async function loadFirstPage(nextSort: CommentSort, mode: 'reset' | 'sort' | 'refresh') {
  const current = target.value
  if (!current) return
  const requestId = ++requestSeq
  failed.value = false
  loadMoreFailed.value = false
  loadingMore.value = false
  if (mode === 'reset') {
    items.value = []
    threads.value = {}
    total.value = 0
    hasMore.value = false
    loading.value = true
  } else if (mode === 'sort') {
    pendingSort.value = nextSort
  } else {
    refreshing.value = true
  }
  try {
    const result = await invoke<CommentPage>('get_comments', { ...current, sort: nextSort, page: 1, cursor: null })
    if (requestId !== requestSeq) return
    sort.value = nextSort
    items.value = dedupe(result.items)
    threads.value = {}
    total.value = result.total
    hasMore.value = result.has_more
    page = 1
    cursor = result.cursor
    loadedKey = `${targetKey()}:${nextSort}`
    if (mode !== 'refresh') void nextTick(() => bodyEl.value?.scrollTo({ top: 0 }))
  } catch {
    if (requestId !== requestSeq) return
    if (mode === 'reset' || !items.value.length) failed.value = true
    else toast.error(t('player.comments_reload_failed'))
  } finally {
    if (requestId === requestSeq) {
      loading.value = false
      refreshing.value = false
      pendingSort.value = null
    }
  }
}

async function loadMore() {
  const current = target.value
  if (!current || !hasMore.value || busy.value || loadMoreFailed.value) return
  const requestId = requestSeq
  loadingMore.value = true
  try {
    const result = await invoke<CommentPage>('get_comments', { ...current, sort: sort.value, page: page + 1, cursor })
    if (requestId !== requestSeq) return
    page += 1
    cursor = result.cursor
    const seen = new Set(items.value.map(item => item.id))
    items.value = [...items.value, ...result.items.filter(item => !seen.has(item.id))]
    hasMore.value = result.has_more
  } catch {
    if (requestId === requestSeq) loadMoreFailed.value = true
  } finally {
    if (requestId === requestSeq) loadingMore.value = false
  }
}

function retryLoadMore() {
  loadMoreFailed.value = false
  void loadMore()
}

function selectSort(option: CommentSort) {
  if (option === (pendingSort.value ?? sort.value) || sending.value) return
  void loadFirstPage(option, items.value.length ? 'sort' : 'reset')
}

function refresh() {
  if (busy.value || sending.value) return
  void loadFirstPage(sort.value, items.value.length ? 'refresh' : 'reset')
}

function dedupe(list: CommentItem[]): CommentItem[] {
  const seen = new Set<string>()
  return list.filter(item => !seen.has(item.id) && !!seen.add(item.id))
}

// ---------- 楼中楼 ----------

/** 预览回复已经覆盖全部回复时不需要「展开」，只有还有没露出来的回复才给入口 */
function hasHiddenReplies(comment: CommentItem): boolean {
  return (comment.reply_count ?? 0) > comment.preview_replies.length
}

function replyCountLabel(comment: CommentItem): string {
  return t('player.comments_replies', { count: formatCount(comment.reply_count ?? comment.preview_replies.length) })
}

/** 展开过的楼层显示完整楼中楼（收起时保留内容让收起动画有东西可收），否则平铺预览回复 */
function visibleReplies(comment: CommentItem): CommentItem[] {
  const thread = threads.value[comment.id]
  if (!thread) return comment.preview_replies
  return thread.items.length || !thread.loading ? thread.items : comment.preview_replies
}

function repliesOpen(comment: CommentItem): boolean {
  const thread = threads.value[comment.id]
  return thread ? thread.expanded : comment.preview_replies.length > 0
}

function updateThread(rootId: string, patch: Partial<ReplyThread>) {
  const thread = threads.value[rootId]
  if (thread) threads.value = { ...threads.value, [rootId]: { ...thread, ...patch } }
}

function toggleReplies(comment: CommentItem) {
  const thread = threads.value[comment.id]
  if (!thread) {
    void loadReplies(comment.id)
    return
  }
  updateThread(comment.id, { expanded: !thread.expanded })
}

async function loadReplies(rootId: string) {
  const current = target.value
  if (!current) return
  const thread = threads.value[rootId]
    ?? { items: [], cursor: null, hasMore: true, loading: false, expanded: true, failed: false }
  if (thread.loading || !thread.hasMore) return
  const requestId = requestSeq
  threads.value = { ...threads.value, [rootId]: { ...thread, expanded: true, loading: true, failed: false } }
  try {
    const result = await invoke<CommentPage>('get_comment_replies', { ...current, commentId: rootId, cursor: thread.cursor })
    if (requestId !== requestSeq) return
    const previous = threads.value[rootId]
    if (!previous) return
    const seen = new Set(previous.items.map(item => item.id))
    const merged = [...previous.items, ...result.items.filter(item => !seen.has(item.id))]
    updateThread(rootId, {
      items: merged,
      cursor: result.cursor,
      hasMore: result.has_more && !!result.cursor && merged.length > previous.items.length,
      loading: false,
    })
    // 楼中楼总数比列表里的回复数更新，用它回写，避免「N 条回复」与实际不符
    if (result.total) {
      items.value = items.value.map(item => item.id === rootId ? { ...item, reply_count: result.total } : item)
    }
  } catch {
    if (requestId === requestSeq) updateThread(rootId, { loading: false, failed: true })
  }
}

function retryReplies(rootId: string) {
  updateThread(rootId, { failed: false })
  void loadReplies(rootId)
}

// ---------- 点赞 ----------

function applyLike(list: CommentItem[], id: string, liked: boolean): CommentItem[] {
  return list.map((item) => {
    if (item.id === id) {
      return { ...item, liked, like_count: Math.max(0, item.like_count + (liked ? 1 : -1)) }
    }
    if (item.preview_replies.some(reply => reply.id === id)) {
      return { ...item, preview_replies: applyLike(item.preview_replies, id, liked) }
    }
    return item
  })
}

async function toggleLike(comment: CommentItem) {
  const current = target.value
  if (!current || !likeEnabled.value || likingIds.value.has(comment.id)) return
  if (!loggedIn.value) {
    toast.error(t('player.comments_like_login_required'))
    return
  }
  const liked = !comment.liked
  const requestId = requestSeq
  likingIds.value = new Set(likingIds.value).add(comment.id)
  try {
    const result = await invoke<CommentActionResult>('set_comment_liked', { ...current, commentId: comment.id, liked })
    if (requestId !== requestSeq) return
    if (result.ok) {
      items.value = applyLike(items.value, comment.id, liked)
      threads.value = Object.fromEntries(Object.entries(threads.value).map(([rootId, thread]) => [
        rootId,
        thread.items.some(item => item.id === comment.id) ? { ...thread, items: applyLike(thread.items, comment.id, liked) } : thread,
      ]))
    } else {
      toast.error(withCode(likeFailureText(result.reason), result.code))
    }
  } catch {
    if (requestId === requestSeq) toast.error(t('player.comments_like_failed'))
  } finally {
    const next = new Set(likingIds.value)
    next.delete(comment.id)
    likingIds.value = next
  }
}

function likeFailureText(reason: FailureReason | null): string {
  if (reason === 'permission') return t('player.comments_like_login_required')
  if (reason === 'verification') return t('player.comments_verification_failed')
  if (reason === 'closed') return t('player.comments_closed')
  return t('player.comments_like_failed')
}

// ---------- 发表 ----------

function startReply(comment: CommentItem, rootId: string) {
  if (sending.value) return
  replyTarget.value = { commentId: comment.id, rootId, userName: displayName(comment) }
  sendStatus.value = null
  void nextTick(() => draftInput.value?.focus())
}

function cancelReply() {
  if (sending.value) return
  replyTarget.value = null
  sendStatus.value = null
}

async function sendComment() {
  const current = target.value
  if (!current || !canSend.value) return
  const content = draft.value.trim()
  const reply = replyTarget.value
  const requestId = requestSeq
  sending.value = true
  sendStatus.value = null
  try {
    const result = await invoke<CommentActionResult>('send_comment', {
      ...current,
      content,
      rootId: reply?.rootId ?? null,
      replyToId: reply?.commentId ?? null,
    })
    if (requestId !== requestSeq) return
    if (!result.ok) {
      sendStatus.value = { kind: 'failure', reason: result.reason, code: result.code }
      return
    }
    draft.value = ''
    replyTarget.value = null
    sendStatus.value = { kind: 'success' }
    sending.value = false
    if (!reply) {
      // 一级评论按最新排序刷新才能马上看到自己发的（对齐 Android sendComment）
      if (sort.value === 'newest') refresh()
      else selectSort('newest')
    } else {
      const { [reply.rootId]: _, ...rest } = threads.value
      threads.value = rest
      void loadReplies(reply.rootId)
    }
  } catch {
    // 网络层失败时评论可能已经发出，保留草稿让用户刷新核对，不自动重发
    if (requestId === requestSeq) sendStatus.value = { kind: 'uncertain' }
  } finally {
    sending.value = false
  }
}

const sendStatusText = computed(() => {
  const status = sendStatus.value
  if (!status) return ''
  if (status.kind === 'success') return t('player.comments_send_success')
  if (status.kind === 'uncertain') return t('player.comments_send_uncertain')
  const message = status.reason === 'permission' ? t('player.comments_send_login_required')
    : status.reason === 'verification' ? t('player.comments_verification_failed')
      : status.reason === 'closed' ? t('player.comments_closed')
        : t('player.comments_send_failed')
  return withCode(message, status.code)
})

function onDraftKeydown(event: KeyboardEvent) {
  if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) {
    event.preventDefault()
    void sendComment()
  }
}

function onDraftInput() {
  if (sendStatus.value) sendStatus.value = null
  autoGrow()
}

function autoGrow() {
  const el = draftInput.value
  if (!el) return
  el.style.height = 'auto'
  el.style.height = `${Math.min(el.scrollHeight, 120)}px`
}

watch(draft, () => void nextTick(autoGrow))

function withCode(message: string, code: number | null | undefined): string {
  return code === null || code === undefined ? message : t('player.comments_error_code', { message, code })
}

// ---------- 正文 / 配图 ----------

type Segment = { kind: 'text'; text: string } | { kind: 'emote'; emote: CommentEmote }

/**
 * Splits a comment body on its emote placeholders (matching Android's
 * splitCommentContent): longer placeholders win and unmatched text is kept as-is.
 *
 * The template calls this per comment, and every page change replaces `items`, which
 * re-renders the whole column: without a cache each page rescanned every comment loaded so
 * far character by character (O(loaded comments squared)). A comment's body and emotes are
 * never mutated in place, so caching per comment object is safe.
 */
const segmentCache = new WeakMap<CommentItem, Segment[]>()
function segments(comment: CommentItem): Segment[] {
  const cached = segmentCache.get(comment)
  if (cached) return cached
  const candidates = [...new Map(comment.emotes.filter(emote => emote.placeholder).map(emote => [emote.placeholder, emote])).values()]
    .sort((a, b) => b.placeholder.length - a.placeholder.length)
  if (!candidates.length) {
    const plain: Segment[] = [{ kind: 'text', text: comment.content }]
    segmentCache.set(comment, plain)
    return plain
  }
  const result: Segment[] = []
  let pending = ''
  let index = 0
  const content = comment.content
  while (index < content.length) {
    const matched = candidates.find(emote => content.startsWith(emote.placeholder, index))
    if (!matched) {
      pending += content[index]
      index += 1
      continue
    }
    if (pending) result.push({ kind: 'text', text: pending })
    pending = ''
    result.push({ kind: 'emote', emote: matched })
    index += matched.placeholder.length
  }
  if (pending) result.push({ kind: 'text', text: pending })
  segmentCache.set(comment, result)
  return result
}

function singleImageStyle(image: CommentImage) {
  const ratio = image.width > 0 && image.height > 0 ? image.width / image.height : 1
  return { aspectRatio: String(Math.max(ratio, 0.6)) }
}

const preview = ref<{ images: CommentImage[]; index: number } | null>(null)
useEscapeClose(() => preview.value !== null, () => { preview.value = null })

function openPreview(images: CommentImage[], index: number) {
  preview.value = { images, index }
}

function stepPreview(delta: number) {
  const current = preview.value
  if (!current) return
  preview.value = { ...current, index: (current.index + delta + current.images.length) % current.images.length }
}

// ---------- 右键菜单 ----------

const menu = ref<{ open: boolean; x: number; y: number; comment: CommentItem | null; rootId: string }>({
  open: false, x: 0, y: 0, comment: null, rootId: '',
})
const menuItems = computed<ContextMenuActionItem[]>(() => [
  { id: 'copy', label: t('player.comments_copy'), icon: 'content_copy' },
  { id: 'reply', label: t('player.comments_reply'), icon: 'reply', disabled: !loggedIn.value || sending.value },
])

function openMenu(event: MouseEvent, comment: CommentItem, rootId: string) {
  event.preventDefault()
  menu.value = { open: true, x: event.clientX, y: event.clientY, comment, rootId }
}

async function onMenuClick(item: ContextMenuActionItem) {
  const comment = menu.value.comment
  if (!comment) return
  if (item.id === 'copy') {
    try {
      await writeClipboardText(comment.content)
      toast.success(t('player.comments_copied'))
    } catch {
      toast.error(t('player.comments_copy_failed'))
    }
  } else if (item.id === 'reply') {
    startReply(comment, menu.value.rootId)
  }
}

// ---------- 格式化 ----------

function displayName(comment: CommentItem): string {
  return comment.user_name.trim() || t('player.comments_anonymous')
}

const timeFormatter = computed(() => new Intl.DateTimeFormat(locale.value, { year: 'numeric', month: 'short', day: 'numeric' }))
function formatTime(ms: number): string {
  if (!ms) return ''
  const diff = Date.now() - ms
  if (diff < 60_000) return t('recent.just_now')
  if (diff < 3_600_000) return t('recent.minutes_ago', { count: Math.floor(diff / 60_000) })
  if (diff < 86_400_000) return t('recent.hours_ago', { count: Math.floor(diff / 3_600_000) })
  if (diff < 7 * 86_400_000) return t('recent.days_ago', { count: Math.floor(diff / 86_400_000) })
  return timeFormatter.value.format(new Date(ms))
}

const compactNumber = computed(() => new Intl.NumberFormat(locale.value, { notation: 'compact', maximumFractionDigits: 1 }))
function formatCount(value: number): string {
  return value > 0 ? compactNumber.value.format(value) : ''
}

// ---------- 生命周期 ----------

// 打开时才拉取；同一首、同一排序重新打开直接复用
watch(() => [props.open, targetKey()] as const, ([open, key]) => {
  if (key !== draftTargetKey) {
    draftTargetKey = key
    draft.value = ''
    replyTarget.value = null
    sendStatus.value = null
  }
  if (!open || !target.value) return
  if (!sorts.value.includes(sort.value)) sort.value = 'hot'
  if (`${key}:${sort.value}` !== loadedKey || failed.value) void loadFirstPage(sort.value, 'reset')
}, { immediate: true })

watch(() => props.open, (open) => {
  if (!open) {
    menu.value.open = false
    preview.value = null
  }
})

const sentinel = ref<HTMLElement | null>(null)
let observer: IntersectionObserver | null = null
watch(sentinel, (element) => {
  observer?.disconnect()
  if (!element) return
  observer = new IntersectionObserver((entries) => {
    if (entries.some(entry => entry.isIntersecting)) void loadMore()
  }, { root: bodyEl.value, rootMargin: '400px 0px' })
  observer.observe(element)
})
onBeforeUnmount(() => observer?.disconnect())

/** 首屏 / 翻页新进来的评论按批内序号错开入场，翻页时不会整页一起闪 */
function enterDelay(index: number): string {
  return `${Math.min(index % PAGE_SIZE, 8) * 28}ms`
}
</script>

<template>
  <Teleport to="body">
    <Transition name="comments-sheet">
      <div v-if="open" class="comments-overlay" @click.self="emit('update:open', false)">
        <aside class="comments-panel" role="dialog" :aria-label="t('player.comments')">
          <header class="comments-header">
            <div class="comments-heading">
              <h3>{{ t('player.comments') }}</h3>
              <Transition name="comments-fade">
                <span v-if="total" :key="total" class="comments-total">{{ t('player.comments_total', { count: formatCount(total) || total }) }}</span>
              </Transition>
            </div>
            <button
              v-if="target"
              class="comments-icon-btn"
              :class="{ spinning: refreshing }"
              :disabled="busy || sending"
              :title="t('player.comments_refresh')"
              :aria-label="t('player.comments_refresh')"
              @click="refresh"
            >
              <span class="material-symbols-rounded">refresh</span>
            </button>
            <button class="comments-icon-btn" :aria-label="t('common.close')" @click="emit('update:open', false)">
              <span class="material-symbols-rounded">close</span>
            </button>
          </header>
          <p v-if="track" class="comments-track">{{ track.title }}<span v-if="track.artist"> · {{ track.artist }}</span></p>

          <div v-if="target" class="comments-sorts" role="tablist">
            <button
              v-for="option in sorts"
              :key="option"
              role="tab"
              :aria-selected="(pendingSort ?? sort) === option"
              class="comments-sort"
              :class="{ active: (pendingSort ?? sort) === option }"
              :disabled="sending"
              @click="selectSort(option)"
            >
              <span v-if="pendingSort === option" class="material-symbols-rounded spinning small">progress_activity</span>
              {{ t(`player.comments_${option}`) }}
            </button>
          </div>
          <div class="comments-progress" :class="{ active: pendingSort !== null || refreshing }" />

          <div ref="bodyEl" class="comments-body" :class="{ dimmed: pendingSort !== null }">
            <Transition name="comments-fade" mode="out-in">
              <div v-if="!target" key="unavailable" class="comments-state">
                <span class="material-symbols-rounded">comments_disabled</span>
                <p>{{ t('player.comments_unavailable') }}</p>
              </div>
              <div v-else-if="loading" key="loading" class="comments-state">
                <span class="material-symbols-rounded spinning">progress_activity</span>
              </div>
              <div v-else-if="failed" key="failed" class="comments-state">
                <span class="material-symbols-rounded">wifi_off</span>
                <p>{{ t('player.comments_failed') }}</p>
                <button class="comments-pill-btn" @click="loadFirstPage(sort, 'reset')">{{ t('player.retry') }}</button>
              </div>
              <div v-else-if="!items.length" key="empty" class="comments-state">
                <span class="material-symbols-rounded">chat_bubble</span>
                <p>{{ t('player.comments_empty') }}</p>
              </div>
              <div v-else :key="`list:${loadedKey}`" class="comments-list">
                <TransitionGroup name="comment" appear>
                  <article
                    v-for="(comment, index) in items"
                    :key="comment.id"
                    class="comment"
                    :style="{ '--enter-delay': enterDelay(index) }"
                    @contextmenu="openMenu($event, comment, comment.id)"
                  >
                    <div class="comment-head">
                      <div class="comment-avatar">
                        <BilibiliCoverImage v-if="comment.avatar_url" :src="comment.avatar_url" loading="lazy">
                          <span class="material-symbols-rounded filled">person</span>
                        </BilibiliCoverImage>
                        <span v-else class="material-symbols-rounded filled">person</span>
                      </div>
                      <div class="comment-who">
                        <span class="comment-user">{{ displayName(comment) }}</span>
                        <span v-if="comment.time_ms" class="comment-time">{{ formatTime(comment.time_ms) }}</span>
                      </div>
                      <span v-if="comment.user_level" class="comment-level">{{ t('player.comments_level', { level: comment.user_level }) }}</span>
                      <span class="comment-floor">{{ t('player.comments_floor', { floor: index + 1 }) }}</span>
                    </div>

                    <p class="comment-content"><template v-for="(segment, segmentIndex) in segments(comment)" :key="segmentIndex"><img
                      v-if="segment.kind === 'emote'"
                      class="comment-emote"
                      :src="segment.emote.url"
                      :alt="segment.emote.placeholder"
                      :title="segment.emote.placeholder"
                      referrerpolicy="no-referrer"
                      loading="lazy"
                    ><template v-else>{{ segment.text }}</template></template></p>

                    <div
                      v-if="comment.images.length"
                      class="comment-images"
                      :class="{ single: comment.images.length === 1 }"
                    >
                      <button
                        v-for="(image, imageIndex) in comment.images.slice(0, 9)"
                        :key="image.url"
                        class="comment-image"
                        :style="comment.images.length === 1 ? singleImageStyle(image) : undefined"
                        :aria-label="t('player.comments_image')"
                        @click="openPreview(comment.images, imageIndex)"
                      >
                        <BilibiliCoverImage :src="image.url" loading="lazy" />
                      </button>
                    </div>

                    <div v-if="comment.quotes.length" class="comment-quotes">
                      <div v-for="(quote, quoteIndex) in comment.quotes" :key="quoteIndex" class="comment-quote">
                        <span class="comment-quote-who">{{ t('player.comments_reply_to', { name: quote.user_name || t('player.comments_anonymous') }) }}</span>
                        <span class="comment-quote-text">{{ quote.content ?? t('player.comments_deleted') }}</span>
                      </div>
                    </div>

                    <div class="replies-collapse" :class="{ open: repliesOpen(comment) }">
                      <div class="replies-inner">
                        <TransitionGroup name="reply" tag="div" class="comment-replies">
                          <div
                            v-for="reply in visibleReplies(comment)"
                            :key="reply.id"
                            class="reply"
                            @contextmenu.stop="openMenu($event, reply, comment.id)"
                          >
                            <div class="reply-avatar">
                              <BilibiliCoverImage v-if="reply.avatar_url" :src="reply.avatar_url" loading="lazy">
                                <span class="material-symbols-rounded filled">person</span>
                              </BilibiliCoverImage>
                              <span v-else class="material-symbols-rounded filled">person</span>
                            </div>
                            <div class="reply-main">
                              <div class="reply-meta">
                                <span class="reply-user">{{ displayName(reply) }}</span>
                                <span v-if="reply.time_ms" class="comment-time">{{ formatTime(reply.time_ms) }}</span>
                              </div>
                              <p class="reply-content"><template v-for="(segment, segmentIndex) in segments(reply)" :key="segmentIndex"><img
                                v-if="segment.kind === 'emote'"
                                class="comment-emote"
                                :src="segment.emote.url"
                                :alt="segment.emote.placeholder"
                                :title="segment.emote.placeholder"
                                referrerpolicy="no-referrer"
                                loading="lazy"
                              ><template v-else>{{ segment.text }}</template></template></p>
                              <div v-if="reply.images.length" class="comment-images small">
                                <button
                                  v-for="(image, imageIndex) in reply.images.slice(0, 9)"
                                  :key="image.url"
                                  class="comment-image"
                                  :aria-label="t('player.comments_image')"
                                  @click="openPreview(reply.images, imageIndex)"
                                >
                                  <BilibiliCoverImage :src="image.url" loading="lazy" />
                                </button>
                              </div>
                              <div v-if="reply.quotes.length" class="comment-quotes">
                                <div v-for="(quote, quoteIndex) in reply.quotes" :key="quoteIndex" class="comment-quote">
                                  <span class="comment-quote-who">{{ t('player.comments_reply_to', { name: quote.user_name || t('player.comments_anonymous') }) }}</span>
                                  <span class="comment-quote-text">{{ quote.content ?? t('player.comments_deleted') }}</span>
                                </div>
                              </div>
                              <div class="reply-actions">
                                <button class="comment-text-btn" :disabled="!loggedIn || sending" @click="startReply(reply, comment.id)">
                                  {{ t('player.comments_reply') }}
                                </button>
                                <button
                                  class="comment-like small"
                                  :class="{ liked: reply.liked }"
                                  :disabled="!likeEnabled || likingIds.has(reply.id)"
                                  :aria-pressed="reply.liked"
                                  :aria-label="t(reply.liked ? 'player.comments_unlike' : 'player.comments_like')"
                                  @click="toggleLike(reply)"
                                >
                                  <span v-if="likingIds.has(reply.id)" class="material-symbols-rounded spinning">progress_activity</span>
                                  <Transition v-else name="like-pop" mode="out-in">
                                    <span :key="String(reply.liked)" class="material-symbols-rounded" :class="{ filled: reply.liked }">thumb_up</span>
                                  </Transition>
                                  <span class="comment-like-count">{{ formatCount(reply.like_count) }}</span>
                                </button>
                              </div>
                            </div>
                          </div>
                        </TransitionGroup>
                        <div v-if="threads[comment.id]" class="replies-footer">
                          <span v-if="threads[comment.id].loading" class="material-symbols-rounded spinning small">progress_activity</span>
                          <button v-else-if="threads[comment.id].failed" class="comment-text-btn" @click="retryReplies(comment.id)">
                            {{ t('player.comments_replies_failed') }} · {{ t('player.retry') }}
                          </button>
                          <button v-else-if="threads[comment.id].hasMore" class="comment-text-btn" @click="loadReplies(comment.id)">
                            {{ t('player.comments_more_replies') }}
                          </button>
                          <span v-else-if="!threads[comment.id].items.length" class="replies-empty">{{ t('player.comments_empty_replies') }}</span>
                        </div>
                      </div>
                    </div>

                    <div class="comment-actions">
                      <button class="comment-chip-btn" :disabled="!loggedIn || sending" @click="startReply(comment, comment.id)">
                        <span class="material-symbols-rounded">reply</span>{{ t('player.comments_reply') }}
                      </button>
                      <button
                        v-if="threads[comment.id] || hasHiddenReplies(comment)"
                        class="comment-chip-btn"
                        :disabled="threads[comment.id]?.loading && !threads[comment.id]?.items.length"
                        @click="toggleReplies(comment)"
                      >
                        <span class="material-symbols-rounded chevron" :class="{ flipped: threads[comment.id]?.expanded }">expand_more</span>
                        {{ threads[comment.id]?.expanded ? t('player.comments_hide_replies') : replyCountLabel(comment) }}
                      </button>
                      <button
                        class="comment-like"
                        :class="{ liked: comment.liked }"
                        :disabled="!likeEnabled || likingIds.has(comment.id)"
                        :aria-pressed="comment.liked"
                        :aria-label="t(comment.liked ? 'player.comments_unlike' : 'player.comments_like')"
                        @click="toggleLike(comment)"
                      >
                        <span v-if="likingIds.has(comment.id)" class="material-symbols-rounded spinning">progress_activity</span>
                        <Transition v-else name="like-pop" mode="out-in">
                          <span :key="String(comment.liked)" class="material-symbols-rounded" :class="{ filled: comment.liked }">thumb_up</span>
                        </Transition>
                        <span class="comment-like-count">{{ formatCount(comment.like_count) }}</span>
                      </button>
                    </div>
                  </article>
                </TransitionGroup>
                <div v-if="hasMore" ref="sentinel" class="comments-more">
                  <span v-if="loadingMore" class="material-symbols-rounded spinning small">progress_activity</span>
                  <button v-else-if="loadMoreFailed" class="comments-pill-btn" @click="retryLoadMore">{{ t('player.retry') }}</button>
                </div>
                <p v-else class="comments-end">{{ t('player.comments_no_more') }}</p>
              </div>
            </Transition>
          </div>

          <footer v-if="target" class="comments-composer">
            <Transition name="composer-row">
              <div v-if="replyTarget" class="composer-reply">
                <span class="material-symbols-rounded">reply</span>
                <span class="composer-reply-who">{{ t('player.comments_reply_to', { name: replyTarget.userName }) }}</span>
                <button class="comments-icon-btn small" :disabled="sending" :aria-label="t('player.comments_cancel_reply')" @click="cancelReply">
                  <span class="material-symbols-rounded">close</span>
                </button>
              </div>
            </Transition>
            <div class="composer-row" :class="{ error: overLimit, disabled: !loggedIn }">
              <textarea
                ref="draftInput"
                v-model="draft"
                class="composer-input"
                rows="1"
                :disabled="sending || !loggedIn"
                :placeholder="loggedIn ? (replyTarget ? t('player.comments_reply_to', { name: replyTarget.userName }) : t('player.comments_write_hint')) : t('player.comments_login_hint')"
                @keydown="onDraftKeydown"
                @input="onDraftInput"
              />
              <span v-if="draftLength" class="composer-count" :class="{ error: overLimit }">{{ draftLength }}/{{ lengthLimit }}</span>
              <button
                class="composer-send"
                :disabled="!canSend"
                :aria-label="t('player.comments_send')"
                :title="t('player.comments_send')"
                @click="sendComment"
              >
                <span v-if="sending" class="material-symbols-rounded spinning">progress_activity</span>
                <span v-else class="material-symbols-rounded filled">send</span>
              </button>
            </div>
            <Transition name="comments-fade">
              <p
                v-if="sendStatusText"
                :key="sendStatusText"
                class="composer-status"
                :class="{ error: sendStatus?.kind !== 'success' }"
              >{{ sendStatusText }}</p>
            </Transition>
          </footer>

          <Transition name="comments-fade">
            <div v-if="preview" class="comments-lightbox" @click.self="preview = null">
              <button class="comments-icon-btn lightbox-close" :aria-label="t('common.close')" @click="preview = null">
                <span class="material-symbols-rounded">close</span>
              </button>
              <button v-if="preview.images.length > 1" class="comments-icon-btn lightbox-nav prev" @click="stepPreview(-1)">
                <span class="material-symbols-rounded">chevron_left</span>
              </button>
              <Transition name="comments-fade" mode="out-in">
                <BilibiliCoverImage :key="preview.images[preview.index].url" :src="preview.images[preview.index].url" class="lightbox-image" />
              </Transition>
              <button v-if="preview.images.length > 1" class="comments-icon-btn lightbox-nav next" @click="stepPreview(1)">
                <span class="material-symbols-rounded">chevron_right</span>
              </button>
              <span v-if="preview.images.length > 1" class="lightbox-index">{{ preview.index + 1 }} / {{ preview.images.length }}</span>
            </div>
          </Transition>
        </aside>
      </div>
    </Transition>
  </Teleport>
  <ContextMenu v-model:open="menu.open" :x="menu.x" :y="menu.y" :items="menuItems" @click="onMenuClick" />
</template>

<style scoped lang="scss">
.comments-overlay {
  position: fixed;
  inset: 0;
  z-index: 260;
  display: flex;
  justify-content: flex-end;
  background: rgb(0 0 0 / 0.4);
}

.comments-panel {
  position: relative;
  width: 460px;
  max-width: 94vw;
  height: 100%;
  display: flex;
  flex-direction: column;
  padding-top: var(--titlebar-height, 36px);
  background: var(--bg-solid-container, var(--md-surface-container));
  color: var(--md-on-surface);
  box-shadow: -4px 0 24px rgb(0 0 0 / 0.3);
}

/* 开合都走 transition：半路关掉会从当前位置反向收回，不会闪现 */
.comments-sheet-enter-active,
.comments-sheet-leave-active {
  transition: background-color 280ms var(--ease-standard);
  .comments-panel { transition: transform 320ms var(--ease-emphasized-decel); }
}
.comments-sheet-leave-active .comments-panel { transition-timing-function: var(--ease-emphasized-accel); transition-duration: 220ms; }
.comments-sheet-enter-from,
.comments-sheet-leave-to {
  background-color: transparent;
  .comments-panel { transform: translateX(100%); }
}

.comments-header {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 14px 12px 4px 20px;
}

.comments-heading {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: baseline;
  gap: 10px;
  h3 { font-size: 20px; font-weight: 600; }
}

.comments-total { font-size: 12px; color: var(--md-on-surface-variant); font-variant-numeric: tabular-nums; }

.comments-icon-btn {
  width: 36px;
  height: 36px;
  flex-shrink: 0;
  border-radius: 50%;
  display: grid;
  place-items: center;
  color: var(--md-on-surface-variant);
  transition: background var(--duration-short) var(--ease-standard);
  &:hover:not(:disabled) { background: var(--md-surface-container-highest); }
  &:disabled { opacity: 0.5; }
  &.small { width: 28px; height: 28px; .material-symbols-rounded { font-size: 18px; } }
  &.spinning .material-symbols-rounded { animation: comments-spin 1s linear infinite; }
}

.comments-track {
  padding: 0 20px;
  font-size: 12px;
  color: var(--md-on-surface-variant);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.comments-sorts {
  display: flex;
  gap: 8px;
  padding: 12px 20px 10px;
}

.comments-sort {
  height: 32px;
  padding: 0 14px;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--md-outline-variant);
  font-size: 13px;
  color: var(--md-on-surface-variant);
  transition: background var(--duration-short) var(--ease-standard), color var(--duration-short) var(--ease-standard),
    border-color var(--duration-short) var(--ease-standard);
  &:hover:not(.active) { background: var(--md-surface-container-high); }
  &.active {
    background: var(--md-secondary-container);
    color: var(--md-on-secondary-container);
    border-color: transparent;
  }
}

/* 切换排序 / 刷新时的顶部进度条（对齐 Android LinearProgressIndicator） */
.comments-progress {
  position: relative;
  height: 2px;
  overflow: hidden;
  background: var(--md-outline-variant);
  &::after {
    content: '';
    position: absolute;
    inset: 0 auto 0 0;
    width: 40%;
    background: var(--md-primary);
    opacity: 0;
    transform: translateX(-100%);
    transition: opacity 200ms var(--ease-standard);
  }
  &.active::after {
    opacity: 1;
    animation: comments-progress 1.1s cubic-bezier(0.4, 0, 0.2, 1) infinite;
  }
}

.comments-body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 12px 14px 16px;
  overscroll-behavior: contain;
  transition: opacity 200ms var(--ease-standard);
  &.dimmed { opacity: 0.55; pointer-events: none; }
}

.comments-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.comments-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
  padding: 64px 16px;
  color: var(--md-on-surface-variant);
  text-align: center;
  > .material-symbols-rounded { font-size: 36px; opacity: 0.5; }
  p { font-size: 13px; }
}

.comments-pill-btn {
  padding: 6px 18px;
  border: 1px solid var(--md-outline-variant);
  border-radius: var(--radius-full);
  color: var(--md-primary);
  font-size: 13px;
  &:hover { background: var(--md-surface-container-high); }
}

/* 一级评论卡片（对齐 Android CommentItem：extraLarge 圆角 + surfaceContainer 底） */
.comment {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 14px 16px 10px;
  border-radius: 20px;
  background: var(--md-surface-container-high);
}

.comment-head {
  display: flex;
  align-items: center;
  gap: 10px;
}

.comment-avatar,
.reply-avatar {
  flex-shrink: 0;
  border-radius: 50%;
  overflow: hidden;
  display: grid;
  place-items: center;
  background: var(--md-surface-container-highest);
  color: var(--md-on-surface-variant);
  :deep(img) { width: 100%; height: 100%; object-fit: cover; }
}

.comment-avatar {
  width: 38px;
  height: 38px;
  .material-symbols-rounded { font-size: 20px; }
}

.comment-who {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.comment-user,
.reply-user {
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.comment-user { font-size: 14px; }

.comment-time {
  font-size: 11px;
  color: var(--md-on-surface-variant);
  flex-shrink: 0;
}

.comment-level {
  flex-shrink: 0;
  padding: 2px 6px;
  border-radius: 6px;
  font-size: 10px;
  font-weight: 600;
  background: var(--md-secondary-container);
  color: var(--md-on-secondary-container);
}

.comment-floor {
  flex-shrink: 0;
  font-size: 11px;
  color: var(--md-on-surface-variant);
  font-variant-numeric: tabular-nums;
}

.comment-content,
.reply-content {
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  user-select: text;
}

.comment-content { font-size: 14px; line-height: 1.65; }

.comment-emote {
  display: inline-block;
  width: 1.4em;
  height: 1.4em;
  margin: 0 1px;
  vertical-align: -0.32em;
  object-fit: contain;
}

/* 配图：单图按原比例（长图收敛到 220px 高并裁剪），多图 3 列方格（对齐 Android CommentImageGrid） */
.comment-images {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 4px;
  &.single {
    grid-template-columns: minmax(0, 72%);
    .comment-image { aspect-ratio: auto; max-height: 220px; }
  }
  &.small { grid-template-columns: repeat(3, minmax(0, 72px)); }
}

.comment-image {
  aspect-ratio: 1;
  border-radius: 10px;
  overflow: hidden;
  background: var(--md-surface-container-highest);
  cursor: zoom-in;
  transition: transform 160ms var(--ease-standard), filter 160ms var(--ease-standard);
  &:hover { filter: brightness(1.06); }
  &:active { transform: scale(0.98); }
  :deep(img) { width: 100%; height: 100%; object-fit: cover; display: block; }
}

.comment-quotes {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 10px 12px;
  border-radius: 14px;
  background: var(--md-surface-container-highest);
}

.comment-quote {
  display: flex;
  flex-direction: column;
  gap: 2px;
  font-size: 12px;
  line-height: 1.5;
}

.comment-quote-who { font-weight: 600; color: var(--md-primary); }
.comment-quote-text { color: var(--md-on-surface-variant); white-space: pre-wrap; overflow-wrap: anywhere; }

/* 楼中楼开合：grid 行高 0fr <-> 1fr，展开 / 收起都能平滑过渡且不用量高度 */
.replies-collapse {
  display: grid;
  grid-template-rows: 0fr;
  opacity: 0;
  margin-top: -10px;
  transition: grid-template-rows 320ms var(--ease-emphasized-decel), opacity 220ms var(--ease-standard),
    margin-top 320ms var(--ease-emphasized-decel);
  &.open {
    grid-template-rows: 1fr;
    opacity: 1;
    margin-top: 0;
  }
}

.replies-inner {
  min-height: 0;
  overflow: hidden;
}

.comment-replies {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.reply {
  display: flex;
  gap: 8px;
  padding: 10px 12px 6px;
  border-radius: 14px;
  background: var(--md-surface-container-highest);
}

.reply-avatar {
  width: 24px;
  height: 24px;
  .material-symbols-rounded { font-size: 16px; }
}

.reply-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.reply-meta {
  display: flex;
  align-items: baseline;
  gap: 8px;
  min-width: 0;
}

.reply-user { font-size: 12px; color: var(--md-primary); }
.reply-content { font-size: 13px; line-height: 1.55; }

.reply-actions {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 4px;
}

.replies-footer {
  display: flex;
  justify-content: center;
  padding-top: 6px;
  &:empty { display: none; }
}

.replies-empty { font-size: 12px; color: var(--md-on-surface-variant); }

.comment-actions {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 6px;
}

.comment-chip-btn,
.comment-like {
  height: 32px;
  padding: 0 12px;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  border-radius: var(--radius-full);
  font-size: 12px;
  font-weight: 600;
  background: var(--md-surface-container-highest);
  transition: background 160ms var(--ease-standard), color 160ms var(--ease-standard), opacity 160ms var(--ease-standard);
  .material-symbols-rounded { font-size: 17px; }
  &:disabled { opacity: 0.5; }
}

.comment-chip-btn {
  color: var(--md-primary);
  &:hover:not(:disabled) { background: color-mix(in srgb, var(--md-primary) 14%, var(--md-surface-container-highest)); }
  .chevron { transition: transform 260ms var(--ease-emphasized-decel); }
  .chevron.flipped { transform: rotate(180deg); }
}

.comment-like {
  min-width: 52px;
  justify-content: center;
  color: var(--md-on-surface-variant);
  font-variant-numeric: tabular-nums;
  &:hover:not(:disabled) { background: color-mix(in srgb, var(--md-on-surface) 8%, var(--md-surface-container-highest)); }
  &.liked {
    background: var(--md-primary-container);
    color: var(--md-on-primary-container);
  }
  &.small {
    height: 26px;
    min-width: 44px;
    padding: 0 8px;
    background: transparent;
    .material-symbols-rounded { font-size: 15px; }
    &.liked { background: var(--md-primary-container); }
  }
  .comment-like-count:empty { display: none; }
}

.comment-text-btn {
  padding: 2px 8px;
  border-radius: var(--radius-full);
  font-size: 12px;
  font-weight: 600;
  color: var(--md-primary);
  &:hover:not(:disabled) { background: color-mix(in srgb, var(--md-primary) 10%, transparent); }
  &:disabled { opacity: 0.5; }
}

.comments-more {
  display: flex;
  justify-content: center;
  min-height: 48px;
  padding: 8px 0;
  color: var(--md-on-surface-variant);
}

.comments-end {
  padding: 16px 0 4px;
  text-align: center;
  font-size: 12px;
  color: var(--md-on-surface-variant);
}

/* 底部输入框（对齐 Android CommentComposer） */
.comments-composer {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 10px 14px 14px;
  border-top: 1px solid var(--md-outline-variant);
  background: var(--bg-solid-container, var(--md-surface-container));
}

.composer-reply {
  display: flex;
  align-items: center;
  gap: 6px;
  padding-left: 6px;
  font-size: 12px;
  color: var(--md-primary);
  > .material-symbols-rounded { font-size: 16px; }
}

.composer-reply-who {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 600;
}

.composer-row {
  display: flex;
  align-items: flex-end;
  gap: 8px;
  padding: 6px 6px 6px 16px;
  border-radius: 22px;
  border: 1px solid var(--md-outline-variant);
  background: var(--md-surface-container-high);
  transition: border-color 160ms var(--ease-standard), box-shadow 160ms var(--ease-standard);
  &:focus-within {
    border-color: var(--md-primary);
    box-shadow: 0 0 0 1px var(--md-primary);
  }
  &.error,
  &.error:focus-within {
    border-color: var(--md-error);
    box-shadow: 0 0 0 1px var(--md-error);
  }
  &.disabled { opacity: 0.7; }
}

.composer-input {
  flex: 1;
  min-width: 0;
  min-height: 22px;
  max-height: 120px;
  margin: 7px 0;
  padding: 0;
  border: 0;
  outline: none;
  resize: none;
  background: transparent;
  color: var(--md-on-surface);
  font: inherit;
  font-size: 14px;
  line-height: 22px;
  &::placeholder { color: var(--md-on-surface-variant); }
}

.composer-count {
  align-self: center;
  font-size: 11px;
  color: var(--md-on-surface-variant);
  font-variant-numeric: tabular-nums;
  &.error { color: var(--md-error); font-weight: 600; }
}

.composer-send {
  width: 36px;
  height: 36px;
  flex-shrink: 0;
  border-radius: 50%;
  display: grid;
  place-items: center;
  background: var(--md-primary);
  color: var(--md-on-primary);
  transition: opacity 160ms var(--ease-standard), transform 160ms var(--ease-standard), background 160ms var(--ease-standard);
  .material-symbols-rounded { font-size: 18px; }
  &:hover:not(:disabled) { transform: scale(1.05); }
  &:active:not(:disabled) { transform: scale(0.95); }
  &:disabled {
    background: var(--md-surface-container-highest);
    color: var(--md-on-surface-variant);
  }
}

.composer-status {
  padding-left: 8px;
  font-size: 12px;
  color: var(--md-primary);
  &.error { color: var(--md-error); }
}

/* 配图大图预览 */
.comments-lightbox {
  position: absolute;
  inset: 0;
  z-index: 5;
  display: grid;
  place-items: center;
  padding: 56px 48px;
  background: rgb(0 0 0 / 0.86);
  .comments-icon-btn {
    position: absolute;
    color: #fff;
    background: rgb(255 255 255 / 0.1);
    &:hover { background: rgb(255 255 255 / 0.2); }
  }
}

.lightbox-close { top: calc(var(--titlebar-height, 36px) + 8px); right: 12px; }
.lightbox-nav { top: 50%; transform: translateY(-50%); &.prev { left: 8px; } &.next { right: 8px; } }
.lightbox-index { position: absolute; bottom: 18px; font-size: 12px; color: rgb(255 255 255 / 0.8); font-variant-numeric: tabular-nums; }

.lightbox-image {
  max-width: 100%;
  max-height: 100%;
  border-radius: 12px;
  overflow: hidden;
  :deep(img) { max-width: 100%; max-height: calc(100vh - 160px); object-fit: contain; display: block; }
}

/* ---------- 动画 ---------- */

.comments-fade-enter-active,
.comments-fade-leave-active { transition: opacity 180ms var(--ease-standard); }
.comments-fade-enter-from,
.comments-fade-leave-to { opacity: 0; }

.comment-enter-active {
  transition: opacity 300ms var(--ease-standard), transform 360ms var(--ease-emphasized-decel);
  transition-delay: var(--enter-delay, 0ms);
}
.comment-enter-from { opacity: 0; transform: translateY(14px); }

.reply-enter-active { transition: opacity 260ms var(--ease-standard), transform 300ms var(--ease-emphasized-decel); }
.reply-enter-from { opacity: 0; transform: translateY(-6px); }
.reply-move { transition: transform 300ms var(--ease-emphasized-decel); }

.like-pop-enter-active { animation: comments-like-pop 360ms var(--ease-emphasized-decel); }
.like-pop-leave-active { transition: opacity 80ms linear; }
.like-pop-leave-to { opacity: 0; }

.composer-row-enter-active,
.composer-row-leave-active {
  transition: opacity 200ms var(--ease-standard), transform 240ms var(--ease-emphasized-decel), margin-bottom 240ms var(--ease-emphasized-decel);
}
.composer-row-enter-from,
.composer-row-leave-to { opacity: 0; transform: translateY(6px); margin-bottom: -30px; }

.spinning { animation: comments-spin 1s linear infinite; }
.spinning.small { font-size: 18px; }

@keyframes comments-spin { to { transform: rotate(360deg); } }

@keyframes comments-progress {
  from { transform: translateX(-100%); }
  to { transform: translateX(250%); }
}

@keyframes comments-like-pop {
  0% { transform: scale(0.6); }
  55% { transform: scale(1.25); }
  100% { transform: scale(1); }
}

@media (prefers-reduced-motion: reduce) {
  .comment-enter-active,
  .reply-enter-active,
  .reply-move,
  .replies-collapse,
  .like-pop-enter-active { transition: none; animation: none; }
}
</style>
