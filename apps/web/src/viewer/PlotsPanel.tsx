// The Plots tab: the run's series drawn with uPlot (design proposal §7.1), live while the run
// goes on. A vertical line marks the playhead's time; a click on a plot moves the playhead to
// the frame closest to that time. The statistics window is shaded; dragging across a plot sets it.
//
// uPlot is not a React component: it owns a <div> and draws into its own canvas. So each chart
// is created once in an effect (and destroyed in its cleanup), and later renders only hand it new
// data — the pattern for wrapping any imperative widget in React, like hosting a WinForms control
// in WPF.
import { useEffect, useMemo, useRef } from 'react'
import uPlot from 'uplot'
import 'uplot/dist/uPlot.min.css'
import { diagnosticPlots, forcePlots, frameAt, probePlots, type Plot } from './plots.ts'
import { useElement, useMeta, useSeries, useWindow } from './seriesFeed.ts'
import type { Window } from './statistics.ts'
import { useViewer } from './store.ts'
import type { Viewer } from './Viewer.ts'

const COLOURS = ['#3b82f6', '#f59e0b', '#10b981', '#ef4444', '#a855f7', '#14b8a6']
const HEIGHT = 150

/** The plots of the current series (recomputed when the feed changes). */
function usePlots(element: string | null): Plot[] {
  const feed = useSeries((s) => s.feed)
  const version = useSeries((s) => s.version)
  const meta = useMeta() // the reference scales in use (the session may override them)
  return useMemo(() => {
    void version // the buffer is mutable: `version` says when to look again
    if (!feed) return []
    const b = feed.buffer
    return [
      ...forcePlots(b.table('forces'), meta, element),
      ...probePlots(b.table('probes')),
      ...diagnosticPlots(b.table('diagnostics')),
    ]
  }, [feed, version, meta, element])
}

const SELECT = 'h-7 rounded-md border border-border bg-background px-1.5 text-xs'

/** The element whose forces are plotted (and whose statistics are computed). */
export function ElementSelect() {
  const { element, available } = useElement()
  if (available.length < 2 || !element) return null
  return (
    <label className="flex items-center gap-2 text-xs text-muted-foreground">
      Forces on
      <select
        className={SELECT}
        data-testid="plots-element"
        value={element}
        onChange={(e) => useSeries.getState().setElement(e.target.value)}
      >
        {available.map((n) => (
          <option key={n} value={n}>
            {n}
          </option>
        ))}
      </select>
    </label>
  )
}

export function PlotsPanel({ viewer }: { viewer: Viewer | null }) {
  const { element } = useElement()
  const plots = usePlots(element)
  const meta = useMeta()
  const playhead = useViewer((s) => s.frame?.header.simTime ?? null)
  const last = useViewer((s) => s.timeline.last)
  const { window } = useWindow()
  const seek = (t: number) => {
    if (viewer && meta && last >= 0) viewer.seek(frameAt(t, meta, last))
  }
  const setWindow = (w: Window) => useSeries.getState().setWindow(w)
  if (plots.length === 0) {
    return <p className="text-sm text-muted-foreground">No series yet.</p>
  }
  return (
    <div className="flex flex-col gap-3" data-testid="plots">
      <ElementSelect />
      {plots.map((p) => (
        <Chart
          key={p.id}
          plot={p}
          playhead={playhead}
          window={window}
          onSeek={seek}
          onWindow={setWindow}
        />
      ))}
    </div>
  )
}

/** A theme colour of index.css (light or dark), for the canvas. */
function cssVar(name: string, fallback: string): string {
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim()
  return v || fallback
}

function chartData(p: Plot): uPlot.AlignedData {
  return [p.time as number[], ...p.lines.map((l) => l.values)]
}

interface ChartProps {
  plot: Plot
  playhead: number | null
  window: Window | null
  onSeek: (t: number) => void
  onWindow: (w: Window) => void
}

