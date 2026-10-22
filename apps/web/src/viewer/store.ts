// The viewer's UI state (a Zustand store, like the editor's): which quantity is shown and how
// (colour map, range), the current frame's header and statistics, the view (zoom/pan) and the
// value under the pointer. The frames themselves are typed arrays owned by `Viewer`, never here.
import { create } from 'zustand'
import { fitView, stepZoom, zoomAt, type View } from '../editor/view.ts'
import type { FrameHeader } from '../frames/lffr.ts'
import type { ReleaseMode } from './simulation.ts'
import { quantitiesOf, type Quantity } from './quantities.ts'
import { accumulate, rangeFor, type RangeMode, type Seen } from './ranges.ts'
import type { FrameStats } from './stats.ts'
import { INITIAL_TIMELINE, type Timeline } from './timeline.ts'

export interface ViewerState {
  /** The run's grid (cells), once its scene is loaded. */
  grid: { width: number; height: number; cellSize: number } | null
  quantities: Quantity[]
  /** Key of the quantity shown. */
  selected: string | null
  /** Per quantity: colour map, range mode and manual range chosen by the user. */
  colormaps: Record<string, string>
  modes: Record<string, RangeMode>
  manual: Record<string, [number, number]>
  /** Show blocks as they are, without bilinear interpolation. */
  nearest: boolean
  /** Overlays over the field. */
  overlays: { vectors: boolean; streamlines: boolean; particles: boolean; dye: boolean }
  /** Where particles are released, and the line drawn for `line`. */
  release: ReleaseMode
  releaseLine: [number, number, number, number] | null
  /** What dragging on the canvas does. */
  tool: 'pan' | 'line' | 'paint'
  /** The dye the brush paints (0-2). */
  paintDye: number
  /** Whether this GPU can render to float textures (particles and dye need it). */
  floatTargets: boolean
  /** What the frames so far span (seeded from the stream's `hello`). */
  seen: Seen
  frame: { header: FrameHeader; stats: FrameStats } | null
  /** Frames shown since the view opened. */
  received: number
  /** Play/pause/scrub state (the Viewer drives it). */
  timeline: Timeline
  view: View
  viewport: { width: number; height: number }
  /** The cell under the pointer and the shown quantity's value there. */
  cursor: { cell: [number, number]; value: number | null } | null

  reset(): void
  setGrid(grid: ViewerState['grid']): void
  seed(seen: Seen): void
  setFrame(header: FrameHeader, stats: FrameStats): void
  select(key: string): void
  setColormap(key: string, name: string): void
  setMode(key: string, mode: RangeMode): void
  setManual(key: string, range: [number, number]): void
  setNearest(nearest: boolean): void
  setOverlay(key: keyof ViewerState['overlays'], on: boolean): void
  setRelease(release: ReleaseMode): void
  setReleaseLine(line: ViewerState['releaseLine']): void
  setTool(tool: ViewerState['tool']): void
  setPaintDye(dye: number): void
  setFloatTargets(ok: boolean): void
  setView(view: View): void
  setViewport(width: number, height: number): void
  zoomStep(direction: 1 | -1, at?: [number, number]): void
  fit(): void
  setCursor(cursor: ViewerState['cursor']): void
  setTimeline(patch: Partial<Timeline>): void
}

const INITIAL = {
  grid: null,
  quantities: [],
  selected: null,
  colormaps: {},
  modes: {},
  manual: {},
  nearest: false,
  overlays: { vectors: false, streamlines: false, particles: false, dye: false },
  release: 'inlets',
  releaseLine: null,
  tool: 'pan',
  paintDye: 0,
  floatTargets: true,
  seen: {},
  frame: null,
  received: 0,
  timeline: INITIAL_TIMELINE,
  view: { zoom: 1, offsetX: 0, offsetY: 0 },
  viewport: { width: 0, height: 0 },
  cursor: null,
} satisfies Partial<ViewerState>

