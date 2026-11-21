import { describe, expect, it } from 'vitest'
import { Spring, type SpringParams } from '@/utils/spring'

const FRAME = 1 / 60

/** 逐帧推进并记录每帧位置 */
function run(spring: Spring, seconds: number, step = FRAME): number[] {
  const positions: number[] = []
  const frames = Math.round(seconds / step)
  for (let i = 0; i < frames; i++) {
    spring.update(step)
    positions.push(spring.getCurrentPosition())
  }
  return positions
}

function springTo(target: number, params: Partial<SpringParams> = {}, from = 0): Spring {
  const spring = new Spring(from)
  spring.updateParams(params)
  spring.setTargetPosition(target)
  return spring
}

describe('Spring', () => {
  it('starts at rest at the initial position', () => {
    expect(new Spring().getCurrentPosition()).toBe(0)
    const spring = new Spring(5)
    expect(spring.getCurrentPosition()).toBe(5)
    expect(spring.arrived()).toBe(true)
    spring.update(1)
    expect(spring.getCurrentPosition()).toBe(5)
  })

  it('does not move before any time has elapsed', () => {
    const spring = springTo(10)
    spring.update(0)
    expect(spring.getCurrentPosition()).toBe(0)
    expect(spring.arrived()).toBe(false)
  })

  it('settles exactly on the target and reports arrival', () => {
    const spring = springTo(10)
    run(spring, 3)
    expect(spring.arrived()).toBe(true)
    expect(spring.getCurrentPosition()).toBe(10)
  })

  it('overshoots the target with the default under-damped parameters', () => {
    const positions = run(springTo(10), 2)
    expect(Math.max(...positions)).toBeGreaterThan(10.1)
  })

  it('approaches monotonically without overshoot when over-damped', () => {
    const positions = run(springTo(10, { damping: 40 }), 3)
    for (let i = 1; i < positions.length; i++) {
      expect(positions[i]).toBeGreaterThanOrEqual(positions[i - 1])
    }
    expect(Math.max(...positions)).toBeLessThanOrEqual(10)
  })

  it('suppresses overshoot when soft is set even with low damping', () => {
    const positions = run(springTo(10, { damping: 2, soft: true }), 3)
    expect(Math.max(...positions)).toBeLessThanOrEqual(10)
    expect(positions.at(-1)).toBe(10)
  })

  it('moves a heavier spring more slowly', () => {
    const light = springTo(10, { mass: 1 })
    const heavy = springTo(10, { mass: 4 })
    light.update(0.1)
    heavy.update(0.1)
    expect(heavy.getCurrentPosition()).toBeLessThan(light.getCurrentPosition())
  })

  it('is frame-rate independent because it evaluates a closed-form solution', () => {
    const coarse = springTo(10)
    const fine = springTo(10)
    coarse.update(0.5)
    run(fine, 0.5, 0.005)
    expect(fine.getCurrentPosition()).toBeCloseTo(coarse.getCurrentPosition(), 6)
  })

  it('keeps position continuous when retargeted mid-flight', () => {
    const spring = springTo(10)
    run(spring, 0.1)
    const before = spring.getCurrentPosition()
    spring.setTargetPosition(-5)
    spring.update(0)
    expect(spring.getCurrentPosition()).toBeCloseTo(before, 9)
    run(spring, 4)
    expect(spring.getCurrentPosition()).toBe(-5)
  })

  it('carries velocity across a retarget instead of restarting from rest', () => {
    const moving = springTo(10)
    run(moving, 0.1)
    const start = moving.getCurrentPosition()
    moving.setTargetPosition(start)
    moving.update(0.02)
    // 仍保有朝 +x 的速度，目标虽在原地也会先冲过去
    expect(moving.getCurrentPosition()).toBeGreaterThan(start)
  })

  it('delays a target change until the delay has elapsed', () => {
    const spring = new Spring(0)
    spring.setTargetPosition(10, 0.5)
    expect(spring.arrived()).toBe(false)

    run(spring, 0.4)
    expect(spring.getCurrentPosition()).toBe(0)

    run(spring, 0.2)
    run(spring, 0.1)
    expect(spring.getCurrentPosition()).toBeGreaterThan(0)
  })

  it('cancels a pending delayed target when an immediate target is set', () => {
    const spring = new Spring(0)
    spring.setTargetPosition(10, 0.2)
    spring.setTargetPosition(-3)
    run(spring, 4)
    expect(spring.getCurrentPosition()).toBe(-3)
  })

  it('reports not-arrived while delayed params are pending, then settles', () => {
    const spring = new Spring(2)
    spring.updateParams({ stiffness: 300 }, 0.5)
    expect(spring.arrived()).toBe(false)
    spring.update(0.6)
    expect(spring.arrived()).toBe(true)
    expect(spring.getCurrentPosition()).toBe(2)
  })

  it('applies delayed params only after the delay', () => {
    const delayed = springTo(10)
    delayed.updateParams({ stiffness: 1000 }, 1)
    const baseline = springTo(10)
    delayed.update(0.05)
    baseline.update(0.05)
    expect(delayed.getCurrentPosition()).toBeCloseTo(baseline.getCurrentPosition(), 9)
  })

  it('merges consecutive delayed param updates', () => {
    const merged = new Spring(0)
    merged.updateParams({ stiffness: 400 }, 0.1)
    merged.updateParams({ damping: 80 }, 0.1)
    merged.update(0.1)
    merged.setTargetPosition(10)

    const expected = springTo(10, { stiffness: 400, damping: 80 })
    run(merged, 1)
    run(expected, 1)
    expect(merged.getCurrentPosition()).toBeCloseTo(expected.getCurrentPosition(), 9)
  })

  it('pins the position instantly and drops pending work on setPosition', () => {
    const spring = springTo(10)
    run(spring, 0.1)
    spring.setTargetPosition(20, 0.5)
    spring.updateParams({ stiffness: 500 }, 0.5)

    spring.setPosition(3)
    expect(spring.getCurrentPosition()).toBe(3)
    expect(spring.arrived()).toBe(true)

    run(spring, 1)
    expect(spring.getCurrentPosition()).toBe(3)
  })
})
