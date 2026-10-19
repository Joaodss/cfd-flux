// The editor's layout: header, notices, tool options, tool strip, canvas, side panel, status bar.
import { useEffect, useState } from 'react'
import { discardDraft, pendingDraft, startAutosave } from './app/autosave.ts'
import { openDroppedFile, openFile, save, saveAsFile } from './app/documents.ts'
import { Header } from './app/Header.tsx'
import { JobsList } from './app/JobsList.tsx'
import { JobView } from './app/JobView.tsx'
import { parseRoute, useRoute } from './app/route.ts'
import { ToolOptions } from './app/ToolOptions.tsx'
import { Toolbar } from './app/Toolbar.tsx'
import { startValidation } from './app/validation.ts'
import { EditorCanvas } from './editor/EditorCanvas.tsx'
import { isDirty, useEditor } from './editor/store.ts'
import { handleKey } from './editor/tools/controller.ts'
import { ScenePanel } from './panels/ScenePanel.tsx'
import { StatusBar } from './panels/StatusBar.tsx'
import type { Draft } from './scene/drafts.ts'
import { fromScene } from './scene/io.ts'

/** The editor's keyboard shortcuts (ignored while typing in a field). */
function useShortcuts() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.ctrlKey || e.metaKey
      // Document commands work everywhere, even in a field.
      if (mod && e.key.toLowerCase() === 's') {
        e.preventDefault()
        void (e.shiftKey ? saveAsFile() : save())
        return
      }
      if (mod && e.key.toLowerCase() === 'o') {
        e.preventDefault()
        void openFile()
        return
      }
      // The rest belongs to the editor, not the run views.
      if (parseRoute(window.location.hash).name !== 'editor') return
      const t = e.target
      if (t instanceof HTMLInputElement || t instanceof HTMLTextAreaElement) return
      if (t instanceof HTMLSelectElement) return
      if (t instanceof HTMLElement && t.closest('[role=menu], [role=dialog]')) return
      if (handleKey(e)) e.preventDefault()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])
}

/** Validation, autosave, the browser's "leave site?" warning, and scene files dropped on the window. */
function useDocumentGuards() {
  useEffect(() => {
    const stopAutosave = startAutosave()
    const stopValidation = startValidation()
    const beforeUnload = (e: BeforeUnloadEvent) => {
      if (isDirty(useEditor.getState())) e.preventDefault()
    }
    const dragOver = (e: DragEvent) => {
      if (e.dataTransfer?.types.includes('Files')) e.preventDefault()
    }
    const drop = (e: DragEvent) => {
      const file = e.dataTransfer?.files[0]
      if (!file) return
      e.preventDefault()
      void openDroppedFile(file)
    }
    window.addEventListener('beforeunload', beforeUnload)
    window.addEventListener('dragover', dragOver)
    window.addEventListener('drop', drop)
    return () => {
      stopAutosave()
      stopValidation()
      window.removeEventListener('beforeunload', beforeUnload)
      window.removeEventListener('dragover', dragOver)
      window.removeEventListener('drop', drop)
    }
  }, [])
}

/** Offers back the draft an earlier session left (a crash or a reload with unsaved changes). */
function DraftOffer() {
  const [draft, setDraft] = useState<Draft | null>(null)
  useEffect(() => {
    let cancelled = false
    void pendingDraft().then((d) => !cancelled && setDraft(d))
    return () => {
      cancelled = true
    }
  }, [])
  if (!draft) return null
  const restore = async () => {
    setDraft(null)
    try {
      useEditor.getState().load(await fromScene(draft.scene))
      // Still unsaved: keep it dirty, so the draft is kept until it is saved.
      useEditor.setState({ savedVersion: -1 })
      useEditor.getState().notify('info', `Restored “${draft.scene.name}”`)
    } catch (e) {
      useEditor.getState().notify('error', `The draft cannot be restored: ${(e as Error).message}`)
    }
  }
  return (
    <div
      className="flex items-center gap-3 border-b border-primary/40 bg-primary/10 px-3 py-1 text-sm"
      data-testid="draft-offer"
    >
      Unsaved drawing “{draft.scene.name}” from{' '}
      {new Date(draft.savedAt).toLocaleString('en-GB', { dateStyle: 'medium', timeStyle: 'short' })}
      <button className="text-xs font-medium underline" onClick={restore}>
        Restore
      </button>
      <button
        className="text-xs underline"
        onClick={() => {
          setDraft(null)
          discardDraft()
        }}
      >
        Discard
      </button>
    </div>
  )
}

function NoticeBar() {
  const notice = useEditor((s) => s.notice)
  const dismiss = useEditor((s) => s.dismissNotice)
  // Information fades after a few seconds; errors stay until dismissed.
  useEffect(() => {
    if (notice?.kind !== 'info') return
    const t = setTimeout(dismiss, 4000)
    return () => clearTimeout(t)
  }, [notice, dismiss])
  if (!notice) return null
  const error = notice.kind === 'error'
  return (
    <div
      role={error ? 'alert' : 'status'}
      data-testid="notice"
      className={
        error
          ? 'flex items-center gap-2 border-b border-red-500/40 bg-red-500/10 px-3 py-1 text-sm text-red-400'
          : 'flex items-center gap-2 border-b border-border bg-muted px-3 py-1 text-sm'
      }
    >
      {notice.text}
      <button className="ml-auto text-xs underline" onClick={dismiss}>
        dismiss
      </button>
    </div>
  )
}

export default function App() {
  useShortcuts()
  useDocumentGuards()
  const route = useRoute()
  // The editor's document stays in the store while a run is shown: "Editor" returns to it as it was.
  if (route.name === 'job') return <JobView id={route.id} />
  if (route.name === 'jobs') return <JobsList />
  return (
    <div className="flex h-full flex-col">
      <Header />
      <DraftOffer />
      <NoticeBar />
      <ToolOptions />
      <div className="flex min-h-0 flex-1">
        <Toolbar />
        <EditorCanvas />
        <ScenePanel />
      </div>
      <StatusBar />
    </div>
  )
}
