// WebGL2 renderer of the editor's cells (design proposal §3). The layers are uploaded unchanged
// into integer textures; a fragment shader over the whole canvas turns each screen pixel into a
// cell, looks its colour up and draws grid lines. Rendering happens on demand, not in a loop.
import type { Colours } from '../colours.ts'
import type { Layers } from '../layers.ts'
import type { View } from '../view.ts'
import { FRAGMENT, VERTEX } from './shaders.ts'

/** Grid lines appear from this zoom (CSS pixels per cell). */
export const GRID_MIN_ZOOM = 8

/** Element palette texture: 256 × 256 texels covers every u16 element id. */
const PALETTE_SIDE = 256

export class Renderer {
  private readonly gl: WebGL2RenderingContext
  private readonly program: WebGLProgram
  private readonly vao: WebGLVertexArrayObject
  private readonly uniforms: Record<string, WebGLUniformLocation | null>
  private readonly cellTex: WebGLTexture
  private readonly elementTex: WebGLTexture
  private readonly paletteTex: WebGLTexture
  private layers: Layers | null = null
  private colours: Colours | null = null
  private paletteIds: number[] = []
  readonly maxTextureSize: number

  readonly canvas: HTMLCanvasElement

  constructor(canvas: HTMLCanvasElement) {
    this.canvas = canvas
    const gl = canvas.getContext('webgl2', { antialias: false, alpha: false })
    if (!gl) throw new Error('WebGL2 is not available in this browser')
    this.gl = gl
    this.maxTextureSize = gl.getParameter(gl.MAX_TEXTURE_SIZE) as number
    this.program = link(gl, VERTEX, FRAGMENT)
    this.vao = gl.createVertexArray()
    this.uniforms = Object.fromEntries(
      [
        'u_cellType',
        'u_elementId',
        'u_palette',
        'u_grid',
        'u_origin',
        'u_zoom',
        'u_cellColours',
        'u_background',
        'u_gridColour',
        'u_borderColour',
        'u_showGrid',
      ].map((n) => [n, gl.getUniformLocation(this.program, n)]),
    )
    this.cellTex = integerTexture(gl)
    this.elementTex = integerTexture(gl)
    this.paletteTex = integerTexture(gl)
    gl.bindTexture(gl.TEXTURE_2D, this.paletteTex)
    gl.texImage2D(
      gl.TEXTURE_2D,
      0,
      gl.RGBA8,
      PALETTE_SIDE,
      PALETTE_SIDE,
      0,
      gl.RGBA,
      gl.UNSIGNED_BYTE,
      null,
    )
  }

