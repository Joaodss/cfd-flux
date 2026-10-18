// "Open from server": the scenes saved on cfd-server, newest first.
import { Trash2 } from 'lucide-react'
import { useEffect, useState } from 'react'
import { api, type SceneSummary } from '../api/client.ts'
import { Dialog, DialogContent } from '../components/ui/dialog.tsx'
import { openServerScene } from './documents.ts'

type Listing =
  | { state: 'loading' }
  | { state: 'error'; message: string }
  | { state: 'ready'; scenes: SceneSummary[] }

function when(ms: number): string {
  return new Date(ms).toLocaleString('en-GB', { dateStyle: 'medium', timeStyle: 'short' })
}

/** The list, fetched when the dialog mounts (the parent mounts it only while open). */
function SceneList({ onOpened }: { onOpened: () => void }) {
  const [listing, setListing] = useState<Listing>({ state: 'loading' })
  const [reload, setReload] = useState(0)
  useEffect(() => {
    // An effect that talks to an external system (the server); `cancelled` ignores a reply that
    // arrives after the dialog closed or reloaded (like a CancellationToken).
    let cancelled = false
    api.listScenes().then(
      (scenes) => !cancelled && setListing({ state: 'ready', scenes }),
      (e: Error) => !cancelled && setListing({ state: 'error', message: e.message }),
    )
    return () => {
      cancelled = true
    }
  }, [reload])

  if (listing.state === 'loading') return <p className="text-sm text-muted-foreground">Loading…</p>
  if (listing.state === 'error') return <p className="text-sm text-red-400">{listing.message}</p>
  if (listing.scenes.length === 0) {
    return <p className="text-sm text-muted-foreground">No scenes saved on the server yet.</p>
  }
  return (
    <ul className="-mx-1 flex flex-col overflow-y-auto" data-testid="server-scenes">
      {listing.scenes.map((s) => (
        <li key={s.id} className="group flex items-center">
          <button
            type="button"
            className="flex min-w-0 flex-1 items-baseline gap-3 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent"
            onClick={async () => {
              if (await openServerScene(s.id, s.name)) onOpened()
            }}
          >
            <span className="truncate">{s.name}</span>
            <span className="ml-auto shrink-0 text-xs text-muted-foreground">
              {when(s.updatedAt)}
            </span>
          </button>
          <button
            type="button"
            aria-label={`Delete ${s.name}`}
            title={`Delete “${s.name}” from the server`}
            className="invisible ml-1 grid size-7 place-items-center rounded-md text-muted-foreground group-hover:visible hover:bg-accent hover:text-red-400 [&_svg]:size-4"
            onClick={async () => {
              if (!window.confirm(`Delete “${s.name}” from the server?`)) return
              await api.deleteScene(s.id).catch(() => {})
              setReload((n) => n + 1)
            }}
          >
            <Trash2 />
          </button>
        </li>
      ))}
    </ul>
  )
}

export function ServerScenesDialog(props: {
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange}>
      <DialogContent title="Open from the server" description="Scenes saved on cfd-server">
        {props.open && <SceneList onOpened={() => props.onOpenChange(false)} />}
      </DialogContent>
    </Dialog>
  )
}
