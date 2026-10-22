// Feeds the series buffer from the job's stream and files: the columns from `hello`, the rows
// of every `series` message, and the CSV files and `meta.json` once the job has started (read
// again after a `resync`). The Plots and Statistics tabs subscribe to `useSeries`.
import { useMemo } from 'react'
import { create } from 'zustand'
import type { RunMeta, StreamMessage } from '../api/jobs.ts'
import { shownElement } from './elements.ts'
import { coefficients, forceElements } from './plots.ts'
import {
  defaultWindow,
  forceStatistics,
  wallNusselt,
  type ForceStatistics,
  type WallNusselt,
  type Window,
} from './statistics.ts'
import { SERIES, SeriesBuffer, type SeriesName } from './series.ts'

export interface SeriesSource {
  meta(): Promise<RunMeta>
  /** A file of the run as text (`forces.csv`, …). */
  text(name: string): Promise<string>
}

export class SeriesFeed {
  buffer = new SeriesBuffer()
  meta: RunMeta | null = null
  private loading = false
  private started = false
  private disposed = false
  private readonly source: SeriesSource
  private readonly onChange: () => void

  constructor(source: SeriesSource, onChange: () => void) {
    this.source = source
    this.onChange = onChange
  }

  onMessage(m: StreamMessage): void {
    switch (m.type) {
      case 'hello':
        if (m.series) this.buffer.setColumns(m.series)
        this.started = m.job.status !== 'queued'
        break
      case 'status':
        this.started ||= m.status !== 'queued'
        break
      case 'series':
        this.started = true
        if (m.resync) this.buffer.forgetFiles()
        this.buffer.append(m.rows)
        break
      case 'done':
        this.started = true
        break
      default:
        return
    }
    this.onChange()
    void this.load()
  }

  /** Reads what is still missing (files, meta), one round at a time; failures retry later. */
  private async load(): Promise<void> {
    if (this.loading || !this.started) return
    this.loading = true
    for (;;) {
      const missing = this.missingFiles()
      if (missing.length === 0 && this.meta) break
      const hadMeta = this.meta !== null
      await Promise.all([
        this.meta
          ? null
          : this.source.meta().then(
              (m) => (this.meta = m),
              () => {},
            ),
        ...missing.map((n) =>
          this.source.text(`${n}.csv`).then(
            (t) => !this.disposed && this.buffer.setFile(n, t),
            () => {},
          ),
        ),
      ])
      // `meta.json` says whether there is a `heat.csv`: read it in a second round if so.
      if (hadMeta || !this.meta?.run.heat) break
    }
    this.loading = false
    if (!this.disposed) this.onChange()
  }

  /** The series files still to read; `heat.csv` only when `meta.json` says the run has one. */
  private missingFiles(): SeriesName[] {
    return SERIES.filter(
      (n) => this.buffer.needsFile(n) && (n !== 'heat' || this.meta?.run.heat === true),
    )
  }

  dispose(): void {
    this.disposed = true
  }
}

interface SeriesState {
  feed: SeriesFeed | null
  /** Bumped when the feed's data changes (the data itself is not React state). */
  version: number
  /** Force columns of the run's bodies (`elements.ts`), once its scene is known. */
  bodies: string[]
  /** The element chosen for the force plot and statistics (`null`: the first body). */
  element: string | null
  /** The statistics window dragged on a plot (`null`: the second half of the run so far). */
  window: Window | null
  setFeed(feed: SeriesFeed | null): void
  bump(): void
  setBodies(bodies: string[]): void
  setElement(element: string): void
  setWindow(window: Window | null): void
}

export const useSeries = create<SeriesState>()((set) => ({
  feed: null,
  version: 0,
  bodies: [],
  element: null,
  window: null,
  setFeed: (feed) =>
    set((s) => ({ feed, version: s.version + 1, bodies: [], element: null, window: null })),
  bump: () => set((s) => ({ version: s.version + 1 })),
  setBodies: (bodies) => set({ bodies }),
  setElement: (element) => set({ element }),
  setWindow: (window) => set({ window }),
}))

/** The statistics window: the one dragged, else the second half of the forces' time so far. */
export function useWindow(): { window: Window | null; chosen: boolean } {
  const feed = useSeries((s) => s.feed)
  const version = useSeries((s) => s.version)
  const chosen = useSeries((s) => s.window)
  return useMemo(() => {
    void version
    if (chosen) return { window: chosen, chosen: true }
    const time = feed?.buffer.table('forces')?.data[1] ?? []
    return { window: defaultWindow(time), chosen: false }
  }, [feed, version, chosen])
}

/** The force element shown (chosen, else the first body, else the first) and the choices. */
export function useElement(): { element: string | null; available: string[] } {
  const feed = useSeries((s) => s.feed)
  const version = useSeries((s) => s.version)
  const bodies = useSeries((s) => s.bodies)
  const chosen = useSeries((s) => s.element)
  return useMemo(() => {
    void version
    const forces = feed?.buffer.table('forces')
    const available = forces ? forceElements(forces) : []
    return { element: shownElement(available, bodies, chosen), available }
  }, [feed, version, bodies, chosen])
}

/** The force statistics of the shown element over the window, or null. */
export function useStatistics(): ForceStatistics | null {
  const feed = useSeries((s) => s.feed)
  const version = useSeries((s) => s.version)
  const { element } = useElement()
  const { window } = useWindow()
  return useMemo(() => {
    void version
    const forces = feed?.buffer.table('forces')
    const meta = feed?.meta
    if (!forces || !meta || !element || !window) return null
    const c = coefficients(forces, element, meta)
    return c && forceStatistics(forces.data[1], c.cd, c.cl, window, meta)
  }, [feed, version, element, window])
}

/** The Nusselt number of every thermal wall element over the window (thermal runs with `heat.csv`). */
export function useNusselt(): WallNusselt[] {
  const feed = useSeries((s) => s.feed)
  const version = useSeries((s) => s.version)
  const { window } = useWindow()
  return useMemo(() => {
    void version
    const heat = feed?.buffer.table('heat')
    const meta = feed?.meta
    const reference = meta?.dimensionless.reference
    if (!heat || !reference?.conductivity || !window) return []
    return wallNusselt(heat.columns, heat.data, window, {
      length: reference.length,
      conductivity: reference.conductivity,
      deltaT: reference.temperatureDifference,
    })
  }, [feed, version, window])
}
