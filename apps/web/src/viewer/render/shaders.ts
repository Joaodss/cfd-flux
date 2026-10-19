// GLSL ES 3.0 shader of the field viewer (the vertex shader is the editor's full-screen triangle).

/**
 * Each screen pixel → its cell (y up). Non-fluid cells get their cell type's colour. Fluid cells
 * sample the field (f16 texture, one texel per k × k block, bilinear unless `u_nearest`), divide
 * by the block's fluid fraction (the samples are 0 at non-fluid cells, so this is the mean over
 * the block's fluid cells), reduce to the displayed scalar and look its colour up in the map.
 */
export const FIELD_FRAGMENT = `#version 300 es
precision highp float;
precision highp int;
precision highp usampler2D;

uniform usampler2D u_cellType;  // full resolution, R8UI
uniform sampler2D u_field;      // R16F or RG16F, ceil(W/k) × ceil(H/k)
uniform sampler2D u_fraction;   // R8, same size: fluid fraction of each block
uniform sampler2D u_lut;        // 256 × 1 RGBA8 colour map
uniform bool u_hasField;
uniform ivec2 u_grid;
uniform vec2 u_fieldCells;      // the field texture's extent in cells: size × k
uniform int u_component;        // 0 scalar, 1 magnitude, 2 x, 3 y
uniform vec2 u_range;           // value ↦ 0 and 1
uniform vec2 u_origin;          // device pixels of the domain's bottom-left corner
uniform float u_zoom;           // device pixels per cell
uniform vec4 u_cellColours[6];
uniform vec4 u_background;
uniform vec4 u_borderColour;
uniform vec4 u_noData;

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
  if (ct != 0u) {
    colour = vec4(u_cellColours[min(ct, 5u)].rgb, 1.0);
    return;
  }
  if (!u_hasField) {
    colour = u_cellColours[0];
    return;
  }
  vec2 uv = c / u_fieldCells;
  vec2 v = texture(u_field, uv).rg;
  float f = texture(u_fraction, uv).r;
  v /= max(f, 1.0 / 255.0);
  float s = u_component == 1 ? length(v) : u_component == 3 ? v.y : v.x;
  if (isinf(s) || isnan(s)) {
    colour = u_noData;
    return;
  }
  float t = clamp((s - u_range.x) / (u_range.y - u_range.x), 0.0, 1.0);
  // Texel centres: entry i of 256 sits at (i + 0.5) / 256.
  colour = vec4(texture(u_lut, vec2((t * 255.0 + 0.5) / 256.0, 0.5)).rgb, 1.0);
}
`
