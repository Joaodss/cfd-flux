// Colour maps, generated from Rust's `cfd_io::image::Colormap` (`cfd-cli colormaps`), so the
// viewer and `cfd-cli render` paint the same colours: 9 samples per map, linearly interpolated.
import maps from '../generated/colormaps.json' with { type: 'json' }

export interface Colormap {
  name: string
  diverging: boolean
  samples: readonly (readonly number[])[]
}

export const COLORMAPS: readonly Colormap[] = maps

export function colormap(name: string): Colormap {
  return COLORMAPS.find((m) => m.name === name) ?? COLORMAPS[0]
}

/** The colour at `t ∈ [0, 1]` (clamped; NaN → 0), as Rust's `Colormap::rgb` computes it. */
export function colourAt(map: Colormap, t: number): [number, number, number] {
  const s = map.samples
  const c = Number.isNaN(t) ? 0 : Math.min(1, Math.max(0, t))
  const x = c * (s.length - 1)
  const i = Math.min(Math.floor(x), s.length - 2)
  const f = x - i
  return [0, 1, 2].map((k) => Math.round(s[i][k] + (s[i + 1][k] - s[i][k]) * f)) as [
    number,
    number,
    number,
  ]
}

/** A 256-entry RGBA lookup table for a 256 × 1 texture (entry i is the colour at i / 255). */
export function lut(map: Colormap): Uint8Array {
  const out = new Uint8Array(256 * 4)
  for (let i = 0; i < 256; i++) {
    out.set([...colourAt(map, i / 255), 255], i * 4)
  }
  return out
}

/** A CSS gradient through the samples, left to right (the legend's colour bar). */
export function cssGradient(map: Colormap): string {
  const n = map.samples.length - 1
  const stops = map.samples.map(([r, g, b], i) => `rgb(${r} ${g} ${b}) ${(100 * i) / n}%`)
  return `linear-gradient(to right, ${stops.join(', ')})`
}
