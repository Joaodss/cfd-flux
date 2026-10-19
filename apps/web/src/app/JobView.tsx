// The job view (`#/jobs/{id}`): status, progress, pause/resume/cancel and the run's log, from the
// job's WebSocket stream. The Phase 5 viewer (fields, playback) will take the empty area.
import { ArrowLeft, Pause, Play, Square } from 'lucide-react'
import { useEffect, useReducer, useState } from 'react'
import { ApiError } from '../api/client.ts'
import { ACTIVE, followJob, jobs } from '../api/jobs.ts'
import { INITIAL, reduce } from './jobState.ts'
import { Button } from '../components/ui/button.tsx'
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

  useEffect(() => {
    // Subscribing to an external system (the server's stream): the effect's cleanup closes it
    // when the id changes or the view goes away.
    dispatch({ kind: 'reset' })
    return followJob(
      id,
      (m) => dispatch({ kind: 'message', m }),
      (normal) => dispatch({ kind: 'closed', normal }),
    )
  }, [id, attempt])

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
      <main className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-4">
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
                style={{ width: `${progress?.percent ?? (status === 'completed' ? 100 : 0)}%` }}
              />
            </div>
            <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
              <Stat
                label="Progress"
                testId="job-percent"
                value={`${(progress?.percent ?? (status === 'completed' ? 100 : 0)).toFixed(1)} %`}
              />
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
              <Stat label="Mass drift" value={progress ? formatValue(progress.massDrift) : '—'} />
              <Stat label="Wall time" value={elapsed !== null ? formatDuration(elapsed) : '—'} />
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
            <p className="mt-auto text-xs text-muted-foreground">
              The field viewer (colour maps, playback) arrives with Phase 5; the results are stored
              on the server ({`results/${job.id}`}).
            </p>
          </>
        )}
      </main>
    </div>
  )
}
