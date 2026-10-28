// The drawing surface: a WebGL2 canvas for the cells and a transparent 2D canvas on top for the
// selection outline and the brush cursor. The renderer and the pointer handlers are imperative
// code kept in an effect; React renders this component once.
import { useEffect, useRef, useState } from 'react'
import { CELL_COLOURS, coloursFor, rgbaCss } from './colours.ts'
import { edgeRuns, SIDES, WALL_EDGES, type EdgeKindShown } from './edges.ts'
import { elementLabel } from './elements.ts'
import { CellType } from './layers.ts'
import { Renderer } from './render/renderer.ts'
import { useEditor, type EditorState } from './store.ts'
import { pointerDown, pointerMove, pointerUp } from './tools/controller.ts'
import { brushSpans } from './tools/raster.ts'
import { cellToScreen, screenToCell } from './view.ts'

/** Wheel delta (pixels) per zoom step: a mouse notch is 100–120, trackpads send many small deltas. */
const WHEEL_STEP = 100

/** The edge band: CSS pixels between the domain and the band, and the band's width. */
const EDGE_GAP = 2
const EDGE_BAND = 4

/** Band colours where there is no element colour (walls, periodic edges). */
const EDGE_COLOURS: Record<EdgeKindShown, string> = {
  wall: '#94a3b8',
  periodic: '#a78bfa',
  inlet: rgbaCss(CELL_COLOURS[CellType.Inlet]),
  outlet: rgbaCss(CELL_COLOURS[CellType.Outlet]),
}

/** Label colours: light enough to read on the dark margin (element colours can be dark). */
const EDGE_TEXT: Record<EdgeKindShown, string> = {
  wall: '#cbd5e1',
  periodic: '#c4b5fd',
  inlet: '#4ade80',
  outlet: '#f87171',
}

/**
 * Each domain edge's boundary, just outside the domain: a band coloured by kind (wall, periodic
 * dashed, inlets and outlets in their element's colour) and its name (the drawn element's, or
 * the edge kind's) where the stretch is long enough — so a new scene visibly has walls all around
 * (Q14).
 */
function drawEdges(ctx: CanvasRenderingContext2D, s: EditorState): void {
  const { layers, view } = s
  const edges = s.table.grid.edges ?? WALL_EDGES
  const colours = coloursFor(s.table.elements)
  const x0 = view.offsetX
  const x1 = view.offsetX + layers.width * view.zoom
  const yTop = view.offsetY
  const yBottom = view.offsetY + layers.height * view.zoom
  const out = EDGE_GAP + EDGE_BAND / 2 // band centre, outside the domain
  ctx.font = '10px system-ui, sans-serif'
  ctx.textBaseline = 'middle'
  for (const side of SIDES) {
    const vertical = side === 'left' || side === 'right'
    for (const run of edgeRuns(layers, edges, side)) {
      // The run's stretch along the edge in screen pixels, and the band's position across it.
      const a = vertical ? yBottom - run.end * view.zoom : x0 + run.start * view.zoom
      const b = vertical ? yBottom - run.start * view.zoom : x0 + run.end * view.zoom
      const across = { left: x0 - out, right: x1 + out, bottom: yBottom + out, top: yTop - out }[
        side
      ]
      const element = run.elementId ? colours.elements.get(run.elementId) : undefined
      ctx.strokeStyle = element && run.kind !== 'wall' ? rgbaCss(element) : EDGE_COLOURS[run.kind]
      ctx.lineWidth = EDGE_BAND
      ctx.setLineDash(run.kind === 'periodic' ? [6, 4] : [])
      ctx.beginPath()
      if (vertical) {
        ctx.moveTo(across, a)
        ctx.lineTo(across, b)
      } else {
        ctx.moveTo(a, across)
        ctx.lineTo(b, across)
      }
      ctx.stroke()
      // The name, outside the band, centred on the run when it fits.
      const e = run.elementId ? s.table.elements.find((x) => x.id === run.elementId) : undefined
      const text = e ? elementLabel(e) : run.kind
      const w = ctx.measureText(text).width
      if (w + 8 > b - a) continue
      const label = EDGE_BAND / 2 + 8 // from the band centre to the text's centre line
      const at = {
        left: across - label,
        right: across + label,
        bottom: across + label,
        top: across - label,
      }[side]
      ctx.save()
      ctx.translate(vertical ? at : (a + b) / 2, vertical ? (a + b) / 2 : at)
      if (vertical) ctx.rotate(-Math.PI / 2)
      ctx.fillStyle = EDGE_TEXT[run.kind]
      ctx.textAlign = 'center'
      ctx.fillText(text, 0, 0)
      ctx.restore()
    }
  }
  ctx.setLineDash([])
}