  /** Uses a new document's layers: allocates the textures and uploads everything. */
  setLayers(layers: Layers): void {
    const { gl } = this
    const { width, height } = layers
    if (width > this.maxTextureSize || height > this.maxTextureSize) {
      throw new Error(
        `this GPU's textures are limited to ${this.maxTextureSize} cells per side (grid: ${width} × ${height})`,
      )
    }
    this.layers = layers
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1)
    gl.pixelStorei(gl.UNPACK_ROW_LENGTH, 0)
    gl.pixelStorei(gl.UNPACK_SKIP_PIXELS, 0)
    gl.pixelStorei(gl.UNPACK_SKIP_ROWS, 0)
    // Texture row 0 is the scene's row y = 0, so `texelFetch(t, ivec2(x, y))` is cell (x, y).
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
      layers.cellType,
    )
    gl.bindTexture(gl.TEXTURE_2D, this.elementTex)
    gl.texImage2D(
      gl.TEXTURE_2D,
      0,
      gl.R16UI,
      width,
      height,
      0,
      gl.RED_INTEGER,
      gl.UNSIGNED_SHORT,
      layers.elementId,
    )
    layers.takeDirty()
  }

  /** Uploads the tiles changed since the last call (texSubImage2D straight from the layers). */
  private uploadDirty(): void {
    const { gl, layers } = this
    if (!layers) return
    const rects = layers.takeDirty()
    if (rects.length === 0) return
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1)
    gl.pixelStorei(gl.UNPACK_ROW_LENGTH, layers.width)
    for (const r of rects) {
      gl.pixelStorei(gl.UNPACK_SKIP_PIXELS, r.x0)
      gl.pixelStorei(gl.UNPACK_SKIP_ROWS, r.y0)
      const w = r.x1 - r.x0
      const h = r.y1 - r.y0
      gl.bindTexture(gl.TEXTURE_2D, this.cellTex)
      gl.texSubImage2D(
        gl.TEXTURE_2D,
        0,
        r.x0,
        r.y0,
        w,
        h,
        gl.RED_INTEGER,
        gl.UNSIGNED_BYTE,
        layers.cellType,
      )
      gl.bindTexture(gl.TEXTURE_2D, this.elementTex)
      gl.texSubImage2D(
        gl.TEXTURE_2D,
        0,
        r.x0,
        r.y0,
        w,
        h,
        gl.RED_INTEGER,
        gl.UNSIGNED_SHORT,
        layers.elementId,
      )
    }
    gl.pixelStorei(gl.UNPACK_ROW_LENGTH, 0)
    gl.pixelStorei(gl.UNPACK_SKIP_PIXELS, 0)
    gl.pixelStorei(gl.UNPACK_SKIP_ROWS, 0)
  }

  /** Sets the colours; only the element palette entries that changed are uploaded. */
  setColours(colours: Colours): void {
    const { gl } = this
    gl.bindTexture(gl.TEXTURE_2D, this.paletteTex)
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1)
    // Clear entries of elements that are gone (alpha 0 = "use the cell type's colour").
    const clear = new Uint8Array(4)
    for (const id of this.paletteIds) {
      if (!colours.elements.has(id)) {
        gl.texSubImage2D(
          gl.TEXTURE_2D,
          0,
          id % PALETTE_SIDE,
          Math.floor(id / PALETTE_SIDE),
          1,
          1,
          gl.RGBA,
          gl.UNSIGNED_BYTE,
          clear,
        )
      }
    }
    for (const [id, c] of colours.elements) {
      if (this.colours?.elements.get(id) === c) continue
      gl.texSubImage2D(
        gl.TEXTURE_2D,
        0,
        id % PALETTE_SIDE,
        Math.floor(id / PALETTE_SIDE),
        1,
        1,
        gl.RGBA,
        gl.UNSIGNED_BYTE,
        new Uint8Array(c),
      )
    }
    this.paletteIds = [...colours.elements.keys()]
    this.colours = colours
  }

  /** Resizes the drawing buffer to the canvas' CSS size × devicePixelRatio. */
  resize(cssWidth: number, cssHeight: number, dpr: number): void {
    const w = Math.max(1, Math.round(cssWidth * dpr))
    const h = Math.max(1, Math.round(cssHeight * dpr))
    if (this.canvas.width !== w || this.canvas.height !== h) {
      this.canvas.width = w
      this.canvas.height = h
    }
  }

  draw(view: View, dpr: number): void {
    const { gl, layers, colours, uniforms: u } = this
    this.uploadDirty()
    gl.viewport(0, 0, this.canvas.width, this.canvas.height)
    const bg = colours?.background ?? [0, 0, 0, 255]
    gl.clearColor(bg[0] / 255, bg[1] / 255, bg[2] / 255, 1)
    gl.clear(gl.COLOR_BUFFER_BIT)
    if (!layers || !colours) return
    gl.useProgram(this.program)
    gl.bindVertexArray(this.vao)
    bind(gl, 0, this.cellTex, u.u_cellType)
    bind(gl, 1, this.elementTex, u.u_elementId)
    bind(gl, 2, this.paletteTex, u.u_palette)
    gl.uniform2i(u.u_grid, layers.width, layers.height)
    // Device-pixel position of the domain's bottom-left corner in GL window coordinates (y up).
    const zoom = view.zoom * dpr
    const originX = view.offsetX * dpr
    const originY = this.canvas.height - (view.offsetY * dpr + layers.height * zoom)
    gl.uniform2f(u.u_origin, originX, originY)
    gl.uniform1f(u.u_zoom, zoom)
    gl.uniform4fv(
      u.u_cellColours,
      colours.cellTypes.flatMap((c) => c.map((v) => v / 255)),
    )
    gl.uniform4fv(
      u.u_background,
      bg.map((v) => v / 255),
    )
    gl.uniform4fv(
      u.u_gridColour,
      colours.grid.map((v) => v / 255),
    )
    gl.uniform4fv(
      u.u_borderColour,
      colours.border.map((v) => v / 255),
    )
    gl.uniform1i(u.u_showGrid, view.zoom >= GRID_MIN_ZOOM ? 1 : 0)
    gl.drawArrays(gl.TRIANGLES, 0, 3)
  }

  dispose(): void {
    const { gl } = this
    gl.deleteTexture(this.cellTex)
    gl.deleteTexture(this.elementTex)
    gl.deleteTexture(this.paletteTex)
    gl.deleteProgram(this.program)
    gl.deleteVertexArray(this.vao)
  }
}

function integerTexture(gl: WebGL2RenderingContext): WebGLTexture {
  const t = gl.createTexture()
  gl.bindTexture(gl.TEXTURE_2D, t)
  // Integer textures cannot be filtered; texelFetch ignores the filter, but the texture must
  // still be "complete", which needs NEAREST and no mipmaps.
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST)
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST)
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE)
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE)
  return t
}

function bind(
  gl: WebGL2RenderingContext,
  unit: number,
  texture: WebGLTexture,
  location: WebGLUniformLocation | null,
): void {
  gl.activeTexture(gl.TEXTURE0 + unit)
  gl.bindTexture(gl.TEXTURE_2D, texture)
  gl.uniform1i(location, unit)
}

function link(gl: WebGL2RenderingContext, vs: string, fs: string): WebGLProgram {
  const program = gl.createProgram()
  for (const [type, src] of [
    [gl.VERTEX_SHADER, vs],
    [gl.FRAGMENT_SHADER, fs],
  ] as const) {
    const s = gl.createShader(type)
    if (!s) throw new Error('could not create a shader')
    gl.shaderSource(s, src)
    gl.compileShader(s)
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) {
      throw new Error(`shader compilation failed: ${gl.getShaderInfoLog(s)}`)
    }
    gl.attachShader(program, s)
    gl.deleteShader(s) // freed with the program
  }
  gl.linkProgram(program)
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    throw new Error(`shader link failed: ${gl.getProgramInfoLog(program)}`)
  }
  return program
}
