// The job view (`#/jobs/{id}`): the field viewer (Phase 5) fed by the job's WebSocket stream,
// and beside it the tabs Plots (the run's series), Statistics and Run (status, progress, log).
import { ArrowLeft, Pause, Play, Square } from 'lucide-react'
import { useCallback, useEffect, useReducer, useRef, useState } from 'react'
import { ApiError } from '../api/client.ts'
import { ACTIVE, followJob, jobs, VIEWER_MAX_SIZE } from '../api/jobs.ts'
import type { Scene } from '../generated/scene.ts'
import { seedFromHello } from '../viewer/ranges.ts'
import { useViewer } from '../viewer/store.ts'
import type { Viewer } from '../viewer/Viewer.ts'
import { fromScene } from '../scene/io.ts'
import { bodies } from '../viewer/elements.ts'
import { PlotsPanel } from '../viewer/PlotsPanel.tsx'
import { StatsPanel } from '../viewer/StatsPanel.tsx'
import { SeriesFeed, useSeries } from '../viewer/seriesFeed.ts'
import { TimelineBar } from '../viewer/TimelineBar.tsx'
import { ViewerCanvas } from '../viewer/ViewerCanvas.tsx'
import { Legend, ViewerToolbar } from '../viewer/ViewerControls.tsx'
import { INITIAL, reduce } from './jobState.ts'
import { Button } from '../components/ui/button.tsx'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '../components/ui/tabs.tsx'
import { formatBytes, formatDuration, formatValue } from '../panels/format.ts'
import { navigate } from './route.ts'

/** The current time, updated every second while `ticking` (state, so a render never reads a clock). */
function useNow(ticking: boolean): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (!ticking) return
    const t = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(t)
  }, [ticking])
  return now
}

const STATUS_STYLE: Record<string, string> = {
  queued: 'bg-muted text-muted-foreground',
  running: 'bg-primary/15 text-primary',
  paused: 'bg-amber-500/15 text-amber-500',
  completed: 'bg-green-500/15 text-green-500',
  failed: 'bg-red-500/15 text-red-400',
  cancelled: 'bg-muted text-muted-foreground',
}

function Stat(props: { label: string; value: string; testId?: string }) {
  return (
    <div className="flex flex-col rounded-md border border-border px-3 py-2">
      <span className="text-[11px] text-muted-foreground">{props.label}</span>
      <span className="text-sm tabular-nums" data-testid={props.testId}>
        {props.value}
      </span>
    </div>
  )
}

