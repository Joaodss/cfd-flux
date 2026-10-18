// PNG import (FR-02): an image becomes a mask of cells — the dark pixels, or the opaque ones —
// resampled (nearest) to a width in cells. Image rows run top-down; mask rows bottom-up (03 §3).

export interface MaskOptions {
  /** `dark`: luminance below the threshold; `alpha`: opacity above it. */
  mode: 'dark' | 'alpha'
  /** 0–1. */
  threshold: number
  invert: boolean
  /** Width of the mask in cells; the height keeps the image's aspect ratio. */
  width: number
}

export interface Mask {
  width: number
  height: number
  /** 1 where the cell is painted, row-major from the bottom row. */
  cells: Uint8Array
}

/** The mask of an RGBA image (`data` as from `getImageData`). */
export function imageMask(data: Uint8ClampedArray, w: number, h: number, o: MaskOptions): Mask {
  const width = Math.max(1, Math.round(o.width))
  const height = Math.max(1, Math.round((h * width) / w))
  const cells = new Uint8Array(width * height)
  const t = o.threshold * 255
  for (let y = 0; y < height; y++) {
    // Cell row y (from the bottom) samples image row iy (from the top), at the cell centre.
    const iy = Math.min(h - 1, Math.floor(((height - 1 - y + 0.5) * h) / height))
    for (let x = 0; x < width; x++) {
      const ix = Math.min(w - 1, Math.floor(((x + 0.5) * w) / width))
      const p = (iy * w + ix) * 4
      const a = data[p + 3]
      let on: boolean
      if (o.mode === 'alpha') {
        on = a > t
      } else {
        // Rec. 709 luminance; transparent pixels count as white (background).
        const lum = 0.2126 * data[p] + 0.7152 * data[p + 1] + 0.0722 * data[p + 2]
        on = a > 127 && lum < t
      }
      cells[y * width + x] = on !== o.invert ? 1 : 0
    }
  }
  return { width, height, cells }
}