/**
 * Draws the domain edges, the selection outline, issue markers, probes and the brush footprint
 * (CSS pixels, scaled by the DPR).
 */
function drawOverlay(ctx: CanvasRenderingContext2D, s: EditorState, dpr: number): void {
  const { width, height } = ctx.canvas
  ctx.setTransform(1, 0, 0, 1, 0, 0)
  ctx.clearRect(0, 0, width, height)
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
  drawEdges(ctx, s)
  ctx.textBaseline = 'alphabetic'
  ctx.textAlign = 'start'
  const H = s.layers.height
  const rect = (x0: number, y0: number, x1: number, y1: number) => {
    // Inclusive cells → the screen rectangle of their outer edges (y up → y down).
    const [sx0, sy0] = cellToScreen(s.view, H, x0, y1 + 1)
    const [sx1, sy1] = cellToScreen(s.view, H, x1 + 1, y0)
    return [
      Math.round(sx0) + 0.5,
      Math.round(sy0) + 0.5,
      Math.round(sx1 - sx0),
      Math.round(sy1 - sy0),
    ] as const
  }
  if (s.selection) {
    const [x, y, w, h] = rect(...s.selection)
    ctx.lineWidth = 1
    ctx.setLineDash([4, 4])
    ctx.strokeStyle = '#000'
    ctx.strokeRect(x, y, w, h)
    ctx.lineDashOffset = 4
    ctx.strokeStyle = '#fff'
    ctx.strokeRect(x, y, w, h)
    ctx.lineDashOffset = 0
    ctx.setLineDash([])
  }
  // Validation issues on cells: a square around the cell, red for errors, amber for warnings
  // (at least 10 px, so they stay visible when zoomed out); the clicked one gets a thicker ring.
  const v = s.validation
  const result = v.status === 'done' ? v.result : v.status === 'pending' ? v.last : null
  for (const issue of result?.issues ?? []) {
    if (!issue.at) continue
    const [x, y] = issue.at
    const [cx, cy] = cellToScreen(s.view, H, x + 0.5, y + 0.5)
    const half = Math.max(5, s.view.zoom * 0.75)
    ctx.lineWidth = 2
    ctx.strokeStyle = issue.severity === 'error' ? '#ef4444' : '#f59e0b'
    ctx.strokeRect(cx - half, cy - half, 2 * half, 2 * half)
  }
  if (s.flash) {
    const [cx, cy] = cellToScreen(s.view, H, s.flash[0] + 0.5, s.flash[1] + 0.5)
    ctx.lineWidth = 3
    ctx.strokeStyle = '#facc15'
    ctx.beginPath()
    ctx.arc(cx, cy, Math.max(12, s.view.zoom * 2), 0, 2 * Math.PI)
    ctx.stroke()
  }
  // Probes: a ring with a cross at the cell centre, the name next to it.
  ctx.font = '11px system-ui, sans-serif'
  ;(s.table.probes ?? []).forEach((p, i) => {
    const [cx, cy] = cellToScreen(s.view, H, p.position[0] + 0.5, p.position[1] + 0.5)
    const r = Math.max(5, s.view.zoom * 0.6)
    const selected = s.selectedProbe === i
    ctx.lineWidth = selected ? 2.5 : 1.5
    for (const [colour, width] of [
      ['rgba(0, 0, 0, 0.7)', ctx.lineWidth + 2],
      [selected ? '#facc15' : '#a855f7', ctx.lineWidth],
    ] as const) {
      ctx.strokeStyle = colour
      ctx.lineWidth = width
      ctx.beginPath()
      ctx.arc(cx, cy, r, 0, 2 * Math.PI)
      ctx.moveTo(cx - r - 3, cy)
      ctx.lineTo(cx + r + 3, cy)
      ctx.moveTo(cx, cy - r - 3)
      ctx.lineTo(cx, cy + r + 3)
      ctx.stroke()
    }
    ctx.fillStyle = 'rgba(0, 0, 0, 0.75)'
    const w = ctx.measureText(p.name).width
    ctx.fillRect(cx + r + 4, cy - r - 15, w + 6, 14)
    ctx.fillStyle = '#fff'
    ctx.fillText(p.name, cx + r + 7, cy - r - 4)
  })
  const brushTool = s.tool === 'pencil' || s.tool === 'eraser'
  if (brushTool && s.cursor && s.view.zoom >= 2) {
    const spans = brushSpans(s.brush)
    const dy = spans.map((r) => r[0])
    const dx0 = Math.min(...spans.map((r) => r[1]))
    const dx1 = Math.max(...spans.map((r) => r[2]))
    const [cx, cy] = s.cursor
    const [x, y, w, h] = rect(cx + dx0, cy + Math.min(...dy), cx + dx1, cy + Math.max(...dy))
    ctx.lineWidth = 1
    ctx.strokeStyle = 'rgba(0, 0, 0, 0.6)'
    ctx.strokeRect(x, y, w, h)
  }
}

