// The viewer's canvas: creates the `Viewer` (WebGL2 + decode worker) once, and handles resizing,
// pan (drag), zoom (wheel) and the value under the pointer. Like the editor's canvas, all of it
// is imperative code in one effect; React renders this component once.
import { useEffect, useRef, useState } from 'react'
import { screenToCell } from '../editor/view.ts'
import { useViewer } from './store.ts'
import { Viewer } from './Viewer.ts'

/** Wheel delta (pixels) per zoom step, as in the editor. */
const WHEEL_STEP = 100

declare global {
  interface Window {
    /** The live viewer, for end-to-end tests (development builds only). */
    __viewer?: Viewer
  }
}

export function ViewerCanvas(props: { onReady: (viewer: Viewer | null) => void }) {
  const containerRef = useRef<HTMLDivElement>(null)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const [error, setError] = useState<string | null>(null)
  const { onReady } = props

  useEffect(() => {
    const container = containerRef.current!
    const canvas = canvasRef.current!
    let viewer: Viewer
    try {
      viewer = new Viewer(canvas)
    } catch (e) {
      // No WebGL2 (or no workers): known only once the context is requested.
      // oxlint-disable-next-line react/set-state-in-effect
      setError((e as Error).message)
      onReady(null)
      return
    }
    if (import.meta.env.DEV) window.__viewer = viewer
    onReady(viewer)

    const resize = () => {
      const { width, height } = container.getBoundingClientRect()
      viewer.resize(width, height, window.devicePixelRatio)
      useViewer.getState().setViewport(width, height)
    }
    const observer = new ResizeObserver(resize)
    observer.observe(container)

    const local = (e: { clientX: number; clientY: number }): [number, number] => {
      const r = canvas.getBoundingClientRect()
      return [e.clientX - r.left, e.clientY - r.top]
    }
    let pan: { id: number; x: number; y: number } | null = null
    // The brush or the release line being dragged (the toolbar's tool), instead of a pan.
    let tool: { id: number; kind: 'paint' | 'line'; from: [number, number] } | null = null
    const cellAt = (e: { clientX: number; clientY: number }): [number, number] | null => {
      const s = useViewer.getState()
      if (!s.grid) return null
      const [sx, sy] = local(e)
      return screenToCell(s.view, s.grid.height, sx, sy)
    }
    const onPointerDown = (e: PointerEvent) => {
      if (pan || tool || (e.button !== 0 && e.button !== 1)) return
      const s = useViewer.getState()
      const cell = cellAt(e)
      if (e.button === 0 && s.tool !== 'pan' && cell) {
        tool = { id: e.pointerId, kind: s.tool, from: cell }
        canvas.setPointerCapture(e.pointerId)
        if (s.tool === 'paint') viewer.paint(cell[0], cell[1])
        else s.setReleaseLine([cell[0], cell[1], cell[0], cell[1]])
        e.preventDefault()
        return
      }
      pan = { id: e.pointerId, x: e.clientX, y: e.clientY }
      canvas.setPointerCapture(e.pointerId)
      canvas.style.cursor = 'grabbing'
      e.preventDefault()
    }
    const onPointerMove = (e: PointerEvent) => {
      const s = useViewer.getState()
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
      if (!s.grid) return
      const [sx, sy] = local(e)
      const cell = screenToCell(s.view, s.grid.height, sx, sy)
      if (tool && e.pointerId === tool.id) {
        if (tool.kind === 'paint') viewer.paint(cell[0], cell[1])
        else s.setReleaseLine([tool.from[0], tool.from[1], cell[0], cell[1]])
      }
      if (!pan) canvas.style.cursor = s.tool === 'pan' ? 'grab' : 'crosshair'
      s.setCursor({ cell, value: viewer.valueAt(cell[0], cell[1]) })
    }
    const onPointerUp = (e: PointerEvent) => {
      if (pan && e.pointerId === pan.id) {
        pan = null
        canvas.style.cursor = useViewer.getState().tool === 'pan' ? 'grab' : 'crosshair'
      }
      if (tool && e.pointerId === tool.id) {
        const s = useViewer.getState()
        const l = s.releaseLine
        // A click without a drag leaves no line.
        if (tool.kind === 'line' && l && Math.hypot(l[2] - l[0], l[3] - l[1]) < 1) {
          s.setReleaseLine(null)
        }
        tool = null
      }
    }
    const onPointerLeave = () => useViewer.getState().setCursor(null)
    let wheel = 0
    const onWheel = (e: WheelEvent) => {
      e.preventDefault()
      wheel += e.deltaMode === WheelEvent.DOM_DELTA_PIXEL ? e.deltaY : e.deltaY * 40
      while (Math.abs(wheel) >= WHEEL_STEP) {
        const dir = wheel < 0 ? 1 : -1
        wheel += dir * WHEEL_STEP
        useViewer.getState().zoomStep(dir, local(e))
      }
    }
    canvas.style.cursor = 'grab'
    canvas.addEventListener('pointerdown', onPointerDown)
    canvas.addEventListener('pointermove', onPointerMove)
    canvas.addEventListener('pointerup', onPointerUp)
    canvas.addEventListener('pointercancel', onPointerUp)
    canvas.addEventListener('pointerleave', onPointerLeave)
    canvas.addEventListener('wheel', onWheel, { passive: false })

    return () => {
      observer.disconnect()
      canvas.removeEventListener('pointerdown', onPointerDown)
      canvas.removeEventListener('pointermove', onPointerMove)
      canvas.removeEventListener('pointerup', onPointerUp)
      canvas.removeEventListener('pointercancel', onPointerUp)
      canvas.removeEventListener('pointerleave', onPointerLeave)
      canvas.removeEventListener('wheel', onWheel)
      onReady(null)
      if (window.__viewer === viewer) delete window.__viewer
      viewer.dispose()
    }
  }, [onReady])

  return (
    <div ref={containerRef} className="relative min-h-0 min-w-0 flex-1 overflow-hidden">
      <canvas
        ref={canvasRef}
        data-testid="viewer-canvas"
        className="absolute inset-0 size-full touch-none"
      />
      {error && (
        <div className="absolute inset-0 grid place-items-center p-8 text-center text-sm text-red-400">
          {error}
        </div>
      )}
    </div>
  )
}
