// The editor's store (Zustand): the scene table, the view and UI state. Pixel layers are only
// referenced here; tools and the renderer read and write them directly (ADR-005).
import { create } from 'zustand'
import { immer } from 'zustand/middleware/immer'
import { newDocument } from '../scene/defaults.ts'
import type { Document, SceneTable } from '../scene/io.ts'
import type { Layers } from './layers.ts'
import { clampZoom, fitView, stepZoom, zoomAt, type View } from './view.ts'

export type ToolId = 'hand'

export interface EditorState {
  table: SceneTable
  layers: Layers
  fluidId: Document['fluidId']
  view: View
  /** Size of the canvas in CSS pixels (set by the canvas component). */
  viewport: { width: number; height: number }
  /** The cell under the pointer, or null when the pointer is off the canvas. */
  cursor: [number, number] | null
  tool: ToolId
  /** Bumped when the layers changed outside the store (tools), so the canvas redraws. */
  layersVersion: number

  load(doc: Document): void
  setViewport(width: number, height: number): void
  setView(view: View): void
  /** Zooms one ladder step in (+1) or out (−1), about a screen point (default: the centre). */
  zoomStep(direction: 1 | -1, at?: [number, number]): void
  zoomTo(zoom: number): void
  fit(): void
  setCursor(cell: [number, number] | null): void
  setTool(tool: ToolId): void
  layersChanged(): void
}

export const useEditor = create<EditorState>()(
  immer((set, get) => {
    const initial = newDocument()
    return {
      table: initial.table,
      layers: initial.layers,
      fluidId: initial.fluidId,
      view: { zoom: 1, offsetX: 0, offsetY: 0 },
      viewport: { width: 0, height: 0 },
      cursor: null,
      tool: 'hand',
      layersVersion: 0,

      load: (doc) =>
        set((s) => {
          s.table = doc.table
          s.layers = doc.layers
          s.fluidId = doc.fluidId
          s.cursor = null
          const { width, height } = s.viewport
          if (width > 0) s.view = fitView(doc.layers.width, doc.layers.height, width, height)
        }),
      setViewport: (width, height) =>
        set((s) => {
          const first = s.viewport.width === 0
          s.viewport = { width, height }
          if (first) s.view = fitView(s.layers.width, s.layers.height, width, height)
        }),
      setView: (view) => set({ view }),
      zoomStep: (direction, at) => {
        const { view, viewport } = get()
        const [sx, sy] = at ?? [viewport.width / 2, viewport.height / 2]
        set({ view: zoomAt(view, stepZoom(view.zoom, direction), sx, sy) })
      },
      zoomTo: (zoom) => {
        const { view, viewport } = get()
        set({ view: zoomAt(view, clampZoom(zoom), viewport.width / 2, viewport.height / 2) })
      },
      fit: () => {
        const { layers, viewport } = get()
        set({ view: fitView(layers.width, layers.height, viewport.width, viewport.height) })
      },
      setCursor: (cell) => {
        const c = get().cursor
        if (c === cell || (c && cell && c[0] === cell[0] && c[1] === cell[1])) return
        set({ cursor: cell })
      },
      setTool: (tool) => set({ tool }),
      layersChanged: () => set((s) => void s.layersVersion++),
    }
  }),
)

// Development builds expose the store for debugging from the console and for browser tests.
if (import.meta.env.DEV) {
  ;(globalThis as { __editor?: typeof useEditor }).__editor = useEditor
}
