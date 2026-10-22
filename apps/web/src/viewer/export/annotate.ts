// What exported images and videos carry besides the field: a footer strip with the colour bar and
// its ticks, the unit, the quantity and the time of the frame (design proposal §8).
import { formatValue } from '../../panels/format.ts'
import { ticks } from '../ranges.ts'

/** Height of the footer strip, in pixels. */
export const FOOTER_HEIGHT = 48
/** The footer needs this much width for its colour bar and text. */
export const FOOTER_MIN_WIDTH = 360

export interface Annotation {
  /** 256 RGBA entries of the colour map. */
  lut: Uint8Array
  range: [number, number]
  unit: string
  label: string
  /** Simulated time (s), the frame index and the step. */
  time: number
  frame: number
  step: number
}

/** The text of the footer: the quantity on the left, the frame on the right, five tick labels. */
export function footerText(a: Annotation): { left: string; right: string; ticks: string[] } {
  return {
    left: a.unit ? `${a.label} (${a.unit})` : a.label,
    right: `t = ${formatValue(a.time)} s · frame ${a.frame} · step ${a.step.toLocaleString('en')}`,
    ticks: ticks(a.range).map((t) => formatValue(t)),
  }
}

/** The size of an image with its footer. */
export function annotatedSize(width: number, height: number): { width: number; height: number } {
  return { width: Math.max(width, FOOTER_MIN_WIDTH), height: height + FOOTER_HEIGHT }
}

/**
 * A new canvas with `image` on top and the footer below it. `image` is the field's canvas
 * (a WebGL canvas read straight after drawing, which `preserveDrawingBuffer` allows).
 */
export function annotate(
  image: HTMLCanvasElement,
  a: Annotation,
  font = '12px system-ui, sans-serif',
): HTMLCanvasElement {
  const size = annotatedSize(image.width, image.height)
  const out = document.createElement('canvas')
  out.width = size.width
  out.height = size.height
  const ctx = out.getContext('2d')!
  ctx.fillStyle = '#18181b'
  ctx.fillRect(0, 0, out.width, out.height)
  ctx.drawImage(image, 0, 0)
  const t = footerText(a)
  const y = image.height
  // The colour bar: 256 columns of the map.
  const barX = 10
  const barW = 220
  for (let i = 0; i < barW; i++) {
    const e = Math.min(255, Math.floor((i / (barW - 1)) * 255)) * 4
    ctx.fillStyle = `rgb(${a.lut[e]} ${a.lut[e + 1]} ${a.lut[e + 2]})`
    ctx.fillRect(barX + i, y + 8, 1, 10)
  }
  ctx.font = font
  ctx.fillStyle = '#d4d4d8'
  ctx.textBaseline = 'top'
  ctx.textAlign = 'center'
  t.ticks.forEach((s, i) => ctx.fillText(s, barX + (i / 4) * barW, y + 21))
  ctx.textAlign = 'left'
  ctx.fillText(t.left, barX + barW + 16, y + 8)
  ctx.textAlign = 'right'
  ctx.fillText(t.right, out.width - 10, y + 28)
  return out
}
