type Rgb = readonly [number, number, number]

/** 播放页动态主题 CSS 变量；没有取色结果时为空对象，回落到样式表默认值 */
export type DynamicColorVars = Partial<Record<
  | '--np-primary'
  | '--np-on-primary'
  | '--np-primary-container'
  | '--np-on-surface'
  | '--np-on-surface-variant'
  | '--waveform-thumb-color',
  string
>>

// AccentBackdrop 底色（对齐 Android：主色降饱和调暗后铺底）
// 强制压暗，保证白字在亮封面/浅主题下仍可读
export function accentBackdropColor(bg: Rgb | null | undefined): string {
  if (!bg) return 'rgb(18, 18, 18)'
  const [r, g, b] = bg
  const luma = (r * 0.299 + g * 0.587 + b * 0.114) / 255
  if (luma <= 0.34) {
    return `rgb(${r}, ${g}, ${b})`
  }
  // 过亮时向中性深色混合，保留色相
  const t = Math.min(1, (luma - 0.34) / 0.4)
  const mix = 0.35 + t * 0.45
  return `rgb(${Math.round(r * (1 - mix) + 18 * mix)}, ${Math.round(g * (1 - mix) + 18 * mix)}, ${Math.round(b * (1 - mix) + 18 * mix)})`
}

// 动态主题 CSS 变量（对齐 Android M3 动态配色）
export function buildDynamicColorVars(lv: Rgb): DynamicColorVars {
  // RGB -> HSL 转换
  const r = lv[0] / 255, g = lv[1] / 255, b = lv[2] / 255
  const max = Math.max(r, g, b), min = Math.min(r, g, b)
  let h = 0, s = 0
  const l = (max + min) / 2
  if (max !== min) {
    const d = max - min
    s = l > 0.5 ? d / (2 - max - min) : d / (max + min)
    if (max === r) h = ((g - b) / d + (g < b ? 6 : 0)) / 6
    else if (max === g) h = ((b - r) / d + 2) / 6
    else h = ((r - g) / d + 4) / 6
  }

  // HSL -> RGB
  const hsl2rgb = (h: number, s: number, l: number): [number, number, number] => {
    if (s === 0) return [Math.round(l * 255), Math.round(l * 255), Math.round(l * 255)]
    const hue2rgb = (p: number, q: number, t: number) => {
      if (t < 0) t += 1; if (t > 1) t -= 1
      if (t < 1/6) return p + (q - p) * 6 * t
      if (t < 1/2) return q
      if (t < 2/3) return p + (q - p) * (2/3 - t) * 6
      return p
    }
    const q = l < 0.5 ? l * (1 + s) : l + s - l * s
    const pp = 2 * l - q
    return [
      Math.round(hue2rgb(pp, q, h + 1/3) * 255),
      Math.round(hue2rgb(pp, q, h) * 255),
      Math.round(hue2rgb(pp, q, h - 1/3) * 255),
    ]
  }

  // 深色播放页使用柔和的亮强调色，避免蓝色封面生成过艳、偏暗的控件
  const primaryS = Math.min(0.65, s)
  const primaryL = 0.80
  const [pr, pg, pb] = hsl2rgb(h, primaryS, primaryL)
  const primary = `rgb(${pr}, ${pg}, ${pb})`

  // 播放按钮用更浅的同色容器，中性取色保持中性
  const pcS = Math.min(0.50, s * 0.8)
  const pcL = 0.88
  const [pcr, pcg, pcb] = hsl2rgb(h, pcS, pcL)
  const primaryContainer = `rgb(${pcr}, ${pcg}, ${pcb})`

  // 主色上文字：基于 primaryContainer 亮度选深/浅色
  const pcLuma = pcr * 0.299 + pcg * 0.587 + pcb * 0.114
  const onPrimary = pcLuma > 140 ? 'rgb(20, 18, 24)' : 'rgb(255, 255, 255)'

  return {
    '--np-primary': primary,
    '--np-on-primary': onPrimary,
    '--np-primary-container': primaryContainer,
    '--np-on-surface': 'rgba(255,255,255,1)',
    '--np-on-surface-variant': 'rgba(255,255,255,0.78)',
    '--waveform-thumb-color': primary,
  }
}
