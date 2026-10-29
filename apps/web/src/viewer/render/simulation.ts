// Tracer particles and dye on the GPU (design proposal §5). Both are *ping-pong* textures: a
// fragment shader reads the previous state from one texture and writes the next into another
// (a framebuffer object, "FBO", is a render target backed by a texture); then the two swap roles.
// Rendering to float textures needs `EXT_color_buffer_float`, present wherever WebGL2 runs
// (also in SwiftShader); without it the layer reports `supported = false`.
import { VERTEX } from '../../editor/render/shaders.ts'
import { bind, link } from '../../gl/gl.ts'
import { DYE_COLOURS, MAX_PARTICLES, type ReleaseMode } from '../simulation.ts'
import type { Colour4, LayerContext } from './overlays.ts'
import {
  DYE_ADVECT,
  DYE_DISPLAY,
  PARTICLE_FRAGMENT,
  PARTICLE_UPDATE,
  PARTICLE_VERTEX,
} from './simShaders.ts'

/** Particles: a 256 × 128 state texture. */
const STATE_W = 256
const STATE_H = MAX_PARTICLES / STATE_W
const SPAWN_ROW = 1024
const MODES: Record<ReleaseMode, number> = { inlets: 0, line: 1, domain: 2 }

interface Target {
  texture: WebGLTexture
  fbo: WebGLFramebuffer
}

/** Two targets that swap: `read` is the current state, `write` receives the next one. */
class PingPong {
  private targets: [Target, Target]
  private flip = 0
  readonly width: number
  readonly height: number

  constructor(targets: [Target, Target], width: number, height: number) {
    this.targets = targets
    this.width = width
    this.height = height
  }

  get read(): Target {
    return this.targets[this.flip]
  }

  get write(): Target {
    return this.targets[1 - this.flip]
  }

  swap(): void {
    this.flip = 1 - this.flip
  }

  targetsList(): Target[] {
    return this.targets
  }
}

const uniforms = (gl: WebGL2RenderingContext, p: WebGLProgram, names: string[]) =>
  Object.fromEntries(names.map((n) => [n, gl.getUniformLocation(p, n)]))

export interface StepParams {
  /** Simulated seconds of this animation frame, and the metres per cell. */
  dt: number
  cellSize: number
  /** Passes the step is split into (nothing moves more than a cell per pass). */
  passes: number
  particles: boolean
  /** Particles in use (the first texels of the state; the others stay unreleased). */
  count: number
  dye: boolean
  release: ReleaseMode
  line: [number, number, number, number] | null
  /** Brush stamps for this step: x, y (cells), radius (cells), dye 0–2. */
  stamps: readonly [number, number, number, number][]
  /** Probability per pass that a dead particle is released. */
  releaseRate: number
}

export class SimLayer {
  /** Whether float textures can be rendered to (particles and dye need it). */
  readonly supported: boolean
  private readonly gl: WebGL2RenderingContext
  private readonly vao: WebGLVertexArrayObject
  private readonly update: WebGLProgram
  private readonly drawProgram: WebGLProgram
  private readonly advect: WebGLProgram
  private readonly display: WebGLProgram
  private readonly uUpdate: Record<string, WebGLUniformLocation | null>
  private readonly uDraw: Record<string, WebGLUniformLocation | null>
  private readonly uAdvect: Record<string, WebGLUniformLocation | null>
  private readonly uDisplay: Record<string, WebGLUniformLocation | null>
  private particles: PingPong | null = null
  private dye: PingPong | null = null
  private spawnTex: WebGLTexture | null = null
  private spawnCount = 0
  /** Cells per dye texel, and the grid, once the cells are known. */
  private cells: { grid: [number, number]; k: number; blocks: [number, number] } | null = null
  private counter = 1

