// The fluid fraction of every block of a downsampled frame. Samples are 0 at non-fluid cells and
// the box filter averages them in, so a block next to a wall reads low; dividing by the block's
// fluid fraction gives exactly the mean over its fluid cells (design proposal §4.1).

/** The box-filter factor of a stream format: `ceil(max(width, height) / maxSize)` (as the server). */
export function boxFactor(width: number, height: number, maxSize: number): number {
  return Math.max(1, Math.ceil(Math.max(width, height) / Math.max(1, maxSize)))
}

/**
 * Fluid fraction per block of `k × k` cells (partial blocks at the edges count the cells they
 * have), from the cell-type layer (0 = fluid). Size `ceil(width / k) × ceil(height / k)`.
 */
export function fluidFraction(
  cellType: Uint8Array,
  width: number,
  height: number,
  k: number,
): Float32Array {
  const bw = Math.ceil(width / k)
  const bh = Math.ceil(height / k)
  const fluid = new Float32Array(bw * bh)
  const cells = new Float32Array(bw * bh)
  for (let y = 0; y < height; y++) {
    const row = Math.floor(y / k) * bw
    for (let x = 0; x < width; x++) {
      const b = row + Math.floor(x / k)
      cells[b]++
      if (cellType[y * width + x] === 0) fluid[b]++
    }
  }
  for (let i = 0; i < fluid.length; i++) fluid[i] /= cells[i]
  return fluid
}
