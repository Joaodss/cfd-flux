// The list of runs (`#/jobs`), newest first; a click opens a run's view.
import { ArrowLeft, Trash2 } from 'lucide-react'
import { useEffect, useState } from 'react'
import { ACTIVE, jobs, type Job } from '../api/jobs.ts'
import { Button } from '../components/ui/button.tsx'
import { navigate } from './route.ts'

type Listing =
  { state: 'loading' } | { state: 'error'; message: string } | { state: 'ready'; jobs: Job[] }

function when(ms: number): string {
  return new Date(ms).toLocaleString('en-GB', { dateStyle: 'medium', timeStyle: 'short' })
}

export function JobsList() {
  const [listing, setListing] = useState<Listing>({ state: 'loading' })
  const [reload, setReload] = useState(0)
  useEffect(() => {
    let cancelled = false
    jobs.list().then(
      (list) => !cancelled && setListing({ state: 'ready', jobs: list }),
      (e: Error) => !cancelled && setListing({ state: 'error', message: e.message }),
    )
    // Refresh while runs are active (their status and progress change).
    const t = setInterval(() => setReload((n) => n + 1), 3000)
    return () => {
      cancelled = true
      clearInterval(t)
    }
  }, [reload])

  return (
    <div className="flex h-full flex-col" data-testid="jobs-list">
      <header className="flex h-11 shrink-0 items-center gap-2 border-b border-border px-3">
        <Button variant="ghost" size="sm" onClick={() => navigate({ name: 'editor' })}>
          <ArrowLeft /> Editor
        </Button>
        <span className="ml-3 font-medium">Runs</span>
      </header>
      <main className="min-h-0 flex-1 overflow-y-auto p-4">
        {listing.state === 'loading' && <p className="text-sm text-muted-foreground">Loading…</p>}
        {listing.state === 'error' && <p className="text-sm text-red-400">{listing.message}</p>}
        {listing.state === 'ready' && listing.jobs.length === 0 && (
          <p className="text-sm text-muted-foreground">No runs yet: draw a scene and press Run.</p>
        )}
        {listing.state === 'ready' && listing.jobs.length > 0 && (
          <table className="w-full text-left text-sm">
            <thead className="text-xs text-muted-foreground">
              <tr>
                <th className="py-1 font-normal">Scene</th>
                <th className="font-normal">Status</th>
                <th className="font-normal">Engine</th>
                <th className="font-normal">Progress</th>
                <th className="font-normal">Started</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {listing.jobs.map((j) => (
                <tr
                  key={j.id}
                  className="cursor-pointer border-t border-border hover:bg-accent/50"
                  onClick={() => navigate({ name: 'job', id: j.id })}
                >
                  <td className="py-1.5">{j.scene}</td>
                  <td>
                    {j.status}
                    {j.reason ? ` (${j.reason})` : ''}
                  </td>
                  <td>{j.backend}</td>
                  <td className="tabular-nums">
                    {j.stepsTotal ? `${Math.round((100 * j.stepsDone) / j.stepsTotal)} %` : '—'}
                  </td>
                  <td className="text-xs text-muted-foreground">{when(j.createdAt)}</td>
                  <td className="text-right">
                    {!ACTIVE.includes(j.status) && (
                      <button
                        type="button"
                        aria-label={`Delete the run of ${j.scene}`}
                        title="Delete this run and its results"
                        className="grid size-7 place-items-center rounded-md text-muted-foreground hover:bg-accent hover:text-red-400 [&_svg]:size-4"
                        onClick={async (e) => {
                          e.stopPropagation() // the row's click opens the run
                          if (!window.confirm(`Delete the run of “${j.scene}” and its results?`))
                            return
                          await jobs.purge(j.id).catch(() => {})
                          setReload((n) => n + 1)
                        }}
                      >
                        <Trash2 />
                      </button>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </main>
    </div>
  )
}
