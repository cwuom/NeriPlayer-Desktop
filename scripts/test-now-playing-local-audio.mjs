import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { compile as compileVue } from 'vue'
import ts from 'typescript'

const source = await readFile(new URL('../src/modules/playback/audioQualityDisplay.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText
const { isLocalAudioPlayback, canSwitchAudioQuality, resolveAudioQualityLabel, actualAudioBitrateLabel } =
  await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`)

let fallbackCalls = 0
const fallback = (source, key) => { fallbackCalls++; return `${source}:${key || 'configured'}` }
const downloaded = { source: 'netease', fromDownload: true, info: { source: 'netease', qualityLabel: '极高', qualityKey: 'exhigh', bitrate: 317.8 } }
assert.equal(isLocalAudioPlayback(downloaded), true)
assert.equal(canSwitchAudioQuality(downloaded), false)
assert.equal(resolveAudioQualityLabel(downloaded, fallback), '')
assert.equal(fallbackCalls, 0)
assert.equal(actualAudioBitrateLabel(downloaded.info), '318 kbps')

for (const local of [
  { source: 'netease', fromDownload: true, info: null },
  { source: 'local', fromDownload: false, info: null },
  { source: 'netease', fromDownload: false, info: { source: 'local', qualityLabel: '无损' } },
  { source: 'youtube', fromDownload: true, info: { format: 'Opus' } },
]) {
  assert.equal(isLocalAudioPlayback(local), true)
  assert.equal(canSwitchAudioQuality(local), false)
  assert.equal(resolveAudioQualityLabel(local, fallback), '')
  assert.equal(actualAudioBitrateLabel(local.info), '')
}
assert.equal(fallbackCalls, 0, 'missing local bitrate must not use configured online quality')

for (const source of ['netease', 'qq', 'bilibili', 'youtube']) {
  const online = { source, fromDownload: false, info: null }
  assert.equal(canSwitchAudioQuality(online), true)
  assert.equal(resolveAudioQualityLabel(online, fallback), `${source}:configured`)
}
assert.equal(canSwitchAudioQuality({ source: 'unknown', fromDownload: false, info: null }), false)
// 已知档位用界面语言的译名，而不是播放层写入的固定中文
assert.equal(resolveAudioQualityLabel({ source: 'youtube', fromDownload: false, info: { qualityLabel: '高', qualityKey: 'high' } }, fallback), 'youtube:high')
// 界面不认识的档位（译名回退为原始 key）才展示平台给的描述
const knownOnly = (source, key) => (key === 'high' ? 'High' : key || '')
assert.equal(resolveAudioQualityLabel({ source: 'bilibili', fromDownload: false, info: { qualityLabel: 'Dolby Atmos', qualityKey: '30250' } }, knownOnly), 'Dolby Atmos')
assert.equal(resolveAudioQualityLabel({ source: 'bilibili', fromDownload: false, info: { qualityLabel: 'Dolby Atmos' } }, knownOnly), 'Dolby Atmos')
assert.equal(resolveAudioQualityLabel({ source: 'netease', fromDownload: false, info: { qualityLabel: '320 kbps', qualityKey: 'exhigh' } }, fallback), 'netease:exhigh')
assert.equal(resolveAudioQualityLabel({ source: 'netease', fromDownload: false, info: { source: 'bilibili', qualityKey: 'high' } }, fallback), 'bilibili:high')
for (const bitrate of [undefined, NaN, Infinity, -1, 0]) assert.equal(actualAudioBitrateLabel({ bitrate }), '')

const componentSource = await readFile(new URL('../src/components/NowPlaying.vue', import.meta.url), 'utf8')
const script = componentSource.match(/<script setup lang="ts">([\s\S]*?)<\/script>/)[1]
const parsed = ts.createSourceFile('NowPlaying.ts', script, ts.ScriptTarget.ES2022, true)
const specFunction = parsed.statements.find(node => ts.isFunctionDeclaration(node) && node.name?.text === 'paperSpecFromAudioInfo')
assert.ok(specFunction, 'playback page should retain its factual audio specification formatter')
const specCompiled = ts.transpileModule(`export ${specFunction.getText(parsed)}`, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText
const { paperSpecFromAudioInfo } = await import(`data:text/javascript;base64,${Buffer.from(specCompiled).toString('base64')}`)
assert.deepEqual(paperSpecFromAudioInfo({ sampleRateHz: 48000, bitDepth: 16, channelCount: 2, specLabel: '极高 | 320 kbps' }, false), ['48 kHz', '16 bit', '2 ch'])
assert.deepEqual(paperSpecFromAudioInfo({ specLabel: '48 kHz | 16 bit | 无损' }, false), [], 'local files must not inherit paper specifications from an online quality label')
assert.deepEqual(paperSpecFromAudioInfo({ sampleRateHz: 44100, bitDepth: 24 }, true), ['44.1 kHz', '24 bit'])
const displayFunctions = ['normalizeAudioDisplayToken', 'isHiddenAudioInfoToken'].map(name => {
  const declaration = parsed.statements.find(node => ts.isFunctionDeclaration(node) && node.name?.text === name)
  assert.ok(declaration)
  return `export ${declaration.getText(parsed)}`
}).join('\n')
const displayCompiled = ts.transpileModule(displayFunctions, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText
const { normalizeAudioDisplayToken } = await import(`data:text/javascript;base64,${Buffer.from(displayCompiled).toString('base64')}`)
assert.equal(normalizeAudioDisplayToken('mpeg', true), 'MPEG', 'a detected MPEG file may use layer 1, 2 or 3')
assert.equal(normalizeAudioDisplayToken('mpeg'), 'MP3', 'online display keeps its existing format mapping')
assert.equal(normalizeAudioDisplayToken('flac', true), 'FLAC')

const downloadStart = componentSource.indexOf('<span v-if="displayedAudioInfo.fromDownload" class="np-download-chip"')
assert.ok(downloadStart >= 0)
let downloadEnd = downloadStart
let spanDepth = 0
for (const span of componentSource.slice(downloadStart).matchAll(/<\/?span\b[^>]*>/g)) {
  spanDepth += span[0].startsWith('</') ? -1 : 1
  if (spanDepth === 0) {
    downloadEnd = downloadStart + span.index + span[0].length
    break
  }
}
const downloadRender = compileVue(componentSource.slice(downloadStart, downloadEnd))
const downloadIndicator = downloadRender({ displayedAudioInfo: { fromDownload: true }, t: () => '正在播放下载' }, [])
const visibleText = node => typeof node === 'string' ? node
  : typeof node.children === 'string' ? node.children
    : Array.isArray(node.children) ? node.children.map(visibleText).join('') : ''
assert.equal(visibleText(downloadIndicator).trim(), 'download_done', 'download playback indicator contains only an underlined check icon')
assert.equal(downloadIndicator.props.title, '正在播放下载')
assert.equal(downloadIndicator.props['aria-label'], '正在播放下载')

const infoPartsStatement = parsed.statements.find(node => ts.isVariableStatement(node)
  && node.declarationList.declarations.some(declaration => declaration.name.getText(parsed) === 'audioInfoParts'))
assert.ok(infoPartsStatement)
const partFunctions = ['currentAudioQualityLabel', 'paperSpecFromAudioInfo', 'addAudioInfoPart',
  'normalizeAudioDisplayToken', 'isHiddenAudioInfoToken', 'isSameAudioInfoToken'].map(name => {
  const declaration = parsed.statements.find(node => ts.isFunctionDeclaration(node) && node.name?.text === name)
  assert.ok(declaration)
  return declaration.getText(parsed)
}).join('\n')
const defaults = {
  showAudioBitrate: true, showAudioFormat: true, showAudioChannels: false,
  showAudioSampleRate: false, showAudioBitDepth: false, showQualitySwitch: false,
  showAudioCodec: false, showAudioSpec: false,
}
const parameterHarness = ts.transpileModule(`
  import { computed, shallowReactive } from ${JSON.stringify(import.meta.resolve('vue'))}
  const { actualAudioBitrateLabel, actualAudioParameterLabels, isLocalAudioPlayback, resolveAudioQualityLabel } =
    await import(${JSON.stringify(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`)})
  export const settings = shallowReactive(${JSON.stringify(defaults)})
  export const displayedAudioInfo = shallowReactive({ value: { info: null, fromDownload: false } })
  export const currentSource = shallowReactive({ value: 'netease' })
  const currentQualityKey = () => 'exhigh'
  const qualityLabelFor = (_source, key) => key === 'exhigh' ? '极高' : key
  ${partFunctions}
  ${infoPartsStatement.getText(parsed)}
  export { audioInfoParts }
`, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText
const display = await import(`data:text/javascript;base64,${Buffer.from(parameterHarness).toString('base64')}`)
const fullInfo = Object.freeze({
  bitrate: 317.8, codec: 'mp3', format: 'mpeg', channelCount: 2,
  sampleRateHz: 48000, bitDepth: 16, qualityLabel: '极高', qualityKey: 'exhigh',
  specLabel: '96 kHz | 24 bit | 无损',
})
const labels = () => display.audioInfoParts.value.map(part => part.text)
for (const playback of [
  { source: 'netease', fromDownload: false },
  { source: 'netease', fromDownload: true },
  { source: 'local', fromDownload: false },
]) {
  display.currentSource.value = playback.source
  display.displayedAudioInfo.value = { info: fullInfo, fromDownload: playback.fromDownload }
  Object.assign(display.settings, defaults)
  assert.deepEqual(labels(), ['318 kbps', 'MPEG'], 'both local files and streams show factual bitrate and format by default')
  display.settings.showAudioBitrate = false
  assert.deepEqual(labels(), ['MPEG'], 'format remains visible when bitrate is disabled')
  display.settings.showAudioFormat = false
  assert.deepEqual(labels(), [], 'bitrate must follow its independent visibility switch')
  for (const [setting, expected] of [
    ['showAudioFormat', 'MPEG'], ['showAudioChannels', '2 ch'],
    ['showAudioSampleRate', '48 kHz'], ['showAudioBitDepth', '16 bit'],
  ]) {
    display.settings[setting] = true
    assert.deepEqual(labels(), [expected], `${setting} must independently show only its actual field`)
    display.settings[setting] = false
  }
  Object.assign(display.settings, { showAudioBitrate: true, showAudioFormat: true,
    showAudioChannels: true, showAudioSampleRate: true, showAudioBitDepth: true })
  assert.deepEqual(labels(), ['318 kbps', 'MPEG', '2 ch', '48 kHz', '16 bit'])
  display.settings.showQualitySwitch = true
  assert.deepEqual(labels(), playback.source === 'local' || playback.fromDownload
    ? ['318 kbps', 'MPEG', '2 ch', '48 kHz', '16 bit']
    : ['极高', '318 kbps', 'MPEG', '2 ch', '48 kHz', '16 bit'])
}
display.settings.showQualitySwitch = false
display.displayedAudioInfo.value = { info: { specLabel: '96 kHz | 24 bit | 无损' }, fromDownload: false }
assert.deepEqual(labels(), [], 'missing measured fields must not be fabricated from a platform quality description')
display.displayedAudioInfo.value = { info: { bitrate: NaN, format: 'unknown', codec: 'local',
  channelCount: Infinity, sampleRateHz: Infinity, bitDepth: -1 }, fromDownload: true }
assert.deepEqual(labels(), [], 'unknown and invalid audio measurements must stay absent')
display.displayedAudioInfo.value = { info: { codec: 'opus', sampleRateHz: 44100 }, fromDownload: true }
assert.deepEqual(labels(), ['Opus', '44.1 kHz'], 'known codec is a fallback when format is absent')
for (const [format, codec, expected] of [
  ['audio/mp4', 'AAC', 'AAC'],
  ['audio/mp4', undefined, 'MP4'],
  ['audio/mp4; codecs="mp4a.40.2"', undefined, 'AAC'],
  ['audio/webm; codecs="opus"', undefined, 'Opus'],
  ['audio/mp4', 'E-AC-3', 'E-AC-3'],
  ['audio/x-flac', undefined, 'FLAC'],
  ['flac', 'FLAC', 'FLAC'],
]) {
  display.displayedAudioInfo.value = { info: { format, codec }, fromDownload: false }
  assert.deepEqual(labels(), [expected], `stream MIME ${format} must not leak into the quality row`)
}
assert.equal(normalizeAudioDisplayToken('audio/mp4'), 'MP4')
assert.equal(normalizeAudioDisplayToken('audio/webm; codecs="opus"'), 'Opus')

const downloadDeclarations = ['isCurrentDownloading', 'isCurrentDownloadCancellable', 'downloadTaskStatusText',
  'downloadActionIcon', 'downloadActionLabel', 'downloadActionDesc', 'downloadActionDisabled', 'handleDownloadAction'].map(name => {
  const declaration = parsed.statements.find(node => ts.isFunctionDeclaration(node) ? node.name?.text === name
    : ts.isVariableStatement(node) && node.declarationList.declarations.some(item => item.name.getText(parsed) === name))
  assert.ok(declaration)
  return declaration.getText(parsed)
}).join('\n')
const downloadHarness = ts.transpileModule(`
  import { computed, shallowReactive } from ${JSON.stringify(import.meta.resolve('vue'))}
  export const actions = []
  export const player = shallowReactive({ currentTrack: { id: 'netease:1' }, isPlayingFromDownload: true,
    handleDownloadedFileRemoved: id => actions.push(['premature-file-removal', id]) })
  export const currentDownloadTask = shallowReactive({ value: undefined })
  export const isCurrentDownloaded = shallowReactive({ value: true })
  const showMoreSheet = { value: true }
  const t = key => key
  const downloadStore = {
    getDownloadedTrack: () => ({ filePath: 'fixture.mp3' }),
    cancelDownload: async id => actions.push(['cancel', id]),
    redownloadTrack: async track => actions.push(['redownload', track.id]),
    downloadTrack: async track => actions.push(['download', track.id]),
  }
  ${downloadDeclarations}
  export { downloadActionDisabled, downloadActionIcon, downloadActionLabel, downloadActionDesc, handleDownloadAction }
`, { compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 } }).outputText
const downloadAction = await import(`data:text/javascript;base64,${Buffer.from(downloadHarness).toString('base64')}`)
assert.equal(downloadAction.downloadActionDisabled.value, false, 'currently playing downloaded songs remain operable')
assert.equal(downloadAction.downloadActionIcon.value, 'refresh')
assert.equal(downloadAction.downloadActionLabel.value, 'download.redownload')
assert.equal(downloadAction.downloadActionDesc.value, 'download.redownload_desc')
await downloadAction.handleDownloadAction()
assert.deepEqual(downloadAction.actions, [['redownload', 'netease:1']], 'current downloaded playback must use the guarded redownload path')
downloadAction.currentDownloadTask.value = { status: 'resolving' }
assert.equal(downloadAction.downloadActionDisabled.value, false)
assert.equal(downloadAction.downloadActionLabel.value, 'download.cancel_task')
await downloadAction.handleDownloadAction()
assert.deepEqual(downloadAction.actions.at(-1), ['cancel', 'netease:1'])
downloadAction.currentDownloadTask.value = { status: 'cancelling' }
assert.equal(downloadAction.downloadActionDisabled.value, true, 'cancellation in progress stays guarded')
downloadAction.currentDownloadTask.value = undefined
downloadAction.isCurrentDownloaded.value = false
await downloadAction.handleDownloadAction()
assert.deepEqual(downloadAction.actions.at(-1), ['download', 'netease:1'])
downloadAction.player.currentTrack = null
assert.equal(downloadAction.downloadActionDisabled.value, true)
console.log('now playing local audio quality tests passed')