export function EditorCanvas() {
  const containerRef = useRef<HTMLDivElement>(null)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const overlayRef = useRef<HTMLCanvasElement>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    const container = containerRef.current!
    const canvas = canvasRef.current!
    const overlayCanvas = overlayRef.current!
    const overlay = overlayCanvas.getContext('2d')!
    let renderer: Renderer
    try {
      renderer = new Renderer(canvas)
    } catch (e) {
      // The failure comes from the browser (no WebGL2), known only once the context is requested.
      // oxlint-disable-next-line react/set-state-in-effect
      setError((e as Error).message)
      return
    }

    // Draw at most once per animation frame, only when something changed.
    let frame = 0
    const draw = () => {
      frame = 0
      const s = useEditor.getState()
      renderer.draw(s.view, window.devicePixelRatio)
      drawOverlay(overlay, s, window.devicePixelRatio)
    }
    const schedule = () => {
      if (frame === 0) frame = requestAnimationFrame(draw)
    }

    let layers: unknown = null
    let elements: unknown = null
    const sync = (s: EditorState) => {
      try {
        if (s.layers !== layers) {
          layers = s.layers
          renderer.setLayers(s.layers)
          setError(null)
        }
        if (s.table.elements !== elements) {
          elements = s.table.elements
          renderer.setColours(coloursFor(s.table.elements))
        }
      } catch (e) {
        setError((e as Error).message)
      }
      schedule()
    }
    sync(useEditor.getState())
    // subscribe() calls back on every store change, outside React's render cycle.
    const unsubscribe = useEditor.subscribe(sync)

    const resize = () => {
      const { width, height } = container.getBoundingClientRect()
      const dpr = window.devicePixelRatio
      renderer.resize(width, height, dpr)
      overlayCanvas.width = Math.max(1, Math.round(width * dpr))
      overlayCanvas.height = Math.max(1, Math.round(height * dpr))
      useEditor.getState().setViewport(width, height)
      schedule()
    }
    const observer = new ResizeObserver(resize)
    observer.observe(container)

    // ---- pointer: tools, pan (hand tool, middle button, or Space + drag), zoom (wheel) ---------
    let pan: { id: number; x: number; y: number } | null = null
    let drawing: number | null = null
    let space = false
    const local = (e: { clientX: number; clientY: number }): [number, number] => {
      const r = overlayCanvas.getBoundingClientRect()
      return [e.clientX - r.left, e.clientY - r.top]
    }
    const cellOf = (e: PointerEvent): [number, number] => {
      const s = useEditor.getState()
      const [sx, sy] = local(e)
      return screenToCell(s.view, s.layers.height, sx, sy)
    }
    const mods = (e: PointerEvent) => ({ shift: e.shiftKey, alt: e.altKey })
    const onPointerDown = (e: PointerEvent) => {
      if (pan || drawing !== null) return
      overlayCanvas.focus()
      if (e.button === 1 || (e.button === 0 && space)) {
        pan = { id: e.pointerId, x: e.clientX, y: e.clientY }
      } else if (e.button === 0) {
        if (pointerDown(cellOf(e), mods(e))) drawing = e.pointerId
        else pan = { id: e.pointerId, x: e.clientX, y: e.clientY }
      } else {
        return
      }
      // Capture: moves and the release arrive here even outside the canvas.
      overlayCanvas.setPointerCapture(e.pointerId)
      e.preventDefault()
    }
    const onPointerMove = (e: PointerEvent) => {
      const s = useEditor.getState()
      if (pan && e.pointerId === pan.id) {
        const v = s.view
        s.setView({
          ...v,
          offsetX: v.offsetX + e.clientX - pan.x,
          offsetY: v.offsetY + e.clientY - pan.y,
        })
        pan.x = e.clientX
        pan.y = e.clientY
      }
      const cell = cellOf(e)
      s.setCursor(cell)
      if (drawing === e.pointerId) pointerMove(cell, mods(e))
    }
    const onPointerUp = (e: PointerEvent) => {
      if (pan && e.pointerId === pan.id) pan = null
      if (drawing === e.pointerId) {
        drawing = null
        pointerUp(cellOf(e), mods(e))
      }
    }
    const onPointerLeave = () => {
      if (drawing === null) useEditor.getState().setCursor(null)
    }
    let wheel = 0
    const onWheel = (e: WheelEvent) => {
      e.preventDefault() // the page itself never scrolls or zooms over the canvas
      wheel += e.deltaMode === WheelEvent.DOM_DELTA_PIXEL ? e.deltaY : e.deltaY * 40
      const s = useEditor.getState()
      while (Math.abs(wheel) >= WHEEL_STEP) {
        const dir = wheel < 0 ? 1 : -1
        wheel += dir * WHEEL_STEP
        s.zoomStep(dir, local(e))
      }
    }
    const setCursorStyle = () => {
      const tool = useEditor.getState().tool
      overlayCanvas.style.cursor =
        space || tool === 'hand' ? 'grab' : tool === 'select' ? 'default' : 'crosshair'
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.code === 'Space' && !(e.target instanceof HTMLInputElement)) {
        space = e.type === 'keydown'
        setCursorStyle()
      }
    }
    const unsubscribeTool = useEditor.subscribe((s, prev) => {
      if (s.tool !== prev.tool) setCursorStyle()
    })
    setCursorStyle()
    overlayCanvas.addEventListener('pointerdown', onPointerDown)
    overlayCanvas.addEventListener('pointermove', onPointerMove)
    overlayCanvas.addEventListener('pointerup', onPointerUp)
    overlayCanvas.addEventListener('pointercancel', onPointerUp)
    overlayCanvas.addEventListener('pointerleave', onPointerLeave)
    // Not passive: preventDefault() must be allowed to stop the browser's own scrolling.
    overlayCanvas.addEventListener('wheel', onWheel, { passive: false })
    window.addEventListener('keydown', onKey)
    window.addEventListener('keyup', onKey)

    return () => {
      unsubscribe()
      unsubscribeTool()
      observer.disconnect()
      cancelAnimationFrame(frame)
      overlayCanvas.removeEventListener('pointerdown', onPointerDown)
      overlayCanvas.removeEventListener('pointermove', onPointerMove)
      overlayCanvas.removeEventListener('pointerup', onPointerUp)
      overlayCanvas.removeEventListener('pointercancel', onPointerUp)
      overlayCanvas.removeEventListener('pointerleave', onPointerLeave)
      overlayCanvas.removeEventListener('wheel', onWheel)
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('keyup', onKey)
      renderer.dispose()
    }
  }, [])

  return (
    <div ref={containerRef} className="relative min-h-0 min-w-0 flex-1 overflow-hidden">
      <canvas ref={canvasRef} className="absolute inset-0 size-full" />
      <canvas
        ref={overlayRef}
        data-testid="editor-canvas"
        tabIndex={-1}
        className="absolute inset-0 size-full touch-none outline-none"
      />
      {error && (
        <div className="absolute inset-0 grid place-items-center p-8 text-center text-sm text-red-400">
          {error}
        </div>
      )}
    </div>
  )
}
