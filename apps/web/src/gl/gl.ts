// WebGL2 helpers shared by the editor's and the viewer's renderers.

export function integerTexture(gl: WebGL2RenderingContext): WebGLTexture {
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

export function bind(
  gl: WebGL2RenderingContext,
  unit: number,
  texture: WebGLTexture,
  location: WebGLUniformLocation | null,
): void {
  gl.activeTexture(gl.TEXTURE0 + unit)
  gl.bindTexture(gl.TEXTURE_2D, texture)
  gl.uniform1i(location, unit)
}

export function link(gl: WebGL2RenderingContext, vs: string, fs: string): WebGLProgram {
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
