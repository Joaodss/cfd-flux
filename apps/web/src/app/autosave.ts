// Keeps a draft of the document in IndexedDB while it has unsaved changes, and offers the draft
// left by an earlier session back at start-up.
import { isDirty, useEditor } from '../editor/store.ts'
import { clearDraft, loadDraft, saveDraft, type Draft } from '../scene/drafts.ts'
import { toScene } from '../scene/io.ts'

/** Seconds of quiet after a change before the draft is written. */
const DELAY_MS = 3000

/** Starts autosaving; returns a function that stops it. */
export function startAutosave(): () => void {
  let timer: ReturnType<typeof setTimeout> | undefined
  let { historyVersion, savedVersion } = useEditor.getState()
  // subscribe() fires on every store change (pointer moves too): act only on edits and saves.
  const unsubscribe = useEditor.subscribe((s) => {
    if (s.historyVersion === historyVersion && s.savedVersion === savedVersion) return
    ;({ historyVersion, savedVersion } = s)
    clearTimeout(timer)
    timer = setTimeout(() => {
      const now = useEditor.getState()
      const write = isDirty(now)
        ? toScene({ table: now.table, layers: now.layers, fluidId: now.fluidId }).then(saveDraft)
        : clearDraft() // saved or freshly loaded: nothing to recover
      write.catch(() => {
        // No IndexedDB (private mode, quota): autosave is a convenience, not an error.
      })
    }, DELAY_MS)
  })
  return () => {
    clearTimeout(timer)
    unsubscribe()
  }
}

/** The draft an earlier session left, if any (null when IndexedDB is unavailable). */
export async function pendingDraft(): Promise<Draft | null> {
  try {
    return await loadDraft()
  } catch {
    return null
  }
}

export function discardDraft(): void {
  void clearDraft().catch(() => {})
}
