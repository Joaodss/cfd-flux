// The viewer's imperative core (no React): the decode worker, the current frame (typed arrays),
// the run's cells and the WebGL renderer. React components drive it and read `useViewer`.
import { CELL_COLOURS } from '../editor/colours.ts'
import type { Rgba } from '../editor/colours.ts'
import { FrameDecoder } from '../frames/decoder.ts'
import { halfToFloat } from '../frames/half.ts'
import type { DecodedFrame } from '../frames/lffr.ts'
import type { Decoded } from '../frames/protocol.ts'
import type { Scene } from '../generated/scene.ts'
import { fromScene } from '../scene/io.ts'
import { colormap, lut } from './colormaps.ts'
import { boxFactor, fluidFraction } from './mask.ts'
import { componentValue } from './quantities.ts'
import { FieldRenderer, type ViewerColours } from './render/fieldRenderer.ts'
import { colormapOf, currentQuantity, currentRange, useViewer, type ViewerState } from './store.ts'

const COLOURS: ViewerColours = {
  // Fluid cells before the first frame: a neutral dark grey.
  cellTypes: [[63, 63, 70, 255], ...CELL_COLOURS.slice(1)] as Rgba[],
  background: [24, 24, 27, 255],
  border: [113, 113, 122, 255],
  noData: [255, 0, 255, 255],
}

/** The run's cells and the blocks of the frames being shown. */
interface Cells {
  cellType: Uint8Array
  width: number
  height: number
  /** Blocks of the frames: size and box factor, with the fluid fraction of each block. */
  blocks: { width: number; height: number; k: number }
  fraction: Float32Array
}

export class Viewer {
  private readonly renderer: FieldRenderer
  private readonly decoder: FrameDecoder
  private cells: Cells | null = null
  private current: DecodedFrame | null = null
  /** A frame that arrived before the scene: shown once the cells are known. */
  private pending: ArrayBuffer | null = null
  /** What the textures hold, to upload only on change. */
  private uploaded: { frame: DecodedFrame | null; field: string | null; lut: string | null } = {
    frame: null,
    field: null,
    lut: null,
  }
  private frameRequest = 0
  private readonly unsubscribe: () => void
  private disposed = false

  constructor(canvas: HTMLCanvasElement, decoder?: FrameDecoder) {
    this.renderer = new FieldRenderer(canvas)
    this.decoder = decoder ?? new FrameDecoder()
    this.unsubscribe = useViewer.subscribe(() => this.schedule())
  }

  /** The run's scene: its cells give the solid mask; `maxSize` is the stream format's. */
  async setScene(scene: Scene, maxSize: number): Promise<void> {
    const doc = await fromScene(scene)
    const { width, height } = doc.layers
    this.setBlocks(doc.layers.cellType, width, height, boxFactor(width, height, maxSize))
    useViewer.getState().setGrid({ width, height, cellSize: scene.grid.cellSize })
    const pending = this.pending
    this.pending = null
    if (pending) this.pushFrame(pending)
  }

  private setBlocks(cellType: Uint8Array, width: number, height: number, k: number): void {
    const blocks = { width: Math.ceil(width / k), height: Math.ceil(height / k), k }
    const fraction = fluidFraction(cellType, width, height, k)
    this.cells = { cellType, width, height, blocks, fraction }
    this.renderer.setCells(cellType, width, height, fraction, blocks)
    this.decoder.setMask(fraction)
    this.uploaded = { frame: null, field: null, lut: this.uploaded.lut }
  }

  /** A compressed frame from the stream (moved to the worker). */
  pushFrame(buffer: ArrayBuffer): void {
    if (!this.cells) {
      this.pending = buffer
      return
    }
    this.decoder.decode(buffer).then(
      (d) => this.show(d),
      (e: Error) => console.warn(`frame skipped: ${e.message}`),
    )
  }

  /** Shows a decoded frame. */
  show({ frame, stats }: Decoded): void {
    if (this.disposed || !this.cells) return
    const { header } = frame
    const c = this.cells
    if (header.width !== c.blocks.width || header.height !== c.blocks.height) {
      // A different format than expected: derive its box factor from its size.
      const k = Math.max(1, Math.ceil(c.width / header.width))
      this.setBlocks(c.cellType, c.width, c.height, k)
    }
    this.current = frame
    useViewer.getState().setFrame(header, stats)
  }

  /** The current frame's header (the timeline) or null. */
  get frame(): DecodedFrame | null {
    return this.current
  }

  /** The value of the shown quantity at a cell (null outside the fluid or before a frame). */
  valueAt(x: number, y: number): number | null {
    const c = this.cells
    const s = useViewer.getState()
    const q = currentQuantity(s)
    const f = this.current?.fields.find((f) => f.name === q?.field)
    if (!c || !q || !f || x < 0 || y < 0 || x >= c.width || y >= c.height) return null
    if (c.cellType[y * c.width + x] !== 0) return null
    const { k, width } = c.blocks
    const b = Math.floor(y / k) * width + Math.floor(x / k)
    const frac = c.fraction[b]
    if (!(frac > 0)) return null
    const read = (j: number) => (f.dtype === 'f16' ? halfToFloat(f.data[j]) : f.data[j])
    const n = f.components
    const v = componentValue(q.component, read(b * n), n > 1 ? read(b * n + 1) : 0)
    return v / frac // the mean over the block's fluid cells (for |u| too: |a/f, b/f| = |a, b|/f)
  }

  resize(cssWidth: number, cssHeight: number, dpr: number): void {
    this.renderer.resize(cssWidth, cssHeight, dpr)
    this.schedule()
  }

  /** The canvas pixel (RGBA) at a CSS position: tests read the image through it. */
  pixelAt(cssX: number, cssY: number): [number, number, number, number] {
    const dpr = window.devicePixelRatio
    this.draw()
    return this.renderer.pixel(Math.floor(cssX * dpr), Math.floor(cssY * dpr))
  }

  /** Draws on the next animation frame (at most once per frame). */
  schedule(): void {
    if (this.frameRequest === 0 && !this.disposed) {
      this.frameRequest = requestAnimationFrame(() => {
        this.frameRequest = 0
        this.draw()
      })
    }
  }

  private draw(): void {
    const s: ViewerState = useViewer.getState()
    const q = currentQuantity(s)
    const frame = this.current
    if (q && frame) {
      const field = frame.fields.find((f) => f.name === q.field)
      if (field && (this.uploaded.frame !== frame || this.uploaded.field !== q.field)) {
        this.renderer.setField(field, frame.header.width, frame.header.height)
        this.uploaded.frame = frame
        this.uploaded.field = q.field
      }
      const map = colormapOf(s, q)
      if (this.uploaded.lut !== map) {
        this.renderer.setLut(lut(colormap(map)))
        this.uploaded.lut = map
      }
    }
    this.renderer.draw(
      s.view,
      window.devicePixelRatio,
      {
        component: q?.component ?? 'scalar',
        range: currentRange(s) ?? [0, 1],
        nearest: s.nearest,
      },
      COLOURS,
    )
  }

  dispose(): void {
    this.disposed = true
    cancelAnimationFrame(this.frameRequest)
    this.unsubscribe()
    this.decoder.dispose()
    this.renderer.dispose()
  }
}
