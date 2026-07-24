interface PhysicalPointLike {
  x: number
  y: number
}

interface PhysicalSizeLike {
  width: number
  height: number
}

interface MonitorWorkAreaLike {
  workArea: {
    position: PhysicalPointLike
    size: PhysicalSizeLike
  }
}

interface WorkAreaRect {
  left: number
  top: number
  right: number
  bottom: number
}

function isFinitePoint(value: PhysicalPointLike): boolean {
  return Number.isFinite(value.x) && Number.isFinite(value.y)
}

function isValidSize(value: PhysicalSizeLike): boolean {
  return Number.isFinite(value.width)
    && Number.isFinite(value.height)
    && value.width > 0
    && value.height > 0
}

function intersectionArea(
  position: PhysicalPointLike,
  size: PhysicalSizeLike,
  area: WorkAreaRect,
): number {
  const width = Math.max(0, Math.min(position.x + size.width, area.right) - Math.max(position.x, area.left))
  const height = Math.max(0, Math.min(position.y + size.height, area.bottom) - Math.max(position.y, area.top))
  return width * height
}

function distanceToArea(
  position: PhysicalPointLike,
  size: PhysicalSizeLike,
  area: WorkAreaRect,
): number {
  const windowRight = position.x + size.width
  const windowBottom = position.y + size.height
  const dx = windowRight < area.left
    ? area.left - windowRight
    : (position.x > area.right ? position.x - area.right : 0)
  const dy = windowBottom < area.top
    ? area.top - windowBottom
    : (position.y > area.bottom ? position.y - area.bottom : 0)
  return dx * dx + dy * dy
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(Math.max(value, minimum), maximum)
}

export function clampWindowPositionToWorkAreas(
  savedPosition: PhysicalPointLike,
  windowSize: PhysicalSizeLike,
  monitors: readonly MonitorWorkAreaLike[],
): PhysicalPointLike | null {
  if (!isFinitePoint(savedPosition) || !isValidSize(windowSize)) return null

  const workAreas = monitors.flatMap(({ workArea }) => {
    if (!workArea || !isFinitePoint(workArea.position) || !isValidSize(workArea.size)) return []
    if (workArea.size.width < windowSize.width || workArea.size.height < windowSize.height) return []
    return [{
      left: workArea.position.x,
      top: workArea.position.y,
      right: workArea.position.x + workArea.size.width,
      bottom: workArea.position.y + workArea.size.height,
    }]
  })
  if (workAreas.length === 0) return null

  const containingArea = workAreas.find(area => (
    savedPosition.x >= area.left
    && savedPosition.y >= area.top
    && savedPosition.x + windowSize.width <= area.right
    && savedPosition.y + windowSize.height <= area.bottom
  ))
  if (containingArea) return { ...savedPosition }

  const targetArea = workAreas.reduce((best, area) => {
    const areaIntersection = intersectionArea(savedPosition, windowSize, area)
    const bestIntersection = intersectionArea(savedPosition, windowSize, best)
    if (areaIntersection !== bestIntersection) {
      return areaIntersection > bestIntersection ? area : best
    }
    return distanceToArea(savedPosition, windowSize, area)
      < distanceToArea(savedPosition, windowSize, best)
      ? area
      : best
  })

  const maxX = Math.max(targetArea.left, targetArea.right - windowSize.width)
  const maxY = Math.max(targetArea.top, targetArea.bottom - windowSize.height)
  return {
    x: clamp(savedPosition.x, targetArea.left, maxX),
    y: clamp(savedPosition.y, targetArea.top, maxY),
  }
}
