// WebGL2 renderer of result fields (design proposal §4.1): the editor's structure (one
// full-screen triangle, a fragment shader per screen pixel, drawing on demand) with half-float
// field textures, the solid mask at full resolution and a colour-map lookup texture.
import type { Rgba } from '../../editor/colours.ts'
import { VERTEX } from '../../editor/render/shaders.ts'
import type { View } from '../../editor/view.ts'
import type { DecodedField } from '../../frames/lffr.ts'
import { bind, integerTexture, link } from '../../gl/gl.ts'
import type { Component } from '../quantities.ts'
import {
  LineLayer,
  VectorLayer,
  type Colour4,
  type LayerContext,
  type VectorStyle,
} from './overlays.ts'
import { FIELD_FRAGMENT } from './shaders.ts'
import { SimLayer, type StepParams } from './simulation.ts'

const COMPONENT: Record<Component, number> = { scalar: 0, magnitude: 1, x: 2, y: 3 }

export interface FieldStyle {
  component: Component
  range: [number, number]
  /** Show the field's blocks as they are (no bilinear interpolation). */
  nearest: boolean
}

/** The overlays to draw over the field (null: not drawn). */
export interface Overlays {
  vectors: VectorStyle | null
  streamlines: Colour4 | null
  /** The dye texture over the field. */
  dye: boolean
  particles: { colour: Colour4; size: number } | null
  /** A marker line (the release line being drawn). */
  mark: Colour4 | null
}

export interface ViewerColours {
  /** Per cell type (the fluid entry is used before any frame arrives). */
  cellTypes: readonly Rgba[]
  background: Rgba
  border: Rgba
  /** Infinite or NaN values. */
  noData: Rgba
}

const norm = (c: Rgba) => c.map((v) => v / 255)

/** A texture that WebGL may filter linearly (float or normalised formats). */
function filteredTexture(gl: WebGL2RenderingContext): WebGLTexture {
  const t = gl.createTexture()
  gl.bindTexture(gl.TEXTURE_2D, t)
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR)
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR)
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE)
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE)
  return t
}

export class FieldRenderer {
  readonly canvas: HTMLCanvasElement
  readonly maxTextureSize: number
  private readonly gl: WebGL2RenderingContext
  private readonly program: WebGLProgram
  private readonly vao: WebGLVertexArrayObject
  private readonly u: Record<string, WebGLUniformLocation | null>
  private readonly cellTex: WebGLTexture
  private readonly fieldTex: WebGLTexture
  /** The velocity, for the arrows (the shown field may be another one). */
  private readonly velocityTex: WebGLTexture
  private hasVelocity = false
  private readonly fractionTex: WebGLTexture
  private readonly lutTex: WebGLTexture
  private readonly vectorLayer: VectorLayer
  private readonly lineLayer: LineLayer
  private readonly markLayer: LineLayer
  private readonly sim: SimLayer
  private grid: [number, number] | null = null
  /** The field texture's size in blocks and the box factor k. */
  private field: { width: number; height: number; k: number } | null = null
  private nearest = false
  /** Whether a field has been uploaded since the cells were set. */
  private hasField = false

  constructor(canvas: HTMLCanvasElement) {
    this.canvas = canvas
    // preserveDrawingBuffer: the image stays readable after drawing (tests, PNG export).
    const gl = canvas.getContext('webgl2', {
      antialias: false,
      alpha: false,
      preserveDrawingBuffer: true,
    })
    if (!gl) throw new Error('WebGL2 is not available in this browser')
    this.gl = gl
    this.maxTextureSize = gl.getParameter(gl.MAX_TEXTURE_SIZE) as number
    this.program = link(gl, VERTEX, FIELD_FRAGMENT)
    this.vao = gl.createVertexArray()
    this.u = Object.fromEntries(
      [
        'u_cellType',
        'u_field',
        'u_fraction',
        'u_lut',
        'u_hasField',
        'u_grid',
        'u_fieldCells',
        'u_component',
        'u_range',
        'u_origin',
        'u_zoom',
        'u_cellColours',
        'u_background',
        'u_borderColour',
        'u_noData',
      ].map((n) => [n, gl.getUniformLocation(this.program, n)]),
    )
    this.cellTex = integerTexture(gl)
    this.fieldTex = filteredTexture(gl)
    this.velocityTex = filteredTexture(gl)
    this.fractionTex = filteredTexture(gl)
    this.lutTex = filteredTexture(gl)
    this.vectorLayer = new VectorLayer(gl)
    this.lineLayer = new LineLayer(gl)
    this.markLayer = new LineLayer(gl)
    this.sim = new SimLayer(gl)
  }

