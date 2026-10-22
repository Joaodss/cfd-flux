// GLSL ES 3.0 shaders of the overlays drawn over the field.

/**
 * Velocity arrows, instanced: one instance per point of a screen-space grid, six vertices each
 * (shaft and two barbs, as GL_LINES). The vertex shader reads the velocity texture itself
 * (texture reads in vertex shaders are core WebGL2), so no per-arrow data is uploaded. Arrows on
 * non-fluid cells or outside the domain collapse to a point outside the clip volume.
 */
export const VECTOR_VERTEX = `#version 300 es
precision highp float;
precision highp int;
precision highp usampler2D;

uniform usampler2D u_cellType;
uniform sampler2D u_field;      // RG16F velocity, one texel per block
uniform sampler2D u_fraction;
uniform ivec2 u_grid;
uniform vec2 u_fieldCells;
uniform vec2 u_origin;          // device pixels of the domain's bottom-left corner
uniform float u_zoom;
uniform vec2 u_canvas;          // device pixels
uniform float u_spacing;        // device pixels between arrows
uniform ivec2 u_first;          // grid index of the first column / row (may be negative)
uniform int u_columns;
uniform float u_scale;          // device pixels per m/s
uniform float u_maxLength;      // device pixels

void main() {
  int i = gl_InstanceID % u_columns;
  int j = gl_InstanceID / u_columns;
  vec2 p = u_origin + (vec2(u_first + ivec2(i, j)) + 0.5) * u_spacing;
  vec2 c = (p - u_origin) / u_zoom;
  ivec2 cell = ivec2(floor(c));
  gl_Position = vec4(2.0, 2.0, 2.0, 1.0);
  if (any(lessThan(cell, ivec2(0))) || any(greaterThanEqual(cell, u_grid))) return;
  if (texelFetch(u_cellType, cell, 0).r != 0u) return;
  vec2 uv = c / u_fieldCells;
  vec2 v = textureLod(u_field, uv, 0.0).rg / max(textureLod(u_fraction, uv, 0.0).r, 1.0 / 255.0);
  float speed = length(v);
  float len = min(speed * u_scale, u_maxLength);
  if (!(len >= 1.0)) return;
  vec2 d = v / speed;
  vec2 tip = p + d * len * 0.5;
  vec2 tail = p - d * len * 0.5;
  float barb = min(len * 0.35, 8.0);
  // The barbs point back from the tip, 25 degrees either side of the shaft.
  float s = (gl_VertexID < 4) ? 0.4226 : -0.4226;
  vec2 back = -d * 0.9063 + vec2(-d.y, d.x) * s;
  vec2 pos = tail;
  if (gl_VertexID == 1 || gl_VertexID == 2 || gl_VertexID == 4) pos = tip;
  if (gl_VertexID == 3 || gl_VertexID == 5) pos = tip + back * barb;
  gl_Position = vec4(pos / u_canvas * 2.0 - 1.0, 0.0, 1.0);
}
`

export const LINE_FRAGMENT = `#version 300 es
precision highp float;
uniform vec4 u_colour;
out vec4 colour;
void main() { colour = u_colour; }
`

/** Segments given in cells (x, y per vertex), drawn as GL_LINES. */
export const LINE_VERTEX = `#version 300 es
precision highp float;
layout(location = 0) in vec2 a_cell;
uniform vec2 u_origin;
uniform float u_zoom;
uniform vec2 u_canvas;
void main() {
  vec2 p = u_origin + a_cell * u_zoom;
  gl_Position = vec4(p / u_canvas * 2.0 - 1.0, 0.0, 1.0);
}
`