  constructor(gl: WebGL2RenderingContext) {
    this.gl = gl
    this.supported = gl.getExtension('EXT_color_buffer_float') !== null
    this.vao = gl.createVertexArray()
    this.update = link(gl, VERTEX, PARTICLE_UPDATE)
    this.drawProgram = link(gl, PARTICLE_VERTEX, PARTICLE_FRAGMENT)
    this.advect = link(gl, VERTEX, DYE_ADVECT)
    this.display = link(gl, VERTEX, DYE_DISPLAY)
    this.uUpdate = uniforms(gl, this.update, [
      'u_state',
      'u_velocity',
      'u_fraction',
      'u_cellType',
      'u_spawns',
      'u_mode',
      'u_spawnCount',
      'u_line',
      'u_grid',
      'u_fieldCells',
      'u_dt',
      'u_cellSize',
      'u_release',
      'u_count',
      'u_seed',
    ])
    this.uDraw = uniforms(gl, this.drawProgram, [
      'u_state',
      'u_origin',
      'u_zoom',
      'u_canvas',
      'u_size',
      'u_colour',
    ])
    this.uAdvect = uniforms(gl, this.advect, [
      'u_dye',
      'u_velocity',
      'u_fraction',
      'u_cellType',
      'u_grid',
      'u_dyeCells',
      'u_k',
      'u_dt',
      'u_cellSize',
      'u_decay',
      'u_stripe',
      'u_stamps',
      'u_stamp',
    ])
    this.uDisplay = uniforms(gl, this.display, [
      'u_dye',
      'u_cellType',
      'u_grid',
      'u_dyeCells',
      'u_origin',
      'u_zoom',
      'u_colours',
    ])
  }

