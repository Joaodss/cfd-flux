// The drawing surface: a WebGL2 canvas for the cells and a transparent 2D canvas on top for the
// selection outline and the brush cursor. The renderer and the pointer handlers are imperative
// code kept in an effect; React renders this component once.
import { useEffect, useRef, useState } from 'react'
import { coloursFor } from './colours.ts'
import { Renderer } from './render/renderer.ts'
import { useEditor, type EditorState } from './store.ts'
import { pointerDown, pointerMove, pointerUp } from './tools/controller.ts'
import { brushSpans } from './tools/raster.ts'
import { cellToScreen, screenToCell } from './view.ts'

/** Wheel delta (pixels) per zoom step: a mouse notch is 100–120, trackpads send many small deltas. */
const WHEEL_STEP = 100

/** Draws the selection outline and the brush footprint (CSS pixels, scaled by the DPR). */
function drawOverlay(ctx: CanvasRenderingContext2D, s: EditorState, dpr: number): void {
  const { width, height } = ctx.canvas
  ctx.setTransform(1, 0, 0, 1, 0, 0)
  ctx.clearRect(0, 0, width, height)
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
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
