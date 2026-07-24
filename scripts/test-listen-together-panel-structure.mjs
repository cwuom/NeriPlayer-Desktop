import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const panelUrl = new URL('../src/components/ListenTogetherPanel.vue', import.meta.url)
const protocolUrl = new URL('../src/stores/listenTogether/protocol.ts', import.meta.url)
const [panelSource, protocolSource] = await Promise.all([
  readFile(panelUrl, 'utf8'),
  readFile(protocolUrl, 'utf8'),
])

const protocolFile = ts.createSourceFile(
  'protocol.ts',
  protocolSource,
  ts.ScriptTarget.Latest,
  true,
  ts.ScriptKind.TS,
)

function interfaceFields(name) {
  const declaration = protocolFile.statements.find(
    (statement) => ts.isInterfaceDeclaration(statement) && statement.name.text === name,
  )
  assert.ok(declaration, `missing protocol interface ${name}`)

  return declaration.members.map((member) => {
    assert.ok(member.name && ts.isIdentifier(member.name), `unsupported field in ${name}`)
    return member.name.text
  })
}

const protocolContracts = {
  ListenTogetherTrack: [
    'stableKey', 'channelId', 'audioId', 'subAudioId', 'playlistContextId',
    'mediaUri', 'streamUrl', 'name', 'artist', 'album', 'durationMs', 'coverUrl',
  ],
  ListenTogetherRoomSettings: [
    'allowMemberControl', 'autoPauseOnMemberChange', 'shareAudioLinks',
  ],
  ListenTogetherMember: ['userUuid', 'nickname', 'userId', 'role', 'joinedAt'],
  ListenTogetherRoomState: [
    'roomId', 'version', 'schemaVersion', 'controllerUserUuid', 'controllerUserId',
    'controllerHeartbeatAt', 'settings', 'members', 'queue', 'currentIndex', 'track',
    'playback', 'controllerOfflineSince', 'roomStatus', 'closedReason', 'updatedAt',
  ],
}

for (const [interfaceName, fields] of Object.entries(protocolContracts)) {
  assert.deepEqual(
    interfaceFields(interfaceName),
    fields,
    `${interfaceName} wire fields changed`,
  )
}

const roomStatusIndex = panelSource.indexOf('data-lt-section="room-status"')
const membersIndex = panelSource.indexOf('data-lt-section="members"')
const controlsIndex = panelSource.indexOf('data-lt-section="controls"')

assert.ok(roomStatusIndex >= 0, 'connected panel needs a dedicated room-status section')
assert.ok(membersIndex > roomStatusIndex, 'members section should follow room status')
assert.ok(controlsIndex > membersIndex, 'room controls should follow the member list')

assert.match(panelSource, /class="lt-room-overview"/)
assert.match(panelSource, /class="lt-status-chip"[^>]*:class="roomStatusTone"/)
assert.match(panelSource, /data-lt-action="copy-invite"[^>]*@click="lt\.copyInviteLink\(\)"/)
assert.match(panelSource, /data-lt-section="members"[\s\S]*?lt\.members\.length/)
assert.match(panelSource, /v-for="m in lt\.members"[^>]*:key="m\.userUuid"/)
assert.match(panelSource, /data-lt-section="controls"[^>]*v-if="lt\.isController"/)

for (const field of ['allowMemberControl', 'autoPauseOnMemberChange', 'shareAudioLinks']) {
  assert.match(
    panelSource,
    new RegExp(`data-lt-section="controls"[\\s\\S]*?updateRoomSettings\\(\\{ ${field}:`),
    `room controls must keep using ${field}`,
  )
}

for (const action of ['create-room', 'join-room', 'leave-room']) {
  assert.match(panelSource, new RegExp(`data-lt-action="${action}"`), `missing ${action} entry`)
}

assert.match(
  panelSource,
  /v-if="!lt\.roomId && lt\.connectionState !== 'connecting'"/,
  'a disconnected session with a room must not fall back to the create/join form',
)
assert.match(
  panelSource,
  /v-else-if="lt\.connectionState !== 'connected'"[\s\S]*?v-if="lt\.roomId"[\s\S]*?data-lt-action="leave-room"/,
  'connecting and reconnecting sessions must keep a leave-room action',
)

assert.match(
  panelSource,
  /\.lt-panel\s*\{[\s\S]*?bottom:\s*88px;[\s\S]*?max-height:\s*calc\(100vh - 136px\)/,
  'desktop panel must stay below the 36px title bar with a visible gap',
)
assert.match(
  panelSource,
  /@media\s*\(max-width:\s*520px\)[\s\S]*?bottom:\s*84px;[\s\S]*?max-height:\s*calc\(100vh - 132px\)/,
  'narrow panel must stay below the title bar even when content scrolls',
)

console.log('listen together panel structure tests passed')
