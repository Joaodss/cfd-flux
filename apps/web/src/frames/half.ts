// Half floats (IEEE 754 binary16). The viewer keeps f16 fields as their raw bits in a
// `Uint16Array` (WebGL uploads them as-is to R16F/RG16F textures) and converts to numbers only
// where it needs them (ranges, the value under the cursor) through a table of all 65,536 values.

let table: Float32Array | null = null

/** All 65,536 f16 values as f32, built on first use (256 kB). */
export function halfTable(): Float32Array {
  if (table) return table
  const t = new Float32Array(65536)
  for (let h = 0; h < 65536; h++) {
    const sign = h & 0x8000 ? -1 : 1
    const exp = (h >> 10) & 0x1f
    const frac = h & 0x3ff
    t[h] =
      exp === 0
        ? sign * frac * 2 ** -24 // zero and subnormals
        : exp === 31
          ? frac === 0
            ? sign * Infinity
            : NaN
          : sign * (1 + frac / 1024) * 2 ** (exp - 15)
  }
  table = t
  return t
}

/** The value of one f16 given by its bits. */
export function halfToFloat(bits: number): number {
  return halfTable()[bits & 0xffff]
}
