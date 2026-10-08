import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const source = await readFile(new URL('../src/stores/listenTogether/queue.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText
const {
  queueReferences,
  findQueueReferenceIndex,
  applyListenTogetherQueueMutation,
  buildListenTogetherQueueMutationPlan,
  setLtQueueReference,
  getLtQueueReference,
} = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`)

const track = (stableKey, name = stableKey) => ({
  stableKey, name, channelId: 'netease', audioId: stableKey, artist: 'artist', durationMs: 1,
})
const reference = (stableKey, occurrence = 0) => ({ stableKey, occurrence })
const apply = (queue, currentIndex, operations, extra = {}) => applyListenTogetherQueueMutation({
  roomQueue: queue, roomCurrentIndex: currentIndex, roomVersion: 7,
  mutation: { baseRoomVersion: 7, operations, ...extra },
})

// 所有引用基于原始 occurrence，连续删除和重排不因前序操作错位
{
  const first = track('dup', 'first')
  const last = track('dup', 'last')
  const queue = [first, track('middle'), last]
  assert.deepEqual(queueReferences(queue), [reference('dup'), reference('middle'), reference('dup', 1)])
  assert.equal(findQueueReferenceIndex(queue, reference('dup', 1)), 2)
  assert.equal(findQueueReferenceIndex(queue, reference('dup', -1)), -1)
  const removed = apply(queue, 2, [{ type: 'remove', target: reference('dup') }])
  assert.deepEqual(removed.queue, [queue[1], last])
  assert.equal(removed.currentIndex, 1)
  assert.equal(removed.currentRemoved, false)
  const cleared = apply(queue, 2, [{ type: 'remove_many', order: [reference('dup'), reference('dup', 1)] }])
  assert.deepEqual(cleared.queue, [queue[1]])
  assert.equal(cleared.currentRemoved, true)
  const selected = apply(queue, 0, [], { targetCurrent: reference('dup', 1) })
  assert.equal(selected.currentIndex, 0)
  assert.equal(selected.targetCurrentIndex, 2)

  // 重复对象也占用不同槽位，不能被对象 indexOf 合并
  const repeatedObject = apply([first, first], 1, [{ type: 'remove', target: reference('dup') }])
  assert.equal(repeatedObject.currentIndex, 0)
  assert.equal(repeatedObject.currentRemoved, false)
}

// compact reorder 仅改变所选槽位，保留并发插入项及当前歌曲
{
  const queue = [track('a'), track('remote'), track('b')]
  const result = apply(queue, 0, [{ type: 'reorder', order: [reference('b'), reference('a')] }])
  assert.deepEqual(result.queue.map(t => t.stableKey), ['b', 'remote', 'a'])
  assert.equal(result.currentIndex, 2)
  const invalidOrder = apply(queue, 0, [
    { type: 'remove', target: reference('b') },
    { type: 'reorder', order: [reference('b'), reference('a'), reference('a')] },
  ])
  assert.deepEqual(invalidOrder.queue.map(t => t.stableKey), ['a', 'remote'])
}

// 插入按 before/prepend/append 放置，已不存在的锚点退回 append
{
  const queue = [track('a'), track('b')]
  const result = apply(queue, 1, [
    { type: 'insert', track: track('x'), placement: 'before', anchor: reference('b') },
    { type: 'insert', track: track('y'), placement: 'before', anchor: reference('b') },
    { type: 'insert', track: track('front'), placement: 'prepend' },
    { type: 'move', target: reference('a'), placement: 'append' },
    { type: 'insert', track: track('tail'), placement: 'before', anchor: reference('missing') },
  ])
  assert.deepEqual(result.queue.map(t => t.stableKey), ['front', 'x', 'y', 'b', 'a', 'tail'])
  assert.equal(result.currentIndex, 3)
}

// builder 可以表达插入、删除、替换和重新排序，往返保持每个重复项
function permutations(values) {
  if (!values.length) return [[]]
  return values.flatMap((value, index) => permutations(values.filter((_, i) => i !== index))
    .map(rest => [value, ...rest]))
}
{
  const base = [track('dup', 'first'), track('b'), track('dup', 'second'), track('c')]
  for (const target of permutations(base)) {
    const plan = buildListenTogetherQueueMutationPlan({ version: 7, queue: base }, target, target.indexOf(base[2]))
    assert.equal(plan.requiresSnapshotFallback, false)
    assert.equal(plan.mutation.baseRoomVersion, 7)
    assert.deepEqual(plan.mutation.targetCurrent, reference('dup', 1))
    const result = applyListenTogetherQueueMutation({
      roomQueue: base, roomCurrentIndex: 2, roomVersion: 7, mutation: plan.mutation,
    })
    assert.deepEqual(result.queue, target)
    assert.equal(result.currentIndex, target.indexOf(base[2]))
  }
  for (const target of [
    [base[0], track('x'), track('y'), base[2]],
    [base[2], base[3]],
    [track('replacement')],
    [],
  ]) {
    const plan = buildListenTogetherQueueMutationPlan({ version: 7, queue: base }, target, 0)
    const result = applyListenTogetherQueueMutation({ roomQueue: base, roomCurrentIndex: 2, mutation: plan.mutation })
    assert.equal(plan.requiresSnapshotFallback, false)
    assert.deepEqual(result.queue, target)
  }
  const copied = [{ ...base[2] }, { ...base[1] }]
  const plan = buildListenTogetherQueueMutationPlan({ version: 7, queue: base }, copied, 0)
  assert.deepEqual(plan.mutation.targetCurrent, reference('dup', 1))
  assert.deepEqual(applyListenTogetherQueueMutation({ roomQueue: base, roomCurrentIndex: 2,
    mutation: plan.mutation }).queue, copied)
}

// 大队列压缩 reorder/remove_many，超限插入必须要求全快照
{
  const queue = Array.from({ length: 100 }, (_, i) => track(String(i)))
  const reorder = buildListenTogetherQueueMutationPlan({ version: 7, queue }, queue.toReversed(), 99)
  assert.equal(reorder.requiresSnapshotFallback, false)
  assert.equal(reorder.mutation.operations.length, 1)
  assert.equal(reorder.mutation.operations[0].type, 'reorder')
  const cleared = buildListenTogetherQueueMutationPlan({ version: 7, queue }, [], -1)
  assert.equal(cleared.mutation.operations[0].type, 'remove_many')
  assert.equal(applyListenTogetherQueueMutation({ roomQueue: queue, roomCurrentIndex: 0,
    mutation: cleared.mutation }).currentIndex, -1)
  const inserted = Array.from({ length: 65 }, (_, i) => track(`new-${i}`))
  assert.equal(buildListenTogetherQueueMutationPlan({ version: 7, queue }, [...queue, ...inserted], 0)
    .requiresSnapshotFallback, true)
  assert.equal(buildListenTogetherQueueMutationPlan({ version: 7, queue }, [track(' ')], 0)
    .requiresSnapshotFallback, true)
  assert.equal(applyListenTogetherQueueMutation({ roomQueue: queue, roomCurrentIndex: 0, roomVersion: 7,
    mutation: { baseRoomVersion: 8, operations: [] } }), null)
  assert.equal(applyListenTogetherQueueMutation({ roomQueue: queue, roomCurrentIndex: 0,
    mutation: { baseRoomVersion: 7, operations: Array(65).fill({ type: 'remove', target: reference('0') }) } }), null)
  for (const operation of [
    { type: 'unknown' }, { type: 'insert' }, { type: 'reorder', order: 'invalid' },
    { type: 'remove', target: reference('0', -1) },
  ]) {
    assert.equal(apply(queue, 0, [operation]), null)
  }
  assert.deepEqual(applyListenTogetherQueueMutation({ roomQueue: queue, roomCurrentIndex: 0, roomVersion: 9,
    mutation: { baseRoomVersion: 7, operations: [] } }).queue, queue)
  const full = Array.from({ length: 2000 }, (_, i) => track(String(i)))
  assert.equal(apply(full, 0, [{ type: 'insert', placement: 'append', track: track('over') }]).queue.length, 2000)
}

// 显式基准槽位优先于相同 metadata，WeakMap 标记不会进入事件 JSON
{
  const base = [track('dup', 'same'), track('dup', 'same')]
  const marked = occurrence => {
    const item = track('dup', 'same')
    setLtQueueReference(item, reference('dup', occurrence))
    return item
  }
  for (const retained of [0, 1]) {
    const target = [marked(retained)]
    const plan = buildListenTogetherQueueMutationPlan({ version: 7, queue: base }, target, 0)
    assert.equal(plan.requiresSnapshotFallback, false)
    assert.deepEqual(plan.mutation.operations, [{ type: 'remove', target: reference('dup', 1 - retained) }])
    assert.deepEqual(plan.mutation.targetCurrent, reference('dup', retained))
    assert.equal(applyListenTogetherQueueMutation({ roomQueue: base, roomCurrentIndex: retained,
      mutation: plan.mutation }).queue[0], base[retained])
  }
  const reversed = buildListenTogetherQueueMutationPlan({ version: 7, queue: base }, [marked(1), marked(0)], 0)
  const result = applyListenTogetherQueueMutation({ roomQueue: base, roomCurrentIndex: 1, mutation: reversed.mutation })
  assert.equal(result.queue[0], base[1])
  assert.equal(result.queue[1], base[0])
  assert.equal(result.currentIndex, 0)
  assert.deepEqual(reversed.mutation.targetCurrent, reference('dup', 1))

  const inserted = track('dup', 'same')
  const insertion = buildListenTogetherQueueMutationPlan({ version: 7, queue: [base[0]] }, [inserted, marked(0)], 0)
  assert.equal(insertion.needsCurrentSelectionAfterApply, true)
  assert.equal(insertion.mutation.targetCurrent, undefined)
  assert.deepEqual(insertion.mutation.operations, [{ type: 'insert', track: inserted,
    anchor: reference('dup'), placement: 'before' }])
  assert.equal(buildListenTogetherQueueMutationPlan({ version: 7, queue: base }, [marked(0), marked(1)], 1)
    .needsCurrentSelectionAfterApply, false)
  assert.equal(buildListenTogetherQueueMutationPlan({ version: 7, queue: base }, [track('new')], 0)
    .needsCurrentSelectionAfterApply, false)
  assert.equal(buildListenTogetherQueueMutationPlan({ version: 7, queue: base }, [marked(0), marked(0)], 0)
    .requiresSnapshotFallback, true)

  const originalReference = reference('dup', 1)
  setLtQueueReference(inserted, originalReference)
  originalReference.occurrence = 0
  assert.deepEqual(getLtQueueReference(inserted), reference('dup', 1))
  const copiedReference = getLtQueueReference(inserted)
  copiedReference.occurrence = 0
  assert.deepEqual(getLtQueueReference(inserted), reference('dup', 1))
  assert.equal(JSON.stringify(inserted).includes('occurrence'), false)
}

console.log('test-listen-together-queue: ok')