export function JobView({ id }: { id: string }) {
  const [state, dispatch] = useReducer(reduce, INITIAL)
  const [attempt, setAttempt] = useState(0)
  const [error, setError] = useState<string | null>(null)
  // The scene with the job it belongs to, so a scene of the previous job is never used.
  const [loaded, setLoaded] = useState<{ id: string; scene: Scene } | null>(null)
  const scene = loaded?.id === id ? loaded.scene : null
  const [viewer, setViewer] = useState<Viewer | null>(null)
  // The latest viewer for the stream's callbacks (a ref: they must not restart the stream).
  const viewerRef = useRef<Viewer | null>(null)
  const onViewer = useCallback((v: Viewer | null) => {
    viewerRef.current = v
    setViewer(v)
  }, [])

  useEffect(() => {
    // Subscribing to an external system (the server's stream): the effect's cleanup closes it
    // when the id changes or the view goes away.
    dispatch({ kind: 'reset' })
    useViewer.getState().reset()
    const series = useSeries.getState()
    const feed = new SeriesFeed(
      { meta: () => jobs.meta(id), text: (name) => jobs.text(id, name) },
      series.bump,
    )
    series.setFeed(feed)
    const close = followJob(
      id,
      (m) => {
        if (m.type === 'hello') useViewer.getState().seed(seedFromHello(m.ranges))
        feed.onMessage(m)
        // Rows go to the feed only: they would re-render the whole view ten times a second.
        if (m.type !== 'series') dispatch({ kind: 'message', m })
      },
      (normal) => dispatch({ kind: 'closed', normal }),
      (frame) => viewerRef.current?.pushFrame(frame),
    )
    return () => {
      close()
      feed.dispose()
      useSeries.getState().setFeed(null)
    }
  }, [id, attempt])

  // The run's own scene: its cells are the viewer's solid mask.
  useEffect(() => {
    let cancelled = false
    jobs.scene(id).then(
      (s) => !cancelled && setLoaded({ id, scene: s }),
      (e: ApiError) => !cancelled && setError(`The run's scene cannot be read: ${e.message}`),
    )
    return () => {
      cancelled = true
    }
  }, [id])

  // The bodies in the flow (the default element of the force plot), from the scene's cells.
  useEffect(() => {
    if (!scene) return
    let cancelled = false
    fromScene(scene).then(
      (doc) => {
        const { elementId, width, height } = doc.layers
        if (!cancelled)
          useSeries.getState().setBodies(bodies(scene.elements, elementId, width, height))
      },
      () => {}, // the viewer reports a scene it cannot read
    )
    return () => {
      cancelled = true
    }
  }, [scene])

  useEffect(() => {
    if (viewer && scene) {
      viewer
        .setScene(scene, id, VIEWER_MAX_SIZE)
        .catch((e: Error) => setError(`The run's scene cannot be shown: ${e.message}`))
    }
  }, [viewer, scene, id])

  // The timeline follows the run while it is active; once it is over, its frames are known.
  const jobStatus = state.job?.status
  const jobFrames = state.job?.frames
  useEffect(() => {
    if (!viewer || !jobStatus) return
    const active = ACTIVE.includes(jobStatus)
    viewer.setRun(active, active ? null : (jobFrames ?? null))
  }, [viewer, jobStatus, jobFrames])

  const { job, progress, connection } = state
  const status = job?.status
  const act = async (f: (id: string) => Promise<unknown>) => {
    setError(null)
    try {
      await f(id)
    } catch (e) {
      setError((e as ApiError).message)
    }
  }
  // A completed run is at 100 % (the last progress message may predate the final steps).
  const percent = status === 'completed' ? 100 : (progress?.percent ?? 0)
  const now = useNow(status !== undefined && ACTIVE.includes(status))
  const elapsed =
    job?.startedAt !== undefined ? ((job.finishedAt ?? now) - job.startedAt) / 1000 : null

  return (
    <div className="flex h-full flex-col" data-testid="job-view">
      <header className="flex h-11 shrink-0 items-center gap-2 border-b border-border px-3">
        <Button
          variant="ghost"
          size="sm"
          onClick={() => navigate({ name: 'editor' })}
          data-testid="back-to-editor"
        >
          <ArrowLeft /> Editor
        </Button>
        <Button variant="ghost" size="sm" onClick={() => navigate({ name: 'jobs' })}>
          All runs
        </Button>
        <span className="ml-3 truncate font-medium">{job?.scene ?? 'Run'}</span>
        {status && (
          <span
            className={`rounded-full px-2 py-0.5 text-xs ${STATUS_STYLE[status]}`}
            data-testid="job-status"
          >
            {status}
            {job?.reason ? ` (${job.reason})` : ''}
          </span>
        )}
        <span className="ml-auto flex gap-1">
          {status === 'running' && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => act(jobs.pause)}
              data-testid="job-pause"
            >
              <Pause /> Pause
            </Button>
          )}
          {status === 'paused' && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => act(jobs.resume)}
              data-testid="job-resume"
            >
              <Play /> Resume
            </Button>
          )}
          {status && ACTIVE.includes(status) && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => act(jobs.cancel)}
              data-testid="job-cancel"
            >
              <Square /> Cancel
            </Button>
          )}
        </span>
      </header>
      <div className="flex min-h-0 flex-1">
        <section className="flex min-w-0 flex-1 flex-col" data-testid="viewer">
          <ViewerToolbar viewer={viewer} />
          <ViewerCanvas onReady={onViewer} />
          <TimelineBar
            viewer={viewer}
            running={state.job ? ACTIVE.includes(state.job.status) : true}
          />
          <Legend />
        </section>
        <aside className="flex w-96 shrink-0 flex-col overflow-y-auto border-l border-border p-4">
          <Tabs defaultValue="run">
            <TabsList>
              <TabsTrigger value="plots" data-testid="job-tab-plots">
                Plots
              </TabsTrigger>
              <TabsTrigger value="stats" data-testid="job-tab-stats">
                Statistics
              </TabsTrigger>
              <TabsTrigger value="run" data-testid="job-tab-run">
                Run
              </TabsTrigger>
            </TabsList>
            <TabsContent value="plots">
              <PlotsPanel viewer={viewer} />
            </TabsContent>
            <TabsContent value="stats">
              <StatsPanel sceneName={job?.scene} />
            </TabsContent>
            <TabsContent value="run">
              {error && <p className="text-sm text-red-400">{error}</p>}
              {connection === 'connecting' && (
                <p className="text-sm text-muted-foreground">Connecting…</p>
              )}
              {connection === 'lost' && (
                <p className="text-sm text-amber-500">
                  The connection to the server was lost.{' '}
                  <button className="underline" onClick={() => setAttempt((n) => n + 1)}>
                    Reconnect
                  </button>
                </p>
              )}
              {job?.message && (
                <p
                  className={
                    status === 'failed' ? 'text-sm text-red-400' : 'text-sm text-muted-foreground'
                  }
                >
                  {job.message}
                </p>
              )}
              {job && (
                <>
                  <div className="h-2 w-full overflow-hidden rounded-full bg-muted">
                    <div
                      className="h-full bg-primary transition-[width]"
                      style={{ width: `${percent}%` }}
                    />
                  </div>
                  <div className="grid grid-cols-2 gap-2">
                    <Stat label="Progress" testId="job-percent" value={`${percent.toFixed(1)} %`} />
                    <Stat
                      label="Step"
                      value={`${(progress?.step ?? job.stepsDone).toLocaleString('en')} / ${job.stepsTotal.toLocaleString('en')}`}
                    />
                    <Stat
                      label="Simulated time"
                      value={progress ? `${formatValue(progress.simTime)} s` : '—'}
                    />
                    {/* Progress messages carry no frame count: it is known once the run is over. */}
                    {status && !ACTIVE.includes(status) && (
                      <Stat label="Frames" value={job.frames.toLocaleString('en')} />
                    )}
                    <Stat
                      label="Engine"
                      value={`${job.backend}${job.device ? ` · ${job.device}` : ''}`}
                    />
                    <Stat
                      label="Throughput"
                      value={progress?.mlups ? `${formatValue(progress.mlups)} MLUPS` : '—'}
                    />
                    <Stat
                      label="Max Mach (lattice)"
                      value={progress ? formatValue(progress.maxMach) : '—'}
                    />
                    <Stat
                      label="Mass drift"
                      value={progress ? formatValue(progress.massDrift) : '—'}
                    />
                    <Stat
                      label="Wall time"
                      value={elapsed !== null ? formatDuration(elapsed) : '—'}
                    />
                    {job.estimate && (
                      <Stat label="Device memory" value={formatBytes(job.estimate.memoryBytes)} />
                    )}
                  </div>
                  {job.warnings?.length || job.notes?.length || state.log.length ? (
                    <section className="text-xs">
                      <h3 className="mb-1 font-medium tracking-wide text-muted-foreground uppercase">
                        Log
                      </h3>
                      <ul className="flex flex-col gap-0.5 font-mono">
                        {job.notes?.map((n, i) => (
                          <li key={`n${i}`}>{n}</li>
                        ))}
                        {job.warnings?.map((w, i) => (
                          <li key={`w${i}`} className="text-amber-500">
                            {w.message}
                          </li>
                        ))}
                        {state.log.map((l, i) => (
                          <li key={`l${i}`}>{l}</li>
                        ))}
                      </ul>
                    </section>
                  ) : null}
                  <p className="text-xs text-muted-foreground">
                    Results on the server: {`results/${job.id}`}
                  </p>
                </>
              )}
            </TabsContent>
          </Tabs>
        </aside>
      </div>
    </div>
  )
}