  /** The largest image side this GPU can draw (viewport and render buffer limits). */
  get maxImage(): number {
    const { gl } = this
    const dims = gl.getParameter(gl.MAX_VIEWPORT_DIMS) as Int32Array
    return Math.min(dims[0], dims[1], gl.getParameter(gl.MAX_RENDERBUFFER_SIZE) as number)
  }

  /** Whether particles and dye can run (float textures can be rendered to). */
  get floatTargets(): boolean {
    return this.sim.supported
  }

  /** The release line to mark, as segments in cells, or null. */
  setMark(segments: Float32Array | null): void {
    this.markLayer.set(segments)
  }

  /** Empties the dye and the particles. */
  clearSim(): void {
    this.sim.clear()
  }

  /** Released particles and the dye's total (tests). */
  measureSim(): { particles: number; dye: number } {
    return this.sim.measure()
  }

  /** Advances the particles and the dye (needs the velocity of a frame to have been uploaded). */
  stepSim(view: View, dpr: number, params: StepParams): void {
    const ctx = this.context(view, dpr)
    if (ctx && this.hasVelocity) this.sim.step(ctx, params)
  }

  private context(view: View, dpr: number): LayerContext | null {
    const { grid, field } = this
    if (!grid || !field) return null
    const zoom = view.zoom * dpr
    return {
      canvas: [this.canvas.width, this.canvas.height],
      origin: [view.offsetX * dpr, this.canvas.height - (view.offsetY * dpr + grid[1] * zoom)],
      zoom,
      grid,
      fieldCells: [field.width * field.k, field.height * field.k],
      cellTex: this.cellTex,
      fieldTex: this.velocityTex,
      fractionTex: this.fractionTex,
    }
  }

  /** Streamline segments held (tests). */
  get streamlineCount(): number {
    return this.lineLayer.count
  }

  /** Replaces the streamline segments (cells) or clears them. */
  setStreamlines(segments: Float32Array | null): void {
    this.lineLayer.set(segments)
  }

