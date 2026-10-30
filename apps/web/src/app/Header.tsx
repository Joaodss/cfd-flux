// Top bar: document commands (new, open, save, examples, PNG import) and the document's name.
import {
  ChevronDown,
  FilePlus,
  FolderOpen,
  History,
  ImageUp,
  LayoutGrid,
  Play,
  Save,
} from 'lucide-react'
import { lazy, Suspense, useState } from 'react'
import { Button } from '../components/ui/button.tsx'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '../components/ui/dropdown-menu.tsx'
import { isDirty, useEditor } from '../editor/store.ts'
import { newScene, openFile, save, saveAsFile, saveToServer } from './documents.ts'
import { IssuesButton } from './IssuesButton.tsx'
import { navigate } from './route.ts'
import { mark } from '../perf.ts'
import { runScene } from './run.ts'

// The dialogs load when first opened, each a chunk of its own: the gallery carries every example
// scene of `scenes/` (it grows with each example), the image import its mask code. This keeps the
// main chunk under Vite's 500 kB warning (docs/development.md, "Bundle size").
const ExampleGallery = lazy(() =>
  import('./ExampleGallery.tsx').then((m) => ({ default: m.ExampleGallery })),
)
const ImportImageDialog = lazy(() =>
  import('./ImportImageDialog.tsx').then((m) => ({ default: m.ImportImageDialog })),
)
const ServerScenesDialog = lazy(() =>
  import('./ServerScenesDialog.tsx').then((m) => ({ default: m.ServerScenesDialog })),
)

type DialogName = 'server' | 'gallery' | 'image'

function sourceLabel(s: ReturnType<typeof useEditor.getState>['source']): string {
  if (!s) return 'not saved'
  if (s.kind === 'file') return s.name
  if (s.kind === 'server') return 'on the server'
  return 'example'
}

export function Header() {
  const name = useEditor((s) => s.table.name)
  const source = useEditor((s) => s.source)
  const dirty = useEditor(isDirty)
  const runnable = useEditor((s) => s.validation.status === 'done' && s.validation.result.ok)
  const [running, setRunning] = useState(false)
  const [dialog, setDialog] = useState<DialogName | null>(null)
  // The dialogs opened so far: mounted from then on, so they keep their state between opens.
  const [opened, setOpened] = useState<DialogName[]>([])
  const open = (d: DialogName) => {
    setDialog(d)
    setOpened((o) => (o.includes(d) ? o : [...o, d]))
  }
  const close = (isOpen: boolean) => !isOpen && setDialog(null)

  return (
    <header className="flex h-11 shrink-0 items-center gap-1 border-b border-border px-3">
      <span className="mr-3 font-semibold tracking-tight">cfd-flux</span>
      <Button variant="ghost" size="sm" onClick={newScene} title="New scene">
        <FilePlus /> New
      </Button>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="sm" data-testid="menu-open">
            <FolderOpen /> Open <ChevronDown />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start">
          <DropdownMenuItem onSelect={() => void openFile()}>File… (Ctrl+O)</DropdownMenuItem>
          <DropdownMenuItem onSelect={() => open('server')} data-testid="open-server">
            From the server…
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="sm" data-testid="menu-save">
            <Save /> Save <ChevronDown />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start">
          <DropdownMenuItem onSelect={() => void save()}>Save (Ctrl+S)</DropdownMenuItem>
          <DropdownMenuItem onSelect={() => void saveAsFile()}>
            Save as a file… (Ctrl+Shift+S)
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => void saveToServer(true)} data-testid="save-server">
            Save to the server as new
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <Button variant="ghost" size="sm" onClick={() => open('gallery')} data-testid="open-gallery">
        <LayoutGrid /> Examples
      </Button>
      <Button
        variant="ghost"
        size="sm"
        onClick={() => open('image')}
        title="Import a PNG as a mask"
      >
        <ImageUp /> Import image
      </Button>
      <span className="ml-4 min-w-0 truncate text-sm" data-testid="scene-name">
        {name}
        {dirty && <span title="Unsaved changes"> •</span>}
      </span>
      <span className="truncate text-xs text-muted-foreground">— {sourceLabel(source)}</span>
      <IssuesButton />
      <Button
        variant="ghost"
        size="sm"
        onClick={() => navigate({ name: 'jobs' })}
        title="Earlier runs"
      >
        <History /> Runs
      </Button>
      <Button
        size="sm"
        data-testid="run"
        disabled={!runnable || running}
        title={
          runnable
            ? 'Run the simulation on the server'
            : 'Fix the issues first (or start the server)'
        }
        onClick={async () => {
          mark('run:click')
          setRunning(true)
          await runScene()
          setRunning(false)
        }}
      >
        <Play /> Run
      </Button>
      <Suspense fallback={null}>
        {opened.includes('server') && (
          <ServerScenesDialog open={dialog === 'server'} onOpenChange={close} />
        )}
        {opened.includes('gallery') && (
          <ExampleGallery open={dialog === 'gallery'} onOpenChange={close} />
        )}
        {opened.includes('image') && (
          <ImportImageDialog open={dialog === 'image'} onOpenChange={close} />
        )}
      </Suspense>
    </header>
  )
}
