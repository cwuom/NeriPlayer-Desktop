import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import vm from 'node:vm'

const assets = new URL('../src-tauri/src/api/youtube/assets/', import.meta.url)
const lib = await readFile(new URL('yt.solver.lib.min.js', assets), 'utf8')
const core = await readFile(new URL('yt.solver.core.min.js', assets), 'utf8')
const fixture = '(function(){function F(a,b,c){var values={s:c,n:""};var prototype={set:function(key,value){values[key]=value;},get:function(key){return values[key];},clone:function(){return this;},solve:function(){values.s=values.s?values.s.split("").reverse().join(""):values.s;values.n=values.n?values.n.split("").reverse().join(""):values.n;}};var url=Object.create(prototype);url.set("alr","yes");return url;}}).call(this);'

const input = {
  type: 'player',
  player: fixture,
  requests: [
    { type: 'sig', challenges: ['abc'] },
    { type: 'n', challenges: ['xyz'] },
  ],
}
const output = vm.runInNewContext(
  lib + '\nObject.assign(globalThis, lib);\n' + core + '\nJSON.stringify(jsc(' + JSON.stringify(input) + '));',
  Object.create(null),
  { timeout: 1000 },
)
const responses = JSON.parse(output).responses
assert.deepEqual(responses[0], { type: 'result', data: { abc: 'cba' } })
assert.deepEqual(responses[1], { type: 'result', data: { xyz: 'zyx' } })
console.log('YouTube bundled EJS signature and throttling fixture passed')
