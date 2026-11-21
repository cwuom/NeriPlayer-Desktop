/**
 * 外部 JSON（InnerTube、网易云 / B 站接口原始响应）的形状收窄工具
 *
 * 这类响应随平台改版变化，逐层声明完整接口既写不全也很快过时；统一按 unknown 接收，
 * 读取时逐层校验形状，取不到就得到 undefined 或空数组，调用方不需要再做类型断言
 */

export type JsonRecord = Readonly<Record<string, unknown>>

/** 普通对象（排除 null 与数组） */
export function isJsonRecord(value: unknown): value is JsonRecord {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

export function asRecord(value: unknown): JsonRecord | undefined {
  return isJsonRecord(value) ? value : undefined
}

export function asArray(value: unknown): unknown[] {
  return Array.isArray(value) ? value : []
}

export function asString(value: unknown): string | undefined {
  return typeof value === 'string' ? value : undefined
}

/** 深搜时要继续展开的子节点：数组元素或对象字段中的对象值（含数组），保持原有顺序 */
export function objectChildren(node: object): object[] {
  const values: unknown[] = Object.values(node)
  return values.filter((value): value is object => !!value && typeof value === 'object')
}

/**
 * 沿路径读取嵌套字段，等价于 `value?.a?.b?.[0]`：
 * 字符串键只在普通对象上读取，数字键只在数组上读取，任一层不匹配都返回 undefined
 */
export function dig(value: unknown, ...path: ReadonlyArray<string | number>): unknown {
  let current = value
  for (const key of path) {
    if (typeof key === 'number') {
      if (!Array.isArray(current)) return undefined
      current = current[key]
    } else {
      if (!isJsonRecord(current)) return undefined
      current = current[key]
    }
  }
  return current
}
