// Overlays drawn over the field: velocity arrows (instanced segments whose vertex shader reads the
// velocity texture) and streamlines (segments computed in the decode worker). Both share the
// field renderer's GL context and textures.
import { bind, link } from '../../gl/gl.ts'
import { LINE_FRAGMENT, LINE_VERTEX, VECTOR_VERTEX } from './overlayShaders.ts'

/** What an overlay needs of the field renderer for one draw. */
export interface LayerContext {
  /** Canvas size and the domain's bottom-left corner, in device pixels. */
  canvas: [number, number]
  origin: [number, number]
  /** Device pixels per cell. */
  zoom: number
  grid: [number, number]
  fieldCells: [number, number]
  cellTex: WebGLTexture
  fieldTex: WebGLTexture
  fractionTex: WebGLTexture
}

export type Colour4 = [number, number, number, number]

const uniforms = (gl: WebGL2RenderingContext, p: WebGLProgram, names: string[]) =>
  Object.fromEntries(names.map((n) => [n, gl.getUniformLocation(p, n)]))

export interface VectorStyle {
  /** Distance between arrows (device pixels). */
  spacing: number
  /** Device pixels per m/s. */
  scale: number
  colour: Colour4
}

export class VectorLayer {
  private readonly gl: WebGL2RenderingContext
  private readonly program: WebGLProgram
  private readonly vao: WebGLVertexArrayObject
  private readonly u: Record<string, WebGLUniformLocation | null>

  constructor(gl: WebGL2RenderingContext) {
    this.gl = gl
    this.program = link(gl, VECTOR_VERTEX, LINE_FRAGMENT)
    this.vao = gl.createVertexArray()
    this.u = uniforms(gl, this.program, [
      'u_cellType',
      'u_field',
      'u_fraction',
      'u_grid',
      'u_fieldCells',
      'u_origin',
      'u_zoom',
      'u_canvas',
      'u_spacing',
      'u_first',
      'u_columns',
      'u_scale',
      'u_maxLength',
      'u_colour',
    ])
  }

  draw(ctx: LayerContext, style: VectorStyle): void {
    const { gl, u } = this
    const { spacing } = style
    const first = [Math.floor(-ctx.origin[0] / spacing), Math.floor(-ctx.origin[1] / spacing)]
    const columns = Math.ceil(ctx.canvas[0] / spacing) + 2
    const rows = Math.ceil(ctx.canvas[1] / spacing) + 2
    gl.useProgram(this.program)
    gl.bindVertexArray(this.vao)
    bind(gl, 0, ctx.cellTex, u.u_cellType)
    bind(gl, 1, ctx.fieldTex, u.u_field)
    bind(gl, 2, ctx.fractionTex, u.u_fraction)
    gl.uniform2i(u.u_grid, ctx.grid[0], ctx.grid[1])
    gl.uniform2f(u.u_fieldCells, ctx.fieldCells[0], ctx.fieldCells[1])
    gl.uniform2f(u.u_origin, ctx.origin[0], ctx.origin[1])
    gl.uniform1f(u.u_zoom, ctx.zoom)
    gl.uniform2f(u.u_canvas, ctx.canvas[0], ctx.canvas[1])
    gl.uniform1f(u.u_spacing, spacing)
    gl.uniform2i(u.u_first, first[0], first[1])
    gl.uniform1i(u.u_columns, columns)
    gl.uniform1f(u.u_scale, style.scale)
    gl.uniform1f(u.u_maxLength, spacing * 1.1)
    gl.uniform4f(u.u_colour, ...style.colour)
    gl.enable(gl.BLEND)
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA)
    gl.drawArraysInstanced(gl.LINES, 0, 6, columns * rows)
    gl.disable(gl.BLEND)
  }

  dispose(): void {
    this.gl.deleteProgram(this.program)
    this.gl.deleteVertexArray(this.vao)
  }
}

/** Line segments in cell coordinates (x0, y0, x1, y1, …). */
export class LineLayer {
  private readonly gl: WebGL2RenderingContext
  private readonly program: WebGLProgram
  private readonly vao: WebGLVertexArrayObject
  private readonly buffer: WebGLBuffer
  private readonly u: Record<string, WebGLUniformLocation | null>
  private vertices = 0

  constructor(gl: WebGL2RenderingContext) {
    this.gl = gl
    this.program = link(gl, LINE_VERTEX, LINE_FRAGMENT)
    this.u = uniforms(gl, this.program, ['u_origin', 'u_zoom', 'u_canvas', 'u_colour'])
    this.vao = gl.createVertexArray()
    this.buffer = gl.createBuffer()
    gl.bindVertexArray(this.vao)
    gl.bindBuffer(gl.ARRAY_BUFFER, this.buffer)
    gl.enableVertexAttribArray(0)
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0)
    gl.bindVertexArray(null)
  }

  /** Replaces the segments (null or empty: nothing is drawn). */
  set(segments: Float32Array | null): void {
    const { gl } = this
    gl.bindBuffer(gl.ARRAY_BUFFER, this.buffer)
    gl.bufferData(gl.ARRAY_BUFFER, segments ?? new Float32Array(0), gl.DYNAMIC_DRAW)
    this.vertices = segments ? segments.length / 2 : 0
  }

  /** Segments held. */
  get count(): number {
    return this.vertices / 2
  }

  draw(ctx: LayerContext, colour: Colour4): void {
    const { gl, u } = this
    if (this.vertices === 0) return
    gl.useProgram(this.program)
    gl.bindVertexArray(this.vao)
    gl.uniform2f(u.u_origin, ctx.origin[0], ctx.origin[1])
    gl.uniform1f(u.u_zoom, ctx.zoom)
    gl.uniform2f(u.u_canvas, ctx.canvas[0], ctx.canvas[1])
    gl.uniform4f(u.u_colour, ...colour)
    gl.enable(gl.BLEND)
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA)
    gl.drawArrays(gl.LINES, 0, this.vertices)
    gl.disable(gl.BLEND)
  }

  dispose(): void {
    this.gl.deleteProgram(this.program)
    this.gl.deleteVertexArray(this.vao)
    this.gl.deleteBuffer(this.buffer)
  }
}
