// Helpers shared by the issue list and the canvas markers.
import type { Issue, Validation } from '../api/client.ts'
import { elementLabel } from '../editor/elements.ts'
import type { EditorState, ValidationState } from '../editor/store.ts'
import type { MessageContext } from '../messages.ts'

/** The latest answer: the current one, or the last one while a new one is pending. */
export function currentValidation(v: ValidationState): Validation | null {
  return v.status === 'done' ? v.result : v.status === 'pending' ? v.last : null
}

/** The issues to show: the top-level failure (a limit, no engine) first, then the scene's. */
export function shownIssues(v: Validation): Issue[] {
  const top: Issue[] =
    !v.ok && v.code && v.code !== 'scene.invalid'
      ? [{ code: v.code, severity: 'error', message: v.message ?? v.code }]
      : []
  return [...top, ...v.issues]
}

export function messageContext(s: Pick<EditorState, 'table'>): MessageContext {
  return {
    elementName: (id) => {
      const e = s.table.elements.find((x) => x.id === id)
      return e ? elementLabel(e) : `element ${id}`
    },
    fluidName: (id) => s.table.fluids.find((f) => f.id === id)?.name ?? `fluid ${id}`,
  }
}
