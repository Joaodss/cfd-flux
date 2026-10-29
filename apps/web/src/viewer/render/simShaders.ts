// GLSL ES 3.0 shaders of the tracer particles and the dye. Positions are in cells, origin at the
// domain's bottom-left corner, as everywhere in the viewer.

/** Hash of an integer (PCG): the particles' random numbers, from their index and a step counter. */
const RANDOM = `
uint pcg(uint v) {
  uint s = v * 747796405u + 2891336453u;
  uint w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
  return (w >> 22u) ^ w;
}
float rand(uint a) { return float(pcg(a)) / 4294967295.0; }
`

/**
 * One step of every particle: a texel of the state texture (RG32F: x, y; x < 0 = not released).
 * Live particles move by the velocity at the frame shown (midpoint rule) and die when they reach
 * a non-fluid cell or leave the domain; dead ones are released with probability `u_release` at
 * the source (the inlets' neighbours, a line, or anywhere). Texels from `u_count` on are not in use
 * (a lower density): they stay, or become, unreleased.
 */
export const PARTICLE_UPDATE = `#version 300 es
precision highp float;
precision highp int;
precision highp usampler2D;
${RANDOM}
uniform sampler2D u_state;
uniform sampler2D u_velocity;   // RG16F or RG32F
uniform sampler2D u_fraction;
uniform usampler2D u_cellType;
uniform sampler2D u_spawns;     // RG32F, 1024 per row
uniform int u_mode;             // 0 spawn points, 1 line, 2 anywhere
uniform int u_spawnCount;
uniform vec4 u_line;            // x0, y0, x1, y1
uniform ivec2 u_grid;
uniform vec2 u_fieldCells;
uniform float u_dt;             // simulated seconds
uniform float u_cellSize;       // metres per cell
uniform float u_release;
uniform int u_count;
uniform uint u_seed;
out vec4 outState;

bool blocked(vec2 p) {
  if (p.x < 0.0 || p.y < 0.0 || p.x >= float(u_grid.x) || p.y >= float(u_grid.y)) return true;
  return texelFetch(u_cellType, ivec2(floor(p)), 0).r != 0u;
}

vec2 velocityAt(vec2 p) {
  vec2 uv = p / u_fieldCells;
  return texture(u_velocity, uv).rg / max(texture(u_fraction, uv).r, 1.0 / 255.0);
}

void main() {
  ivec2 t = ivec2(gl_FragCoord.xy);
  vec2 p = texelFetch(u_state, t, 0).xy;
  uint id = uint(t.y * textureSize(u_state, 0).x + t.x);
  if (int(id) >= u_count) {
    outState = vec4(-1.0, -1.0, 0.0, 1.0);
    return;
  }
  if (p.x >= 0.0) {
    vec2 mid = p + 0.5 * velocityAt(p) * u_dt / u_cellSize;
    vec2 next = p + velocityAt(mid) * u_dt / u_cellSize;
    outState = blocked(next) ? vec4(-1.0, -1.0, 0.0, 1.0) : vec4(next, 0.0, 1.0);
    return;
  }
  uint key = id ^ (u_seed * 2654435761u);
  vec2 spawn = vec2(-1.0);
  if (rand(key) < u_release) {
    float a = rand(key + 1u);
    float b = rand(key + 2u);
    if (u_mode == 0 && u_spawnCount > 0) {
      int i = min(int(a * float(u_spawnCount)), u_spawnCount - 1);
      spawn = texelFetch(u_spawns, ivec2(i % 1024, i / 1024), 0).xy + (vec2(b, rand(key + 3u)) - 0.5) * 0.8;
    } else if (u_mode == 1) {
      spawn = mix(u_line.xy, u_line.zw, a);
    } else if (u_mode == 2) {
      spawn = vec2(a, b) * vec2(u_grid);
    }
    if (spawn.x < 0.0 || blocked(spawn)) spawn = vec2(-1.0);
  }
  outState = vec4(spawn, 0.0, 1.0);
}
`

/** The particles as points, read from the state texture by the vertex shader. */
export const PARTICLE_VERTEX = `#version 300 es
precision highp float;
precision highp int;
uniform sampler2D u_state;
uniform vec2 u_origin;
uniform float u_zoom;
uniform vec2 u_canvas;
uniform float u_size;
void main() {
  ivec2 size = textureSize(u_state, 0);
  vec2 p = texelFetch(u_state, ivec2(gl_VertexID % size.x, gl_VertexID / size.x), 0).xy;
  gl_PointSize = u_size;
  if (p.x < 0.0) {
    gl_Position = vec4(2.0, 2.0, 2.0, 1.0);
    return;
  }
  gl_Position = vec4((u_origin + p * u_zoom) / u_canvas * 2.0 - 1.0, 0.0, 1.0);
}
`