export const useViewer = create<ViewerState>()((set, get) => ({
  ...INITIAL,
  reset: () => set({ ...INITIAL, viewport: get().viewport, floatTargets: get().floatTargets }),
  setGrid: (grid) => {
    const { viewport } = get()
    set({
      grid,
      ...(grid && viewport.width > 0
        ? { view: fitView(grid.width, grid.height, viewport.width, viewport.height) }
        : {}),
    })
  },
  seed: (seen) => set((s) => ({ seen: { ...seen, ...s.seen } })),
  setFrame: (header, stats) =>
    set((s) => {
      const quantities =
        s.quantities.length > 0 && sameFields(s.quantities, header)
          ? s.quantities
          : quantitiesOf(header.fields)
      const selected =
        s.selected && quantities.some((q) => q.key === s.selected)
          ? s.selected
          : (quantities[0]?.key ?? null)
      return {
        frame: { header, stats },
        quantities,
        selected,
        seen: accumulate(s.seen, stats),
        received: s.received + 1,
      }
    }),
  select: (selected) => set({ selected }),
  setColormap: (key, name) => set((s) => ({ colormaps: { ...s.colormaps, [key]: name } })),
  setMode: (key, mode) =>
    set((s) => {
      // Switching to manual starts from the range shown so far.
      const manual =
        mode === 'manual' && !s.manual[key]
          ? { ...s.manual, [key]: currentRange(s) ?? [0, 1] }
          : s.manual
      return { modes: { ...s.modes, [key]: mode }, manual }
    }),
  setManual: (key, range) =>
    set((s) => ({ manual: { ...s.manual, [key]: range }, modes: { ...s.modes, [key]: 'manual' } })),
  setNearest: (nearest) => set({ nearest }),
  setOverlay: (key, on) => set((s) => ({ overlays: { ...s.overlays, [key]: on } })),
  setRelease: (release) => set({ release }),
  setReleaseLine: (releaseLine) => set({ releaseLine }),
  setTool: (tool) => set({ tool }),
  setPaintDye: (paintDye) => set({ paintDye }),
  setFloatTargets: (floatTargets) => set({ floatTargets }),
  setView: (view) => set({ view }),
  setViewport: (width, height) => {
    const { viewport, grid } = get()
    const first = viewport.width === 0
    set({
      viewport: { width, height },
      ...(first && grid ? { view: fitView(grid.width, grid.height, width, height) } : {}),
    })
  },
  zoomStep: (direction, at) => {
    const { view, viewport } = get()
    const [sx, sy] = at ?? [viewport.width / 2, viewport.height / 2]
    set({ view: zoomAt(view, stepZoom(view.zoom, direction), sx, sy) })
  },
  fit: () => {
    const { grid, viewport } = get()
    if (grid) set({ view: fitView(grid.width, grid.height, viewport.width, viewport.height) })
  },
  setCursor: (cursor) => set({ cursor }),
  setTimeline: (patch) => set((s) => ({ timeline: { ...s.timeline, ...patch } })),
}))

function sameFields(quantities: Quantity[], header: FrameHeader): boolean {
  const names = new Set(quantities.map((q) => q.field))
  return header.fields.length === names.size && header.fields.every((f) => names.has(f.name))
}

/** The quantity shown, or null before the first frame. */
export function currentQuantity(s: ViewerState): Quantity | null {
  return s.quantities.find((q) => q.key === s.selected) ?? null
}

/** The range mode of a quantity: chosen, else symmetric for diverging quantities, else all frames. */
export function modeOf(s: ViewerState, q: Quantity): RangeMode {
  return s.modes[q.key] ?? (q.symmetric ? 'symmetric' : 'all')
}

export function colormapOf(s: ViewerState, q: Quantity): string {
  return s.colormaps[q.key] ?? q.colormap
}

/** The colour range of the quantity shown. */
export function currentRange(s: ViewerState): [number, number] | null {
  const q = currentQuantity(s)
  if (!q) return null
  return rangeFor(modeOf(s, q), s.frame?.stats[q.key], s.seen[q.key], s.manual[q.key])
}
