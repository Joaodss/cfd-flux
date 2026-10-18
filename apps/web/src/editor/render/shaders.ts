// GLSL ES 3.0 shaders of the editor renderer.

/** One triangle larger than the screen, from gl_VertexID alone (no vertex buffer). */
export const VERTEX = `#version 300 es
void main() {
  vec2 p = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2);
  gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
`

/**
 * Each screen pixel → its cell (y up, like GL window coordinates) → colour: the element's palette
 * entry when it has one, else its cell type's colour; grid lines and a one-pixel domain border.
 */
export const FRAGMENT = `#version 300 es
precision highp float;
precision highp int;
precision highp usampler2D;

uniform usampler2D u_cellType;
uniform usampler2D u_elementId;
uniform sampler2D u_palette;
uniform ivec2 u_grid;
uniform vec2 u_origin;   // device pixels of the domain's bottom-left corner
uniform float u_zoom;    // device pixels per cell
uniform vec4 u_cellColours[6];
uniform vec4 u_background;
uniform vec4 u_gridColour;
uniform vec4 u_borderColour;
uniform bool u_showGrid;

out vec4 colour;

void main() {
  vec2 c = (gl_FragCoord.xy - u_origin) / u_zoom;
  ivec2 cell = ivec2(floor(c));
  if (any(lessThan(cell, ivec2(0))) || any(greaterThanEqual(cell, u_grid))) {
    vec2 outside = max(-c, c - vec2(u_grid)) * u_zoom;
    colour = max(outside.x, outside.y) <= 1.0 ? u_borderColour : u_background;
    return;
  }
  uint ct = texelFetch(u_cellType, cell, 0).r;
  uint id = texelFetch(u_elementId, cell, 0).r;
  vec4 col = u_cellColours[min(ct, 5u)];
  if (id != 0u) {
    vec4 p = texelFetch(u_palette, ivec2(int(id & 255u), int(id >> 8u)), 0);
    if (p.a > 0.0) col = p;
  }
  if (u_showGrid) {
    vec2 f = fract(c) * u_zoom;
    if (f.x < 1.0 || f.y < 1.0) col = mix(col, u_gridColour, u_gridColour.a);
  }
  colour = vec4(col.rgb, 1.0);
}
`