export const PARTICLE_FRAGMENT = `#version 300 es
precision highp float;
uniform vec4 u_colour;
out vec4 colour;
void main() {
  vec2 d = gl_PointCoord - 0.5;
  if (dot(d, d) > 0.25) discard;
  colour = u_colour;
}
`

/**
 * One semi-Lagrangian step of the dye (RGBA16F: three concentrations in RGB): each texel takes
 * the value found one step upstream, decays, is cleared in non-fluid cells, set to 1 in the cells
 * next to an inlet (a stripe of each dye in turn across the inlet) and raised by brush stamps.
 */
export const DYE_ADVECT = `#version 300 es
precision highp float;
precision highp int;
precision highp usampler2D;

uniform sampler2D u_dye;
uniform sampler2D u_velocity;
uniform sampler2D u_fraction;
uniform usampler2D u_cellType;
uniform ivec2 u_grid;
uniform vec2 u_dyeCells;        // the dye texture's extent in cells
uniform int u_k;                // cells per dye texel
uniform float u_dt;
uniform float u_cellSize;
uniform float u_decay;          // factor per step
uniform float u_stripe;         // cells per inlet stripe
uniform int u_stamps;
uniform vec4 u_stamp[8];        // x, y, radius (cells), dye
out vec4 outDye;

void main() {
  vec2 c = gl_FragCoord.xy * float(u_k);   // the texel centre, in cells
  ivec2 cell = ivec2(floor(c));
  if (any(greaterThanEqual(cell, u_grid)) || texelFetch(u_cellType, cell, 0).r != 0u) {
    outDye = vec4(0.0);
    return;
  }
  vec2 uv = c / u_dyeCells;
  vec2 v = texture(u_velocity, uv).rg / max(texture(u_fraction, uv).r, 1.0 / 255.0);
  vec3 d = texture(u_dye, (c - v * u_dt / u_cellSize) / u_dyeCells).rgb * u_decay;
  for (int i = 0; i < 4; i++) {
    ivec2 n = cell + (i == 0 ? ivec2(-1, 0) : i == 1 ? ivec2(1, 0) : i == 2 ? ivec2(0, -1) : ivec2(0, 1));
    if (any(lessThan(n, ivec2(0))) || any(greaterThanEqual(n, u_grid))) continue;
    if (texelFetch(u_cellType, n, 0).r == 2u) {
      int channel = int(mod(floor(float(cell.y) / u_stripe), 3.0));
      d[channel] = 1.0;
    }
  }
  for (int i = 0; i < 8; i++) {
    if (i >= u_stamps) break;
    float w = smoothstep(u_stamp[i].z, 0.0, length(c - u_stamp[i].xy));
    int channel = int(u_stamp[i].w);
    d[channel] = max(d[channel], w);
  }
  outDye = vec4(clamp(d, 0.0, 1.0), 1.0);
}
`

/** The dye over the field: its colours mixed by concentration, opaque where concentrated. */
export const DYE_DISPLAY = `#version 300 es
precision highp float;
precision highp int;
precision highp usampler2D;

uniform sampler2D u_dye;
uniform usampler2D u_cellType;
uniform ivec2 u_grid;
uniform vec2 u_dyeCells;
uniform vec2 u_origin;
uniform float u_zoom;
uniform vec3 u_colours[3];
out vec4 colour;

void main() {
  vec2 c = (gl_FragCoord.xy - u_origin) / u_zoom;
  ivec2 cell = ivec2(floor(c));
  if (any(lessThan(cell, ivec2(0))) || any(greaterThanEqual(cell, u_grid))) discard;
  if (texelFetch(u_cellType, cell, 0).r != 0u) discard;
  vec3 d = texture(u_dye, c / u_dyeCells).rgb;
  float total = d.r + d.g + d.b;
  if (total < 0.004) discard;
  vec3 rgb = (d.r * u_colours[0] + d.g * u_colours[1] + d.b * u_colours[2]) / total;
  colour = vec4(rgb, min(total, 1.0) * 0.85);
}
`
