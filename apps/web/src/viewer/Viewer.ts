// The viewer's imperative core (no React): the decode worker, the current frame (typed arrays),
// the run's cells and the WebGL renderer. React components drive it and read `useViewer`.
import { CELL_COLOURS } from '../editor/colours.ts'
import type { Rgba } from '../editor/colours.ts'
import { cellToScreen } from '../editor/view.ts'
import { FrameCache } from '../frames/cache.ts'
import { DecodeError, FrameDecoder } from '../frames/decoder.ts'
import { halfToFloat } from '../frames/half.ts'
import type { DecodedFrame } from '../frames/lffr.ts'
import type { Decoded } from '../frames/protocol.ts'
import type { Scene } from '../generated/scene.ts'
import { fromScene } from '../scene/io.ts'
import { colormap, lut } from './colormaps.ts'
import { boxFactor, fluidFraction } from './mask.ts'
import { componentValue } from './quantities.ts'
import { FieldRenderer, type Overlays, type ViewerColours } from './render/fieldRenderer.ts'
import { colormapOf, currentQuantity, currentRange, useViewer, type ViewerState } from './store.ts'
import { advance, prefetch } from './timeline.ts'
import { ARROW_SPACING, arrowScale } from './vectors.ts'

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
  private readonly cache: FrameCache
  private cells: Cells | null = null
  private current: DecodedFrame | null = null
  /** Frames that arrived before the scene: shown once the cells are known. */
  private pending: ArrayBuffer[] = []
  /** Where stored frames come from: the job and the viewer format's longest side. */
  private source: { jobId: string; maxSize: number } | null = null
  /** Whether the run still produces frames. */
  private running = true
  private readonly inFlight = new Map<number, Promise<Decoded | null>>()
  /** What the textures hold, to upload only on change. */
  private uploaded: {
    frame: DecodedFrame | null
    velocity: DecodedFrame | null
    field: string | null
    lut: string | null
  } = {
    frame: null,
    velocity: null,
    field: null,
    lut: null,
  }
  /** The streamlines drawn: the frame they belong to, and the request in the worker. */
  private streamlines: { frame: DecodedFrame | null; index: number; busy: boolean } = {
    frame: null,
    index: -1,
    busy: false,
  }
  private overlays = useViewer.getState().overlays
  private frameRequest = 0
  /** Playback: the fractional frame position and the time of the last animation frame. */
  private playback = { position: 0, time: 0, request: 0 }
  private readonly unsubscribe: () => void
  private disposed = false

  constructor(canvas: HTMLCanvasElement, decoder?: FrameDecoder, cache?: FrameCache) {
    this.renderer = new FieldRenderer(canvas)
    this.decoder = decoder ?? new FrameDecoder()
    this.cache = cache ?? new FrameCache()
    this.unsubscribe = useViewer.subscribe((s) => {
      if (s.overlays !== this.overlays) {
        const streamlinesChanged = s.overlays.streamlines !== this.overlays.streamlines
        this.overlays = s.overlays
        if (streamlinesChanged) this.updateStreamlines(true)
      }
      this.schedule()
    })
  }

  /**
   * The run: its scene (the cells give the solid mask), its job (stored frames) and the viewer
   * format's longest side.
   */
  async setScene(scene: Scene, jobId: string, maxSize: number): Promise<void> {
    const doc = await fromScene(scene)
    const { width, height } = doc.layers
    this.source = { jobId, maxSize }
    this.setBlocks(doc.layers.cellType, width, height, boxFactor(width, height, maxSize))
    useViewer.getState().setGrid({ width, height, cellSize: scene.grid.cellSize })
    const pending = this.pending
    this.pending = []
    for (const b of pending) this.pushFrame(b)
    const t = useViewer.getState().timeline
    if (t.index === null && t.last >= 0 && !this.running) this.seek(t.last)
  }

  private setBlocks(cellType: Uint8Array, width: number, height: number, k: number): void {
    const blocks = { width: Math.ceil(width / k), height: Math.ceil(height / k), k }
    const fraction = fluidFraction(cellType, width, height, k)
    this.cells = { cellType, width, height, blocks, fraction }
    this.renderer.setCells(cellType, width, height, fraction, blocks)
    this.decoder.setMask(fraction)
    this.decoder.setCells(cellType, width, height)
    this.streamlines = { frame: null, index: -1, busy: this.streamlines.busy }
    this.renderer.setStreamlines(null)
    this.uploaded = { frame: null, velocity: null, field: null, lut: this.uploaded.lut }
  }

  /** A compressed frame from the stream (moved to the worker). */
  pushFrame(buffer: ArrayBuffer): void {
    if (!this.cells) {
      this.pending.push(buffer)
      return
    }
    this.decoder.decode(buffer).then(
      (d) => this.onLive(d),
      (e: Error) => console.warn(`frame skipped: ${e.message}`),
    )
  }

  /** A live frame: cached; shown when following the run. */
  private onLive(d: Decoded): void {
    if (this.disposed) return
    const index = d.frame.header.frameIndex
    const s = useViewer.getState()
    const t = s.timeline
    this.cache.put(index, d, t.index ?? index)
    if (index > t.last) s.setTimeline({ last: index })
    // Following the run, or the frame asked for at the end of a live run (`setRun`).
    if ((t.mode === 'live' && index >= (t.index ?? -1)) || (t.buffering && t.index === index)) {
      this.display(index, d)
    }
  }

  /**
   * What the job reports: whether it still runs, and its output count once known (a finished
   * job's stream sends no frames: the last stored one is shown).
   */
  setRun(running: boolean, frames: number | null): void {
    this.running = running
    const s = useViewer.getState()
    if (frames !== null && frames - 1 > s.timeline.last) s.setTimeline({ last: frames - 1 })
    const t = useViewer.getState().timeline
    if (!running && t.mode === 'live') {
      // The run is over: hold its last frame. Live frames may still be in the worker (a fast run
      // ends before they are decoded), so ask for the last one: from the cache, the stream
      // (`onLive`) or the server, whichever comes first.
      s.setTimeline({ mode: 'paused' })
      if (t.last >= 0 && this.cells) this.request(t.last)
    }
  }

  private display(index: number, d: Decoded): void {
    if (this.disposed || !this.cells) return
    const { header } = d.frame
    const c = this.cells
    if (header.width !== c.blocks.width || header.height !== c.blocks.height) {
      // A different format than expected: derive its box factor from its size.
      const k = Math.max(1, Math.ceil(c.width / header.width))
      this.setBlocks(c.cellType, c.width, c.height, k)
    }
    this.current = d.frame
    const s = useViewer.getState()
    s.setFrame(header, d.stats)
    s.setTimeline({ index, buffering: false, note: null })
    this.updateStreamlines(false)
  }

  /**
   * Recomputes the streamlines for the shown frame (in the worker) when they are on and out of
   * date: at once when paused or toggled on; while playing, every few frames, so the worker keeps
   * up. Only one request runs at a time; the frame shown when it ends is looked at again.
   */
  private updateStreamlines(force: boolean): void {
    const s = useViewer.getState()
    if (!s.overlays.streamlines) {
      this.streamlines = { frame: null, index: -1, busy: this.streamlines.busy }
      this.renderer.setStreamlines(null)
      this.schedule()
      return
    }
    const frame = this.current
    const c = this.cells
    const velocity = frame?.fields.find((f) => f.name === 'velocity' && f.components === 2)
    if (!frame || !c || !velocity || this.streamlines.busy || this.streamlines.frame === frame) {
      return
    }
    const index = frame.header.frameIndex
    const playing = s.timeline.mode !== 'paused'
    if (
      playing &&
      !force &&
      this.streamlines.frame &&
      Math.abs(index - this.streamlines.index) < 4
    ) {
      return
    }
    this.streamlines.busy = true
    const k = c.blocks.k
    const data = velocity.data.slice() // the page keeps its own copy of the frame
    const separation = Math.max(3, Math.min(c.width, c.height) / 24)
    void this.decoder
      .streamlines(
        { data, dtype: velocity.dtype, k },
        { separation, step: Math.max(0.5, k * 0.5), maxLength: 1.5 * Math.max(c.width, c.height) },
      )
      .then(
        (r) => {
          this.streamlines = { frame, index, busy: false }
          if (this.disposed) return
          if (useViewer.getState().overlays.streamlines) this.renderer.setStreamlines(r.segments)
          this.schedule()
          this.updateStreamlines(false) // the frame shown may have changed meanwhile
        },
        (e: Error) => {
          this.streamlines.busy = false
          console.warn('streamlines skipped:', e.message)
        },
      )
  }

  /** A frame from the cache or the server (one request per frame at a time). */
  private load(index: number): Promise<Decoded | null> {
    const cached = this.cache.get(index)
    if (cached) return Promise.resolve(cached)
    const pending = this.inFlight.get(index)
    if (pending) return pending
    if (!this.source) return Promise.resolve(null)
    const { jobId, maxSize } = this.source
    const url = `/api/simulations/${encodeURIComponent(jobId)}/frames/${index}?dtype=f16&maxSize=${maxSize}`
    const p = this.decoder.fetch(url).then(
      (d) => {
        this.inFlight.delete(index)
        if (this.disposed) return null
        this.cache.put(index, d, useViewer.getState().timeline.index ?? index)
        return d
      },
      (e: DecodeError) => {
        this.inFlight.delete(index)
        if (!this.disposed) {
          useViewer.getState().setTimeline({
            buffering: false,
            note:
              e.status === 404
                ? `Frame ${index} was not stored (only live frames can be shown)`
                : `Frame ${index}: ${e.message}`,
          })
        }
        return null
      },
    )
    this.inFlight.set(index, p)
    return p
  }

  /** Shows frame `index` (fetching it if needed) once it is there, if it is still wanted. */
  private request(index: number): void {
    const cached = this.cache.get(index)
    if (cached) {
      this.display(index, cached)
      return
    }
    useViewer.getState().setTimeline({ index, buffering: true })
    void this.load(index).then((d) => {
      if (d && useViewer.getState().timeline.index === index) this.display(index, d)
    })
  }

  /** Goes to a frame and pauses there. */
  seek(index: number): void {
    const t = useViewer.getState().timeline
    if (t.last < 0) return
    this.stopLoop()
    useViewer.getState().setTimeline({ mode: 'paused' })
    this.request(Math.max(0, Math.min(t.last, Math.round(index))))
  }

  step(delta: number): void {
    const t = useViewer.getState().timeline
    this.seek((t.index ?? t.last) + delta)
  }

  /** Follows the newest frame (while the run goes on). */
  goLive(): void {
    this.stopLoop()
    const t = useViewer.getState().timeline
    useViewer.getState().setTimeline({ mode: this.running ? 'live' : 'paused' })
    if (t.last >= 0) this.request(t.last)
  }

  play(): void {
    const t = useViewer.getState().timeline
    if (t.last < 0) return
    const start = t.index === null || (t.index >= t.last && !this.running) ? 0 : t.index
    this.stopLoop()
    this.playback.position = start
    this.playback.time = performance.now()
    useViewer.getState().setTimeline({ mode: 'playing' })
    this.request(start)
    this.playback.request = requestAnimationFrame(this.tick)
  }

  pause(): void {
    this.stopLoop()
    useViewer.getState().setTimeline({ mode: 'paused' })
  }

  private stopLoop(): void {
    cancelAnimationFrame(this.playback.request)
    this.playback.request = 0
  }

  /** One animation frame of playback: advance unless waiting for a frame, then prefetch. */
  private readonly tick = (now: number): void => {
    const s = useViewer.getState()
    const t = s.timeline
    if (this.disposed || t.mode !== 'playing') return
    const elapsed = (now - this.playback.time) / 1000
    this.playback.time = now
    if (!t.buffering) {
      const next = advance(this.playback.position, elapsed, t, this.running)
      this.playback.position = next.position
      const index = Math.floor(next.position)
      if (index !== t.index) this.request(index)
      if (next.mode === 'live') {
        this.goLive()
        return
      }
      if (next.mode === 'paused') {
        s.setTimeline({ mode: 'paused' })
        return
      }
      for (const i of prefetch(index, t.last)) void this.load(i)
    }
    this.playback.request = requestAnimationFrame(this.tick)
  }

  /** Streamline segments being drawn (tests). */
  get streamlineSegments(): number {
    return this.renderer.streamlineCount
  }

  /** The canvas as a PNG data URL (tests compare images). */
  snapshot(): string {
    this.draw()
    return this.renderer.canvas.toDataURL()
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

  /** The canvas pixel at the centre of cell (x, y) at the current view (tests). */
  pixelAtCell(x: number, y: number): [number, number, number, number] {
    const { view, grid } = useViewer.getState()
    const [sx, sy] = cellToScreen(view, grid?.height ?? 0, x + 0.5, y + 0.5)
    return this.pixelAt(sx, sy)
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
    const velocity = frame?.fields.find((f) => f.name === 'velocity' && f.components === 2)
    if (frame && velocity && s.overlays.vectors && this.uploaded.velocity !== frame) {
      this.renderer.setVelocity(velocity, frame.header.width, frame.header.height)
      this.uploaded.velocity = frame
    }
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
      this.overlayStyle(s),
    )
  }

  /** The overlays to draw: arrows scaled to the fastest speed seen so far, translucent lines. */
  private overlayStyle(s: ViewerState): Overlays {
    const dpr = window.devicePixelRatio
    const arrows = s.overlays.vectors ? arrowScale(s.seen['velocity']?.max ?? 0) : null
    return {
      vectors: arrows
        ? {
            spacing: ARROW_SPACING * dpr,
            scale: arrows.scale * dpr,
            colour: [1, 1, 1, 0.9],
          }
        : null,
      streamlines: s.overlays.streamlines ? [1, 1, 1, 0.55] : null,
    }
  }

  dispose(): void {
    this.disposed = true
    cancelAnimationFrame(this.frameRequest)
    this.stopLoop()
    this.cache.clear()
    this.unsubscribe()
    this.decoder.dispose()
    this.renderer.dispose()
  }
}
