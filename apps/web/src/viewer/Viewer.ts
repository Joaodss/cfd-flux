// The viewer's imperative core (no React): the decode worker, the current frame (typed arrays),
// the run's cells and the WebGL renderer. React components drive it and read `useViewer`.
import { CELL_COLOURS } from '../editor/colours.ts'
import type { Rgba } from '../editor/colours.ts'
import { cellToScreen, type View } from '../editor/view.ts'
import { FrameCache } from '../frames/cache.ts'
import { DecodeError, FrameDecoder } from '../frames/decoder.ts'
import { halfToFloat } from '../frames/half.ts'
import type { DecodedFrame } from '../frames/lffr.ts'
import type { Decoded } from '../frames/protocol.ts'
import type { Scene } from '../generated/scene.ts'
import { mark, measureSince } from '../perf.ts'
import { fromScene } from '../scene/io.ts'
import { annotate, type Annotation } from './export/annotate.ts'
import { frameCsv } from './export/csv.ts'
import { encodeVideo, type Quality, type VideoResult } from './export/video.ts'
import { colormap, lut } from './colormaps.ts'
import { boxFactor, fluidFraction } from './mask.ts'
import { componentValue } from './quantities.ts'
import { FieldRenderer, type Overlays, type ViewerColours } from './render/fieldRenderer.ts'
import { colormapOf, currentQuantity, currentRange, useViewer, type ViewerState } from './store.ts'
import { advance, prefetch } from './timeline.ts'
import { fullFrameBytes, needsSharpen, type SharpenState } from './sharpen.ts'
import { inletSpawns, simulatedStep, substeps } from './simulation.ts'
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
  /**
   * Bumped when the viewer format changes: frames asked for before (live ones still in a worker,
   * stored ones still being fetched) are of the old format and are dropped.
   */
  private generation = 0
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
  /**
   * The paused frame at full resolution ("sharpen"): the frame it sharpens and its fields at
   * every cell; one request at a time; the frame whose request failed (not asked again).
   */
  private sharp: { of: DecodedFrame; frame: DecodedFrame } | null = null
  private sharpening = false
  private sharpFailed: DecodedFrame | null = null
  private overlays = useViewer.getState().overlays
  /** Particles and dye: the animation loop, simulated seconds between frames, queued brush stamps. */
  private sim = {
    request: 0,
    last: 0,
    frameInterval: 0,
    stamps: [] as [number, number, number, number][],
  }
  private marked: unknown = null
  /** A video is being recorded: the live animation leaves the particles and dye alone. */
  private exporting = false
  private frameRequest = 0
  /** Playback: the fractional frame position and the time of the last animation frame. */
  private playback = { position: 0, time: 0, request: 0 }
  private readonly unsubscribe: () => void
  private disposed = false

  constructor(canvas: HTMLCanvasElement, decoder?: FrameDecoder, cache?: FrameCache) {
    this.renderer = new FieldRenderer(canvas)
    useViewer.getState().setFloatTargets(this.renderer.floatTargets)
    this.decoder = decoder ?? new FrameDecoder()
    this.cache = cache ?? new FrameCache()
    this.unsubscribe = useViewer.subscribe((s) => {
      if (s.overlays !== this.overlays) {
        const streamlinesChanged = s.overlays.streamlines !== this.overlays.streamlines
        this.overlays = s.overlays
        if (streamlinesChanged) this.updateStreamlines(true)
        this.syncAnimation()
      }
      this.updateSharpen()
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

  /**
   * Another viewer format (a setting): the cache is emptied, the blocks and their fluid
   * fraction rebuilt, and the playhead's frame loaded again in the new format. The caller
   * reconnects the stream with the same `maxSize`.
   */
  setMaxSize(maxSize: number): void {
    // Before the scene there is nothing to rebuild: `setScene` brings the format.
    if (!this.source || this.source.maxSize === maxSize) return
    this.source = { ...this.source, maxSize }
    this.generation++
    this.cache.clear()
    this.inFlight.clear()
    this.current = null
    const c = this.cells
    if (c) this.setBlocks(c.cellType, c.width, c.height, boxFactor(c.width, c.height, maxSize))
    this.schedule()
    const t = useViewer.getState().timeline
    if (t.last >= 0 && this.cells) this.request(t.index ?? t.last)
  }

  /** A new cache budget (a setting): frames over it are evicted at once. */
  setBudget(bytes: number): void {
    const t = useViewer.getState().timeline
    this.cache.setBudget(bytes, t.index ?? t.last)
  }

  /** The cache's budget in bytes. */
  get budget(): number {
    return this.cache.budget
  }

  private setBlocks(cellType: Uint8Array, width: number, height: number, k: number): void {
    const blocks = { width: Math.ceil(width / k), height: Math.ceil(height / k), k }
    const fraction = fluidFraction(cellType, width, height, k)
    this.cells = { cellType, width, height, blocks, fraction }
    const spawns = inletSpawns(cellType, width, height)
    this.renderer.setCells(cellType, width, height, fraction, blocks, spawns)
    // Without inlets (a closed cavity) particles are released anywhere in the fluid.
    if (spawns.length === 0 && useViewer.getState().release === 'inlets') {
      useViewer.getState().setRelease('domain')
    }
    this.decoder.setMask(fraction)
    this.decoder.setCells(cellType, width, height)
    this.streamlines = { frame: null, index: -1, busy: this.streamlines.busy }
    this.renderer.setStreamlines(null)
    this.uploaded = { frame: null, velocity: null, field: null, lut: this.uploaded.lut }
    this.sharp = null
  }

  /**
   * Shows the paused playhead's frame at full resolution when zoomed in past the viewer format
   * (`needsSharpen`), and goes back to the viewer format's frame otherwise. Looked at on every
   * change of the viewer's state: one fetch at a time, answers for a frame no longer shown are
   * dropped, and a frame larger than the cache budget is not fetched (the readout says so).
   */
  private updateSharpen(): void {
    const s = useViewer.getState()
    const c = this.cells
    const frame = this.current
    const want =
      !!c &&
      !!frame &&
      !!this.source &&
      !this.exporting &&
      needsSharpen({
        k: c.blocks.k,
        zoom: s.view.zoom,
        dpr: window.devicePixelRatio,
        mode: s.timeline.mode,
        buffering: s.timeline.buffering,
      })
    if (this.sharp && (!want || this.sharp.of !== frame)) {
      this.sharp = null
      this.schedule()
    }
    if (!want || !c || !frame || !this.source) {
      this.setSharpenState({ state: 'off' })
      return
    }
    if (this.sharp) return
    const bytes = fullFrameBytes(c.width, c.height, frame.header.fields)
    if (bytes > this.cache.budget) {
      const mb = (bytes / 2 ** 20).toFixed(0)
      this.setSharpenState({
        state: 'skipped',
        note: `full resolution needs ${mb} MiB, more than the frame cache`,
      })
      return
    }
    if (this.sharpFailed === frame) return
    if (this.sharpening) {
      // Looked at again when the request in flight ends.
      this.setSharpenState({ state: 'loading' })
      return
    }
    // Marked before the store hears of it: its listeners call this method again.
    this.sharpening = true
    this.setSharpenState({ state: 'loading' })
    const index = frame.header.frameIndex
    const names = frame.header.fields.map((f) => f.name).join(',')
    const url = `/api/simulations/${encodeURIComponent(this.source.jobId)}/frames/${index}?dtype=f16&fields=${encodeURIComponent(names)}`
    const generation = this.generation
    this.decoder.fetch(url, { stats: false }).then(
      (d) => {
        this.sharpening = false
        if (this.disposed) return
        const h = d.frame.header
        if (
          generation === this.generation &&
          this.current === frame &&
          h.width === c.width &&
          h.height === c.height
        ) {
          this.sharp = { of: frame, frame: d.frame }
          this.setSharpenState({ state: 'on', width: h.width, height: h.height })
          this.schedule()
        }
        this.updateSharpen() // the playhead or the view may have moved meanwhile
      },
      (e: Error) => {
        this.sharpening = false
        if (this.disposed) return
        this.sharpFailed = frame
        this.setSharpenState({ state: 'skipped', note: `full resolution: ${e.message}` })
        this.updateSharpen()
      },
    )
  }

  private setSharpenState(next: SharpenState): void {
    const s = useViewer.getState()
    if (JSON.stringify(s.sharpen) !== JSON.stringify(next)) s.setSharpen(next)
  }

  /** A compressed frame from the stream (moved to the worker). */
  pushFrame(buffer: ArrayBuffer): void {
    if (!this.cells) {
      this.pending.push(buffer)
      return
    }
    const generation = this.generation
    this.decoder.decode(buffer).then(
      (d) => this.onLive(d, generation),
      (e: Error) => console.warn(`frame skipped: ${e.message}`),
    )
  }

  /** A live frame: cached; shown when following the run. */
  private onLive(d: Decoded, generation: number): void {
    if (this.disposed || generation !== this.generation) return
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
    mark('viewer:frame', { index, mode: useViewer.getState().timeline.mode })
    const s = useViewer.getState()
    s.setFrame(header, d.stats)
    // Simulated seconds between frames, for the particles’ physical speed.
    if (header.frameIndex > 0 && header.simTime > 0) {
      this.sim.frameInterval = header.simTime / header.frameIndex
    }
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

  /** Starts or stops the animation loop that advects particles and dye. */
  private syncAnimation(): void {
    const { particles, dye } = useViewer.getState().overlays
    const want = (particles || dye) && !this.disposed && this.renderer.floatTargets
    if (want && this.sim.request === 0) {
      this.sim.last = performance.now()
      this.sim.request = requestAnimationFrame(this.animate)
    } else if (!want && this.sim.request !== 0) {
      cancelAnimationFrame(this.sim.request)
      this.sim.request = 0
    }
  }

  private readonly animate = (now: number): void => {
    this.sim.request = 0
    if (this.disposed) return
    if (this.exporting) {
      this.syncAnimation()
      return
    }
    this.stepSim((now - this.sim.last) / 1000)
    this.sim.last = now
    this.draw()
    this.syncAnimation()
  }

  /**
   * One animation frame of the particles and the dye: the shown frame's velocity moves them at the
   * playback's physical rate (frames per second × simulated time between frames).
   */
  private stepSim(wall: number): void {
    const { speed } = useViewer.getState().timeline
    this.stepSimBy(simulatedStep(wall, speed, this.sim.frameInterval))
  }

  /** Advances the particles and the dye by `dt` simulated seconds. */
  private stepSimBy(dt: number): void {
    const s = useViewer.getState()
    const { particles, dye } = s.overlays
    const g = s.grid
    if (!g || !this.current || !(dt > 0)) return
    this.ensureVelocity()
    const { count } = substeps(s.seen['velocity']?.max ?? 0, dt, g.cellSize)
    this.renderer.stepSim(s.view, window.devicePixelRatio, {
      dt,
      cellSize: g.cellSize,
      passes: count,
      particles,
      dye,
      release: s.release,
      line: s.releaseLine,
      stamps: this.sim.stamps.splice(0, 8),
      releaseRate: 0.03,
    })
  }

  /** Uploads the shown frame's velocity for the arrows, particles and dye. */
  private ensureVelocity(): void {
    const frame = this.current
    const velocity = frame?.fields.find((f) => f.name === 'velocity' && f.components === 2)
    if (frame && velocity && this.uploaded.velocity !== frame) {
      this.renderer.setVelocity(velocity, frame.header.width, frame.header.height)
      this.uploaded.velocity = frame
    }
  }

  /** Paints dye with the brush at a cell (the next simulation step adds it). */
  paint(x: number, y: number): void {
    const { grid, paintDye } = useViewer.getState()
    if (!grid) return
    const radius = Math.max(2, Math.min(grid.width, grid.height) / 40)
    this.sim.stamps.push([x, y, radius, paintDye])
    if (this.sim.stamps.length > 64) this.sim.stamps.shift()
  }

  /** Empties the dye and releases the particles afresh. */
  clearSim(): void {
    this.renderer.clearSim()
    this.schedule()
  }

  /** Released particles and the dye's total (tests). */
  measureSim(): { particles: number; dye: number } {
    return this.renderer.measureSim()
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
    const generation = this.generation
    const done = () => {
      if (this.inFlight.get(index) === p) this.inFlight.delete(index)
      return !this.disposed && generation === this.generation
    }
    const p: Promise<Decoded | null> = this.decoder.fetch(url).then(
      (d) => {
        if (!done()) return null
        this.cache.put(index, d, useViewer.getState().timeline.index ?? index)
        return d
      },
      (e: DecodeError) => {
        if (done()) {
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
      // Enough frames ahead to keep every frame worker busy.
      const ahead = Math.max(3, this.decoder.parallel + 1)
      for (const i of prefetch(index, t.last, ahead)) void this.load(i)
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
    const sharp = this.sharp?.of === this.current ? this.sharp.frame : null
    const full = sharp?.fields.find((g) => g.name === q.field)
    if (full) {
      // One value per cell: the cell's own (it is fluid).
      const i = y * c.width + x
      const at = (j: number) => (full.dtype === 'f16' ? halfToFloat(full.data[j]) : full.data[j])
      const m = full.components
      return componentValue(q.component, at(i * m), m > 1 ? at(i * m + 1) : 0)
    }
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

  private draw(override?: { view: View; dpr: number }): void {
    const start = performance.now()
    const s: ViewerState = useViewer.getState()
    const q = currentQuantity(s)
    const frame = this.current
    const o = s.overlays
    if (o.vectors || o.particles || o.dye) this.ensureVelocity()
    if (s.releaseLine !== this.marked) {
      const l = s.releaseLine
      this.renderer.setMark(l ? Float32Array.of(...l) : null)
      this.marked = l
    }
    if (q && frame) {
      // The sharpened frame when there is one for the frame shown (overlays keep the blocks).
      const sharp = this.sharp?.of === frame ? this.sharp.frame : null
      const source = sharp ?? frame
      const field = source.fields.find((f) => f.name === q.field)
      if (field && (this.uploaded.frame !== source || this.uploaded.field !== q.field)) {
        this.renderer.setField(field, source.header.width, source.header.height, sharp !== null)
        this.uploaded.frame = source
        this.uploaded.field = q.field
      }
      const map = colormapOf(s, q)
      if (this.uploaded.lut !== map) {
        this.renderer.setLut(lut(colormap(map)))
        this.uploaded.lut = map
      }
    }
    this.renderer.draw(
      override?.view ?? s.view,
      override?.dpr ?? window.devicePixelRatio,
      {
        component: q?.component ?? 'scalar',
        range: currentRange(s) ?? [0, 1],
        nearest: s.nearest,
      },
      COLOURS,
      this.overlayStyle(s, override?.dpr),
    )
    measureSince('viewer:draw', start)
  }

  /** The overlays to draw: arrows scaled to the fastest speed seen so far, translucent lines. */
  private overlayStyle(s: ViewerState, ratio?: number): Overlays {
    const dpr = ratio ?? window.devicePixelRatio
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
      dye: s.overlays.dye,
      particles: s.overlays.particles
        ? { colour: [0.9, 0.97, 1, 0.95], size: Math.max(2, 2 * dpr) }
        : null,
      mark: s.releaseLine && (s.tool === 'line' || s.overlays.particles) ? [1, 0.85, 0.2, 1] : null,
    }
  }

  /** What the footer of an exported image says, or null before the first frame. */
  private annotation(): Annotation | null {
    const s = useViewer.getState()
    const q = currentQuantity(s)
    const range = currentRange(s)
    const header = s.frame?.header
    if (!q || !range || !header) return null
    return {
      lut: lut(colormap(colormapOf(s, q))),
      range,
      unit: q.unit,
      label: q.label,
      time: header.simTime,
      frame: header.frameIndex,
      step: header.step,
    }
  }

  /**
   * The picture as a canvas: the view as it is on screen (`scale` null), or the whole domain at
   * `scale` pixels per cell, overlays included, with the legend and time in a footer if asked.
   * The domain is drawn by resizing the canvas for a moment: the browser shows nothing in between.
   */
  renderImage(scale: number | null, withFooter = true): HTMLCanvasElement {
    const s = useViewer.getState()
    const canvas = this.renderer.canvas
    const copy = () => {
      const c = document.createElement('canvas')
      c.width = canvas.width
      c.height = canvas.height
      c.getContext('2d')!.drawImage(canvas, 0, 0)
      return c
    }
    let image: HTMLCanvasElement
    if (scale === null) {
      this.draw()
      image = copy()
    } else {
      const g = s.grid
      if (!g) throw new Error('Nothing to export yet')
      const w = g.width * scale
      const h = g.height * scale
      if (Math.max(w, h) > this.renderer.maxImage) {
        throw new Error(
          `${w} × ${h} pixels is more than this GPU can draw (${this.renderer.maxImage}); use a smaller size`,
        )
      }
      this.renderer.resize(w, h, 1)
      // View zoom 1 at a device-pixel ratio of `scale`: `scale` pixels per cell, overlays scaled too.
      this.draw({ view: { zoom: 1, offsetX: 0, offsetY: 0 }, dpr: scale })
      image = copy()
      this.renderer.resize(s.viewport.width, s.viewport.height, window.devicePixelRatio)
      this.schedule()
    }
    const info = withFooter ? this.annotation() : null
    return info ? annotate(image, info) : image
  }

  /** The fields of the frame shown as CSV, or null before the first frame. */
  frameCsv(): string | null {
    const s = useViewer.getState()
    if (!this.current || !this.cells || !s.grid) return null
    return frameCsv(this.current, this.cells.fraction, s.grid, this.cells.blocks.k)
  }

  /** Shows frame `index` and waits until everything drawn over it (streamlines) is ready. */
  private async showFrame(index: number): Promise<boolean> {
    const d = await this.load(index)
    if (!d || this.disposed) return false
    this.display(index, d)
    this.ensureVelocity()
    if (useViewer.getState().overlays.streamlines) {
      for (let n = 0; n < 250; n++) {
        if (!this.streamlines.busy && this.streamlines.frame === this.current) break
        await new Promise((r) => setTimeout(r, 20))
      }
    }
    return true
  }

  /** Simulated seconds between output frames, once a frame after the first was shown (else null). */
  get frameInterval(): number | null {
    return this.sim.frameInterval > 0 ? this.sim.frameInterval : null
  }

  /**
   * Records every `step`-th frame of `from`..`to` as a video, one video frame per frame used
   * (frame-exact with WebCodecs), with the overlays and footer. Particles and dye advance by the
   * simulated time between the frames used, after a short warm-up so the first frame is not
   * empty. The timeline goes back to the frame it was on.
   */
  async exportVideo(
    range: { from: number; to: number; step: number; fps: number; scale: number; quality: Quality },
    onProgress: (done: number, total: number) => void,
    signal: AbortSignal,
  ): Promise<VideoResult> {
    const before = useViewer.getState().timeline.index
    // Before pausing: a paused, zoomed-in view would otherwise fetch full-resolution frames.
    this.exporting = true
    this.pause()
    try {
      const { from, to, step, fps, scale, quality } = range
      if (!(await this.showFrame(from))) throw new Error(`Frame ${from} is not available`)
      const size = this.renderImage(scale)
      const width = size.width - (size.width % 2)
      const height = size.height - (size.height % 2)
      const surface = document.createElement('canvas')
      surface.width = width
      surface.height = height
      const ctx = surface.getContext('2d')!
      const { particles, dye } = useViewer.getState().overlays
      if (particles || dye) {
        for (let i = 0; i < 40; i++) this.stepSimBy(this.sim.frameInterval / 4)
      }
      const total = Math.floor((to - from) / step) + 1
      return await encodeVideo({
        width,
        height,
        fps,
        quality,
        count: total,
        signal,
        onProgress: (done) => onProgress(done, total),
        draw: async (i) => {
          const index = from + i * step
          if (!(await this.showFrame(index))) throw new Error(`Frame ${index} is not available`)
          if (i > 0 && (particles || dye)) {
            for (let k = 0; k < step; k++) this.stepSimBy(this.sim.frameInterval)
          }
          ctx.drawImage(this.renderImage(scale), 0, 0)
          return surface
        },
      })
    } finally {
      this.exporting = false
      if (before !== null) this.request(before)
      this.syncAnimation()
    }
  }

  dispose(): void {
    this.disposed = true
    cancelAnimationFrame(this.frameRequest)
    cancelAnimationFrame(this.sim.request)
    this.stopLoop()
    this.cache.clear()
    this.unsubscribe()
    this.decoder.dispose()
    this.renderer.dispose()
  }
}