  /** The run's cells (full resolution; 0 = fluid) and the fluid fraction of each block (size w × h). */
  setCells(
    cellType: Uint8Array,
    width: number,
    height: number,
    fraction: Float32Array,
    blocks: { width: number; height: number; k: number },
    spawns: Float32Array = new Float32Array(0),
  ): void {
    const { gl } = this
    if (width > this.maxTextureSize || height > this.maxTextureSize) {
      throw new Error(
        `this GPU's textures are limited to ${this.maxTextureSize} cells per side (grid: ${width} × ${height})`,
      )
    }
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1)
    gl.bindTexture(gl.TEXTURE_2D, this.cellTex)
    gl.texImage2D(
      gl.TEXTURE_2D,
      0,
      gl.R8UI,
      width,
      height,
      0,
      gl.RED_INTEGER,
      gl.UNSIGNED_BYTE,
      cellType,
    )
    // The fraction as a normalised byte (R8 filters linearly everywhere; 1/255 is plenty).
    const bytes = Uint8Array.from(fraction, (f) => Math.round(f * 255))
    gl.bindTexture(gl.TEXTURE_2D, this.fractionTex)
    gl.texImage2D(
      gl.TEXTURE_2D,
      0,
      gl.R8,
      blocks.width,
      blocks.height,
      0,
      gl.RED,
      gl.UNSIGNED_BYTE,
      bytes,
    )
    this.grid = [width, height]
    this.field = { ...blocks }
    this.hasField = false
    this.hasVelocity = false
    this.sim.setCells([width, height], blocks, spawns)
  }

  /**
   * Uploads a field of the current frame: f16 bits straight into an R16F/RG16F texture (WebGL2
   * filters half floats linearly without extensions), or f32 values into R32F/RG32F.
   */
  setField(field: DecodedField, width: number, height: number): void {
    this.upload(this.fieldTex, field, width, height)
    this.filter(field.dtype === 'f32' || this.nearest)
    this.hasField = true
  }

  /** Uploads the frame's velocity for the arrows. */
  setVelocity(field: DecodedField, width: number, height: number): void {
    this.upload(this.velocityTex, field, width, height)
    const { gl } = this
    const f = field.dtype === 'f32' ? gl.NEAREST : gl.LINEAR
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, f)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, f)
    this.hasVelocity = true
  }

  private upload(tex: WebGLTexture, field: DecodedField, width: number, height: number): void {
    const { gl } = this
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1)
    gl.bindTexture(gl.TEXTURE_2D, tex)
    const two = field.components === 2
    if (field.dtype === 'f16') {
      gl.texImage2D(
        gl.TEXTURE_2D,
        0,
        two ? gl.RG16F : gl.R16F,
        width,
        height,
        0,
        two ? gl.RG : gl.RED,
        gl.HALF_FLOAT,
        field.data,
      )
    } else {
      // 32-bit floats are not filterable without OES_texture_float_linear: show blocks as they are.
      gl.texImage2D(
        gl.TEXTURE_2D,
        0,
        two ? gl.RG32F : gl.R32F,
        width,
        height,
        0,
        two ? gl.RG : gl.RED,
        gl.FLOAT,
        field.data,
      )
    }
  }

  /** Uploads a colour map's 256-entry lookup table. */
  setLut(lut: Uint8Array): void {
    const { gl } = this
    gl.bindTexture(gl.TEXTURE_2D, this.lutTex)
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, 256, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, lut)
  }

  private filter(nearest: boolean): void {
    const { gl } = this
    const f = nearest ? gl.NEAREST : gl.LINEAR
    gl.bindTexture(gl.TEXTURE_2D, this.fieldTex)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, f)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, f)
    gl.bindTexture(gl.TEXTURE_2D, this.fractionTex)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, f)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, f)
  }

  resize(cssWidth: number, cssHeight: number, dpr: number): void {
    const w = Math.max(1, Math.round(cssWidth * dpr))
    const h = Math.max(1, Math.round(cssHeight * dpr))
    if (this.canvas.width !== w || this.canvas.height !== h) {
      this.canvas.width = w
      this.canvas.height = h
    }
  }

  draw(
    view: View,
    dpr: number,
    style: FieldStyle,
    colours: ViewerColours,
    overlays: Overlays = {
      vectors: null,
      streamlines: null,
      dye: false,
      particles: null,
      mark: null,
    },
  ): void {
    const { gl, u, grid, field } = this
    gl.viewport(0, 0, this.canvas.width, this.canvas.height)
    const bg = colours.background
    gl.clearColor(bg[0] / 255, bg[1] / 255, bg[2] / 255, 1)
    gl.clear(gl.COLOR_BUFFER_BIT)
    if (!grid || !field) return
    if (style.nearest !== this.nearest) {
      this.nearest = style.nearest
      this.filter(style.nearest)
    }
    gl.useProgram(this.program)
    gl.bindVertexArray(this.vao)
    bind(gl, 0, this.cellTex, u.u_cellType)
    bind(gl, 1, this.fieldTex, u.u_field)
    bind(gl, 2, this.fractionTex, u.u_fraction)
    bind(gl, 3, this.lutTex, u.u_lut)
    gl.uniform1i(u.u_hasField, this.hasField ? 1 : 0)
    gl.uniform2i(u.u_grid, grid[0], grid[1])
    gl.uniform2f(u.u_fieldCells, field.width * field.k, field.height * field.k)
    gl.uniform1i(u.u_component, COMPONENT[style.component])
    gl.uniform2f(u.u_range, style.range[0], style.range[1])
    const zoom = view.zoom * dpr
    gl.uniform2f(
      u.u_origin,
      view.offsetX * dpr,
      this.canvas.height - (view.offsetY * dpr + grid[1] * zoom),
    )
    gl.uniform1f(u.u_zoom, zoom)
    gl.uniform4fv(u.u_cellColours, colours.cellTypes.flatMap(norm))
    gl.uniform4fv(u.u_background, norm(colours.background))
    gl.uniform4fv(u.u_borderColour, norm(colours.border))
    gl.uniform4fv(u.u_noData, norm(colours.noData))
    gl.drawArrays(gl.TRIANGLES, 0, 3)
    if (!this.hasField) return
    const ctx = this.context(view, dpr)!
    if (overlays.dye && this.hasVelocity) this.sim.drawDye(ctx)
    if (overlays.streamlines) this.lineLayer.draw(ctx, overlays.streamlines)
    if (overlays.mark) this.markLayer.draw(ctx, overlays.mark)
    if (overlays.particles && this.hasVelocity) {
      this.sim.drawParticles(ctx, overlays.particles.colour, overlays.particles.size)
    }
    if (overlays.vectors && this.hasVelocity) this.vectorLayer.draw(ctx, overlays.vectors)
  }

  /** The RGBA pixel at a device-pixel position from the top-left corner (tests, eyedropper). */
  pixel(x: number, y: number): [number, number, number, number] {
    const { gl } = this
    const out = new Uint8Array(4)
    gl.readPixels(x, this.canvas.height - 1 - y, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, out)
    return [out[0], out[1], out[2], out[3]]
  }

  dispose(): void {
    const { gl } = this
    for (const t of [this.cellTex, this.fieldTex, this.velocityTex, this.fractionTex, this.lutTex])
      gl.deleteTexture(t)
    gl.deleteProgram(this.program)
    gl.deleteVertexArray(this.vao)
    this.vectorLayer.dispose()
    this.lineLayer.dispose()
    this.markLayer.dispose()
    this.sim.dispose()
  }
}
