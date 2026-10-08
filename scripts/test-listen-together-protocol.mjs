// Listen-together wire protocol helpers (Android-aligned ExoPlayer ints)
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const source = await readFile(new URL('../src/stores/listenTogether/protocol.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText
const { desktopRepeatToWire, wireRepeatToDesktop, isValidLtNickname, parseLtInvite, resolveLocalRoomControlRestriction } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`,
)

// 对齐 Android LocalRoomControlRestrictionTest
assert.equal(resolveLocalRoomControlRestriction('controller_offline', false, true), null, 'the host is never restricted')
assert.equal(resolveLocalRoomControlRestriction('controller_offline', false, false), 'controller_offline')
assert.equal(resolveLocalRoomControlRestriction('active', false, false), 'member_control_disabled')
assert.equal(resolveLocalRoomControlRestriction('active', true, false), null)
assert.equal(resolveLocalRoomControlRestriction(null, null, false), null)

// 昵称与服务端同一规则：汉字（含 〇、々、扩展区）、ASCII 字母数字，按码点最多 24 个
for (const valid of ['Tester', '听歌的人', '〇々', '𠀀𠀁', 'a'.repeat(24), '𠀀'.repeat(24)]) {
  assert.equal(isValidLtNickname(valid), true, valid)
}
for (const invalid of ['', '   ', 'a'.repeat(25), 'with-dash', 'emoji😀', 'ｆｕｌｌ', 'カタカナ']) {
  assert.equal(isValidLtNickname(invalid), false, invalid)
}

const invite = 'neriplayer://listen-together/join?inviter=Tom&roomId=abc234&secret=s3cret&baseUrl=https%3A%2F%2Fltw.example.com%2F'
assert.deepEqual(parseLtInvite(`来一起听吧 ${invite} 这个房间`), {
  roomId: 'ABC234', joinSecret: 's3cret', baseUrl: 'https://ltw.example.com', inviter: 'Tom', link: invite, hasInvalidBaseUrl: false,
})
assert.equal(
  'inviter' in parseLtInvite('neriplayer://listen-together/join?inviter=bad-name&roomId=ABC234&secret=x'),
  false,
  'an inviter that is not a valid nickname is dropped',
)
assert.equal(parseLtInvite('neriplayer-debug://listen-together/join?roomId=ABC234&secret=x')?.roomId, 'ABC234')
assert.equal(parseLtInvite('neriplayer://listen-together/join?roomId=ABC234'), null, 'an invite without a secret cannot be used')
assert.equal(parseLtInvite('myneriplayer://listen-together/join?roomId=ABC234&secret=x'), null, 'no match inside a longer scheme')
assert.equal(parseLtInvite('neriplayer://listen-together/join?roomId=AB&secret=x'), null)
assert.equal(parseLtInvite('neriplayer://listen-together/join?roomId=ABC234&secret=x&baseUrl=http%3A%2F%2Finsecure.example')?.hasInvalidBaseUrl, true)

assert.equal(desktopRepeatToWire('off'), 0)
assert.equal(desktopRepeatToWire('one'), 1)
assert.equal(desktopRepeatToWire('all'), 2)
assert.equal(desktopRepeatToWire(undefined), 0)
assert.equal(wireRepeatToDesktop(0), 'off')
assert.equal(wireRepeatToDesktop(1), 'one')
assert.equal(wireRepeatToDesktop(2), 'all')
assert.equal(wireRepeatToDesktop(null), null)
assert.equal(wireRepeatToDesktop(99), null)

// Snapshot shape must carry mode fields for create-room
const snapshot = {
  queue: [],
  currentIndex: 0,
  settings: { allowMemberControl: true, autoPauseOnMemberChange: true, shareAudioLinks: true },
  isPlaying: false,
  positionMs: 0,
  repeatMode: desktopRepeatToWire('all'),
  shuffleEnabled: true,
}
assert.equal(snapshot.repeatMode, 2)
assert.equal(snapshot.shuffleEnabled, true)

// Event type names stay Android-aligned
const modeEvent = {
  type: 'PLAYBACK_MODE',
  repeatMode: desktopRepeatToWire('one'),
  shuffleEnabled: false,
}
assert.equal(modeEvent.type, 'PLAYBACK_MODE')
assert.equal(modeEvent.repeatMode, 1)

console.log('test-listen-together-protocol: ok')
