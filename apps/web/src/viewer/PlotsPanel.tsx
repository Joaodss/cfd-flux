// The Plots tab: the run's series drawn with uPlot (design proposal §7.1), live while the run
// goes on. A vertical line marks the playhead's time; a click on a plot moves the playhead to
// the frame closest to that time.
//
// uPlot is not a React component: it owns a <div> and draws into its own canvas. So each chart
// is created once in an effect (and destroyed in its cleanup), and later renders only hand it new
// data — the pattern for wrapping any imperative widget in React, like hosting a WinForms control
// in WPF.
import { useEffect, useMemo, useRef } from 'react'
import uPlot from 'uplot'
import 'uplot/dist/uPlot.min.css'
import { diagnosticPlots, forcePlots, frameAt, probePlots, type Plot } from './plots.ts'
import { useElement, useSeries } from './seriesFeed.ts'
import { useViewer } from './store.ts'
import type { Viewer } from './Viewer.ts'

const COLOURS = ['#3b82f6', '#f59e0b', '#10b981', '#ef4444', '#a855f7', '#14b8a6']
const HEIGHT = 150

/** The plots of the current series (recomputed when the feed changes). */
function usePlots(element: string | null): Plot[] {
  const feed = useSeries((s) => s.feed)
  const version = useSeries((s) => s.version)
  return useMemo(() => {
    void version // the buffer is mutable: `version` says when to look again
    if (!feed) return []
    const b = feed.buffer
    return [
      ...forcePlots(b.table('forces'), feed.meta, element),
      ...probePlots(b.table('probes')),
      ...diagnosticPlots(b.table('diagnostics')),
    ]
  }, [feed, version, element])
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
  const meta = useSeries((s) => s.feed?.meta ?? null)
  const playhead = useViewer((s) => s.frame?.header.simTime ?? null)
  const last = useViewer((s) => s.timeline.last)
  const seek = (t: number) => {
    if (viewer && meta && last >= 0) viewer.seek(frameAt(t, meta, last))
  }
  if (plots.length === 0) {
    return <p className="text-sm text-muted-foreground">No series yet.</p>
  }
  return (
    <div className="flex flex-col gap-3" data-testid="plots">
      <ElementSelect />
      {plots.map((p) => (
        <Chart key={p.id} plot={p} playhead={playhead} onSeek={seek} />
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

function Chart(props: { plot: Plot; playhead: number | null; onSeek: (t: number) => void }) {
  const { plot, playhead, onSeek } = props
  const host = useRef<HTMLDivElement>(null)
  const chart = useRef<uPlot | null>(null)
  // Read by uPlot's hooks and the chart's creation, which outlive any one render.
  const playheadRef = useRef(playhead)
  const onSeekRef = useRef(onSeek)
  const plotRef = useRef(plot)
  useEffect(() => {
    onSeekRef.current = onSeek
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
        cursor: { drag: { x: false, y: false } },
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
          // The playhead: a line across the plot area (canvas pixels).
          draw: [
            (u) => {
              const t = playheadRef.current
              if (t === null) return
              const x = Math.round(u.valToPos(t, 'x', true))
              if (x < u.bbox.left || x > u.bbox.left + u.bbox.width) return
              const ctx = u.ctx
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
    u.over.addEventListener('click', () => {
      const left = u.cursor.left
      if (left === undefined || left < 0) return
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
    chart.current?.redraw(false)
  }, [playhead])

  return <div ref={host} className="w-full" data-testid={`plot-${plot.id}`} />
}
