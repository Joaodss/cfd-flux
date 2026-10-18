// The editor's layout: header, tool strip, canvas, side panel and status bar.
import { useEffect, useState } from 'react'
import { Header } from './app/Header.tsx'
import { ToolOptions } from './app/ToolOptions.tsx'
import { Toolbar } from './app/Toolbar.tsx'
import { EditorCanvas } from './editor/EditorCanvas.tsx'
import { handleKey } from './editor/tools/controller.ts'
import { ScenePanel } from './panels/ScenePanel.tsx'
import { StatusBar } from './panels/StatusBar.tsx'

/** The editor's keyboard shortcuts (ignored while typing in a field). */
function useShortcuts() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target
      if (t instanceof HTMLInputElement || t instanceof HTMLTextAreaElement) return
      if (t instanceof HTMLElement && t.closest('[role=menu], [role=dialog]')) return
      if (handleKey(e)) e.preventDefault()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])
}

export default function App() {
  const [error, setError] = useState<string | null>(null)
  useShortcuts()
  return (
    <div className="flex h-full flex-col">
      <Header onError={setError} />
      {error && (
        <div className="flex items-center gap-2 border-b border-red-500/40 bg-red-500/10 px-3 py-1 text-sm text-red-400">
          {error}
          <button className="ml-auto text-xs underline" onClick={() => setError(null)}>
            dismiss
          </button>
        </div>
      )}
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