function Chart(props: ChartProps) {
  const { plot, playhead, window, onSeek, onWindow } = props
  const host = useRef<HTMLDivElement>(null)
  const chart = useRef<uPlot | null>(null)
  // Read by uPlot's hooks and the chart's creation, which outlive any one render.
  const playheadRef = useRef(playhead)
  const windowRef = useRef(window)
  const onSeekRef = useRef(onSeek)
  const onWindowRef = useRef(onWindow)
  const plotRef = useRef(plot)
  useEffect(() => {
    onSeekRef.current = onSeek
    onWindowRef.current = onWindow
    plotRef.current = plot
  })

  const shape = `${plot.id}|${plot.lines.map((l) => l.label).join(',')}`
  useEffect(() => {
    const el = host.current
    const plot = plotRef.current
    if (!el) return
    const text = cssVar('--muted-foreground', '#9ca3af')
    const grid = cssVar('--border', '#2a2e37')
    const axis = { stroke: text, grid: { stroke: grid, width: 1 }, ticks: { stroke: grid } }
    const u = new uPlot(
      {
        width: Math.max(el.clientWidth, 100),
        height: HEIGHT,
        title: plot.title,
        scales: { x: { time: false } },
        axes: [{ ...axis }, { ...axis, size: 56 }],
        // Dragging selects a time range (the statistics window), without zooming.
        cursor: { drag: { x: true, y: false, setScale: false } },
        series: [
          { label: 't (s)' },
          ...plot.lines.map((l, i) => ({
            label: l.label,
            stroke: COLOURS[i % COLOURS.length],
            width: 1.25,
            points: { show: false },
          })),
        ],
        hooks: {
          // A drag ends with a selection: it becomes the window (shaded by `draw`, so uPlot's
          // own selection box is cleared).
          setSelect: [
            (u) => {
              const { left, width } = u.select
              if (width < 4) return
              onWindowRef.current({
                from: u.posToVal(left, 'x'),
                to: u.posToVal(left + width, 'x'),
              })
              u.setSelect({ left: 0, top: 0, width: 0, height: 0 }, false)
            },
          ],
          // The window (a band) and the playhead (a line), in canvas pixels.
          draw: [
            (u) => {
              const ctx = u.ctx
              const { left, top, width, height } = u.bbox
              const w = windowRef.current
              if (w) {
                const x0 = Math.max(left, u.valToPos(w.from, 'x', true))
                const x1 = Math.min(left + width, u.valToPos(w.to, 'x', true))
                if (x1 > x0) {
                  ctx.save()
                  ctx.fillStyle = 'rgba(59, 130, 246, 0.1)'
                  ctx.fillRect(x0, top, x1 - x0, height)
                  ctx.restore()
                }
              }
              const t = playheadRef.current
              if (t === null) return
              const x = Math.round(u.valToPos(t, 'x', true))
              if (x < left || x > left + width) return
              ctx.save()
              ctx.strokeStyle = text
              ctx.setLineDash([4, 3])
              ctx.beginPath()
              ctx.moveTo(x, u.bbox.top)
              ctx.lineTo(x, u.bbox.top + u.bbox.height)
              ctx.stroke()
              ctx.restore()
            },
          ],
        },
      },
      chartData(plot),
      el,
    )
    // A click (not the end of a drag) moves the playhead.
    let down: number | null = null
    u.over.addEventListener('mousedown', (e) => (down = e.clientX))
    u.over.addEventListener('click', (e) => {
      const left = u.cursor.left
      const dragged = down !== null && Math.abs(e.clientX - down) > 3
      down = null
      if (dragged || left === undefined || left < 0) return
      onSeekRef.current(u.posToVal(left, 'x'))
    })
    chart.current = u
    // Follow the panel's width (the tab can be resized with the window).
    const ro = new ResizeObserver(() => {
      if (el.clientWidth > 0) u.setSize({ width: el.clientWidth, height: HEIGHT })
    })
    ro.observe(el)
    return () => {
      ro.disconnect()
      u.destroy()
      chart.current = null
    }
    // Rebuilt only when the plot's lines change; new data goes through `setData` below.
  }, [shape])

  useEffect(() => {
    chart.current?.setData(chartData(plot))
  }, [plot])

  useEffect(() => {
    playheadRef.current = playhead
    windowRef.current = window
    chart.current?.redraw(false)
  }, [playhead, window])

  return <div ref={host} className="w-full" data-testid={`plot-${plot.id}`} />
}
