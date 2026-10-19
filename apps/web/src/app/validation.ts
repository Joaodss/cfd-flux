// Debounced validation against the server (design proposal §8): after each committed change, once
// the editor has been quiet for a moment, the scene is encoded and posted to
// `/api/scenes/validate`. A newer change aborts the request in flight (AbortController is the
// browser's CancellationToken), so only the latest document's answer is shown.
import { api, ApiError, type Validation } from '../api/client.ts'
import { useEditor, type EditorState } from '../editor/store.ts'
import { toScene } from '../scene/io.ts'

/** Quiet time before validating (ms). */
export const VALIDATE_DELAY_MS = 400

/** The last answer, kept while a new one is pending (the panel does not flicker). */
function last(s: EditorState): Validation | null {
  const v = s.validation
  return v.status === 'done' ? v.result : v.status === 'pending' ? v.last : null
}

/** Validates the current document now; resolves when the answer (or failure) is in the store. */
export async function validateNow(signal?: AbortSignal): Promise<void> {
  const s = useEditor.getState()
  s.setValidation({ status: 'pending', last: last(s) })
  try {
    const scene = await toScene({ table: s.table, layers: s.layers, fluidId: s.fluidId })
    const result = await api.validate(scene, signal)
    useEditor.getState().setValidation({ status: 'done', result })
  } catch (e) {
    if ((e as Error).name === 'AbortError') return // a newer validation replaced this one
    const message = e instanceof ApiError || e instanceof Error ? e.message : String(e)
    useEditor.getState().setValidation({ status: 'offline', message })
  }
}

/** Starts validating after changes; returns a function that stops it. */
export function startValidation(delay = VALIDATE_DELAY_MS): () => void {
  let timer: ReturnType<typeof setTimeout> | undefined
  let controller: AbortController | null = null
  let { historyVersion, layers } = useEditor.getState()
  const run = () => {
    controller?.abort()
    controller = new AbortController()
    void validateNow(controller.signal)
  }
  // subscribe() fires on every store change: act only on edits (history) and new documents.
  const unsubscribe = useEditor.subscribe((s) => {
    if (s.historyVersion === historyVersion && s.layers === layers) return
    ;({ historyVersion, layers } = s)
    clearTimeout(timer)
    timer = setTimeout(run, delay)
  })
  run()
  return () => {
    clearTimeout(timer)
    controller?.abort()
    unsubscribe()
  }
}
