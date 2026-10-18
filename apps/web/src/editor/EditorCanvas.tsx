// The drawing surface: a WebGL2 canvas filling its container. The renderer and the pointer
// handlers are imperative code kept in refs and effects; React renders this component once.
import { useEffect, useRef, useState } from 'react'
import { coloursFor } from './colours.ts'
import { Renderer } from './render/renderer.ts'
import { useEditor } from './store.ts'
import { screenToCell } from './view.ts'

/** Wheel delta (pixels) per zoom step: a mouse notch is 100–120, trackpads send many small deltas. */
const WHEEL_STEP = 100

export function EditorCanvas() {
  const containerRef = useRef<HTMLDivElement>(null)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    const container = containerRef.current!
    const canvas = canvasRef.current!
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
      renderer.draw(useEditor.getState().view, window.devicePixelRatio)
    }
    const schedule = () => {
      if (frame === 0) frame = requestAnimationFrame(draw)
    }

    let layers: unknown = null
    let elements: unknown = null
    const sync = (s: ReturnType<typeof useEditor.getState>) => {
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
      renderer.resize(width, height, window.devicePixelRatio)
      useEditor.getState().setViewport(width, height)
      schedule()
    }
    const observer = new ResizeObserver(resize)
    observer.observe(container)

    // ---- pointer: pan (hand tool, middle button, or space + drag), zoom (wheel), cursor --------
    let pan: { id: number; x: number; y: number } | null = null
    let space = false
    const local = (e: { clientX: number; clientY: number }): [number, number] => {
      const r = canvas.getBoundingClientRect()
      return [e.clientX - r.left, e.clientY - r.top]
    }
    const onPointerDown = (e: PointerEvent) => {
      const s = useEditor.getState()
      if (e.button === 1 || (e.button === 0 && (space || s.tool === 'hand'))) {
        pan = { id: e.pointerId, x: e.clientX, y: e.clientY }
        canvas.setPointerCapture(e.pointerId)
        e.preventDefault()
      }
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
      const [sx, sy] = local(e)
      s.setCursor(screenToCell(s.view, s.layers.height, sx, sy))
    }
    const onPointerUp = (e: PointerEvent) => {
      if (pan && e.pointerId === pan.id) pan = null
    }
    const onPointerLeave = () => useEditor.getState().setCursor(null)
    let wheel = 0
    const onWheel = (e: WheelEvent) => {
      e.preventDefault() // the page itself never scrolls or zooms over the canvas
      wheel += e.deltaMode === WheelEvent.DOM_DELTA_PIXEL ? e.deltaY : e.deltaY * 40
      const s = useEditor.getState()
      while (Math.abs(wheel) >= WHEEL_STEP) {
        const dir = wheel < 0 ? 1 : -1
        wheel -= -dir * WHEEL_STEP
        s.zoomStep(dir, local(e))
      }
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.code === 'Space' && !(e.target instanceof HTMLInputElement)) {
        space = e.type === 'keydown'
        canvas.style.cursor = space || useEditor.getState().tool === 'hand' ? 'grab' : 'crosshair'
      }
    }
    canvas.addEventListener('pointerdown', onPointerDown)
    canvas.addEventListener('pointermove', onPointerMove)
    canvas.addEventListener('pointerup', onPointerUp)
    canvas.addEventListener('pointercancel', onPointerUp)
    canvas.addEventListener('pointerleave', onPointerLeave)
    // Not passive: preventDefault() must be allowed to stop the browser's own scrolling.
    canvas.addEventListener('wheel', onWheel, { passive: false })
    window.addEventListener('keydown', onKey)
    window.addEventListener('keyup', onKey)

    return () => {
      unsubscribe()
      observer.disconnect()
      cancelAnimationFrame(frame)
      canvas.removeEventListener('pointerdown', onPointerDown)
      canvas.removeEventListener('pointermove', onPointerMove)
      canvas.removeEventListener('pointerup', onPointerUp)
      canvas.removeEventListener('pointercancel', onPointerUp)
      canvas.removeEventListener('pointerleave', onPointerLeave)
      canvas.removeEventListener('wheel', onWheel)
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('keyup', onKey)
      renderer.dispose()
    }
  }, [])

  const tool = useEditor((s) => s.tool)
  return (
    <div ref={containerRef} className="relative min-h-0 min-w-0 flex-1 overflow-hidden">
      <canvas
        ref={canvasRef}
        data-testid="editor-canvas"
        className="absolute inset-0 size-full touch-none"
        style={{ cursor: tool === 'hand' ? 'grab' : 'crosshair' }}
      />
      {error && (
        <div className="absolute inset-0 grid place-items-center p-8 text-center text-sm text-red-400">
          {error}
        </div>
      )}
    </div>
  )
}