  private target(
    width: number,
    height: number,
    internal: number,
    format: number,
    type: number,
    filter: number,
    data: ArrayBufferView | null,
  ): Target {
    const { gl } = this
    const texture = gl.createTexture()
    gl.bindTexture(gl.TEXTURE_2D, texture)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, filter)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, filter)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE)
    gl.texImage2D(gl.TEXTURE_2D, 0, internal, width, height, 0, format, type, data)
    const fbo = gl.createFramebuffer()
    gl.bindFramebuffer(gl.FRAMEBUFFER, fbo)
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0)
    const status = gl.checkFramebufferStatus(gl.FRAMEBUFFER)
    gl.bindFramebuffer(gl.FRAMEBUFFER, null)
    if (status !== gl.FRAMEBUFFER_COMPLETE) throw new Error('float render target unavailable')
    return { texture, fbo }
  }

  private free(p: PingPong | null): void {
    for (const t of p?.targetsList() ?? []) {
      this.gl.deleteTexture(t.texture)
      this.gl.deleteFramebuffer(t.fbo)
    }
  }

  /** (Re)creates the state for a run: dye texels are the frames' blocks; all particles unreleased. */
  setCells(
    grid: [number, number],
    blocks: { width: number; height: number; k: number },
    spawns: Float32Array,
  ): void {
    if (!this.supported) return
    const { gl } = this
    this.free(this.dye)
    this.free(this.particles)
    this.cells = { grid, k: blocks.k, blocks: [blocks.width, blocks.height] }
    const none = new Float32Array(STATE_W * STATE_H * 2).fill(-1)
    const state = () => this.target(STATE_W, STATE_H, gl.RG32F, gl.RG, gl.FLOAT, gl.NEAREST, none)
    this.particles = new PingPong([state(), state()], STATE_W, STATE_H)
    const dye = () =>
      this.target(blocks.width, blocks.height, gl.RGBA16F, gl.RGBA, gl.HALF_FLOAT, gl.LINEAR, null)
    this.dye = new PingPong([dye(), dye()], blocks.width, blocks.height)
    this.setSpawns(spawns)
  }

  private setSpawns(points: Float32Array): void {
    const { gl } = this
    if (this.spawnTex) gl.deleteTexture(this.spawnTex)
    const n = points.length / 2
    const rows = Math.max(1, Math.ceil(n / SPAWN_ROW))
    const data = new Float32Array(SPAWN_ROW * rows * 2)
    data.set(points)
    this.spawnTex = gl.createTexture()
    gl.bindTexture(gl.TEXTURE_2D, this.spawnTex)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST)
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RG32F, SPAWN_ROW, rows, 0, gl.RG, gl.FLOAT, data)
    this.spawnCount = n
  }

  /** Empties the dye and releases no particle until the next step. */
  clear(): void {
    const { gl } = this
    for (const p of [this.dye, this.particles]) {
      for (const t of p?.targetsList() ?? []) {
        gl.bindFramebuffer(gl.FRAMEBUFFER, t.fbo)
        gl.clearColor(p === this.particles ? -1 : 0, p === this.particles ? -1 : 0, 0, 1)
        gl.clear(gl.COLOR_BUFFER_BIT)
      }
    }
    gl.bindFramebuffer(gl.FRAMEBUFFER, null)
  }

  /** Advances the particles and/or the dye by `passes` passes. Needs the velocity of a frame. */
  step(ctx: LayerContext, p: StepParams): void {
    const { gl } = this
    const cells = this.cells
    if (!this.supported || !cells || !this.particles || !this.dye) return
    gl.disable(gl.BLEND)
    gl.bindVertexArray(this.vao)
    const dt = p.dt / p.passes
    if (p.particles) {
      const u = this.uUpdate
      gl.useProgram(this.update)
      gl.viewport(0, 0, STATE_W, STATE_H)
      bind(gl, 1, ctx.fieldTex, u.u_velocity)
      bind(gl, 2, ctx.fractionTex, u.u_fraction)
      bind(gl, 3, ctx.cellTex, u.u_cellType)
      bind(gl, 5, this.spawnTex!, u.u_spawns)
      gl.uniform1i(u.u_mode, MODES[p.release])
      gl.uniform1i(u.u_spawnCount, this.spawnCount)
      const line = p.line ?? [-1, -1, -1, -1]
      gl.uniform4f(u.u_line, ...line)
      gl.uniform2i(u.u_grid, ctx.grid[0], ctx.grid[1])
      gl.uniform2f(u.u_fieldCells, ctx.fieldCells[0], ctx.fieldCells[1])
      gl.uniform1f(u.u_dt, dt)
      gl.uniform1f(u.u_cellSize, p.cellSize)
      gl.uniform1f(u.u_release, p.releaseRate)
      gl.uniform1i(u.u_count, p.count)
      for (let i = 0; i < p.passes; i++) {
        bind(gl, 0, this.particles.read.texture, u.u_state)
        gl.uniform1ui(u.u_seed, this.counter++)
        gl.bindFramebuffer(gl.FRAMEBUFFER, this.particles.write.fbo)
        gl.drawArrays(gl.TRIANGLES, 0, 3)
        this.particles.swap()
      }
    }
    if (p.dye) {
      const u = this.uAdvect
      gl.useProgram(this.advect)
      gl.viewport(0, 0, this.dye.width, this.dye.height)
      bind(gl, 1, ctx.fieldTex, u.u_velocity)
      bind(gl, 2, ctx.fractionTex, u.u_fraction)
      bind(gl, 3, ctx.cellTex, u.u_cellType)
      gl.uniform2i(u.u_grid, ctx.grid[0], ctx.grid[1])
      gl.uniform2f(u.u_dyeCells, this.dye.width * cells.k, this.dye.height * cells.k)
      gl.uniform1i(u.u_k, cells.k)
      gl.uniform1f(u.u_dt, dt)
      gl.uniform1f(u.u_cellSize, p.cellSize)
      // About 0.4 /s: the dye fades over a few seconds of simulated time.
      gl.uniform1f(u.u_decay, Math.exp(-0.4 * dt))
      gl.uniform1f(u.u_stripe, Math.max(2, Math.round(ctx.grid[1] / 9)))
      const stamps = p.stamps.slice(0, 8)
      gl.uniform1i(u.u_stamps, 0)
      for (let i = 0; i < p.passes; i++) {
        bind(gl, 0, this.dye.read.texture, u.u_dye)
        // Stamps go in the first pass only.
        gl.uniform1i(u.u_stamps, i === 0 ? stamps.length : 0)
        if (i === 0 && stamps.length > 0) {
          gl.uniform4fv(u.u_stamp, stamps.flat())
        }
        gl.bindFramebuffer(gl.FRAMEBUFFER, this.dye.write.fbo)
        gl.drawArrays(gl.TRIANGLES, 0, 3)
        this.dye.swap()
      }
    }
    gl.bindFramebuffer(gl.FRAMEBUFFER, null)
  }

  /** The dye over the field. */
  drawDye(ctx: LayerContext): void {
    const { gl } = this
    const cells = this.cells
    if (!this.supported || !cells || !this.dye) return
    const u = this.uDisplay
    gl.useProgram(this.display)
    gl.bindVertexArray(this.vao)
    bind(gl, 0, this.dye.read.texture, u.u_dye)
    bind(gl, 1, ctx.cellTex, u.u_cellType)
    gl.uniform2i(u.u_grid, ctx.grid[0], ctx.grid[1])
    gl.uniform2f(u.u_dyeCells, this.dye.width * cells.k, this.dye.height * cells.k)
    gl.uniform2f(u.u_origin, ctx.origin[0], ctx.origin[1])
    gl.uniform1f(u.u_zoom, ctx.zoom)
    gl.uniform3fv(u.u_colours, DYE_COLOURS.flat())
    gl.enable(gl.BLEND)
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA)
    gl.drawArrays(gl.TRIANGLES, 0, 3)
    gl.disable(gl.BLEND)
  }

  /** The released particles among the first `count` as points. */
  drawParticles(ctx: LayerContext, colour: Colour4, size: number, count: number): void {
    const { gl } = this
    if (!this.supported || !this.particles) return
    const u = this.uDraw
    gl.useProgram(this.drawProgram)
    gl.bindVertexArray(this.vao)
    bind(gl, 0, this.particles.read.texture, u.u_state)
    gl.uniform2f(u.u_origin, ctx.origin[0], ctx.origin[1])
    gl.uniform1f(u.u_zoom, ctx.zoom)
    gl.uniform2f(u.u_canvas, ctx.canvas[0], ctx.canvas[1])
    gl.uniform1f(u.u_size, size)
    gl.uniform4f(u.u_colour, ...colour)
    gl.enable(gl.BLEND)
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA)
    gl.drawArrays(gl.POINTS, 0, Math.min(count, STATE_W * STATE_H))
    gl.disable(gl.BLEND)
  }

  /** Released particles and the dye's total (tests; reads the textures back). */
  measure(): { particles: number; dye: number } {
    const { gl } = this
    if (!this.supported || !this.particles || !this.dye) return { particles: 0, dye: 0 }
    const read = (t: Target, w: number, h: number) => {
      const out = new Float32Array(w * h * 4)
      gl.bindFramebuffer(gl.FRAMEBUFFER, t.fbo)
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.FLOAT, out)
      gl.bindFramebuffer(gl.FRAMEBUFFER, null)
      return out
    }
    const s = read(this.particles.read, STATE_W, STATE_H)
    let particles = 0
    for (let i = 0; i < s.length; i += 4) if (s[i] >= 0) particles++
    const d = read(this.dye.read, this.dye.width, this.dye.height)
    let dye = 0
    for (let i = 0; i < d.length; i += 4) dye += d[i] + d[i + 1] + d[i + 2]
    return { particles, dye }
  }

  dispose(): void {
    const { gl } = this
    this.free(this.dye)
    this.free(this.particles)
    if (this.spawnTex) gl.deleteTexture(this.spawnTex)
    for (const p of [this.update, this.drawProgram, this.advect, this.display]) gl.deleteProgram(p)
    gl.deleteVertexArray(this.vao)
  }
}
