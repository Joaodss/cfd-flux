// Document commands shared by the header and the keyboard shortcuts: new, open (file, server,
// example), save (to where the document came from), save as a file, save to the server.
import { api, ApiError } from '../api/client.ts'
import { isDirty, useEditor, type DocSource } from '../editor/store.ts'
import { selectTool } from '../editor/tools/controller.ts'
import { newDocument } from '../scene/defaults.ts'
import type { Example } from '../scene/examples.ts'
import { fileNameFor, openText, saveText } from '../scene/files.ts'
import { fromScene, parseScene, toScene, type Document } from '../scene/io.ts'

const state = () => useEditor.getState()

function message(e: unknown): string {
  return e instanceof ApiError || e instanceof Error ? e.message : String(e)
}

/** Asks before discarding unsaved changes; true to go on. */
export function confirmDiscard(): boolean {
  return !isDirty(state()) || window.confirm('Discard the unsaved changes to this scene?')
}

/** The current document, with a floating selection dropped first. */
function current(): Document {
  selectTool.drop()
  const s = state()
  return { table: s.table, layers: s.layers, fluidId: s.fluidId }
}

function load(doc: Document, source: DocSource | null, what: string): void {
  state().load(doc, source)
  state().notify('info', `Opened ${what}`)
}

export function newScene(): void {
  if (!confirmDiscard()) return
  state().load(newDocument())
  state().dismissNotice()
}

export async function openExample(ex: Example): Promise<void> {
  if (!confirmDiscard()) return
  try {
    load(
      await fromScene(ex.scene),
      { kind: 'example', id: ex.id },
      `the example “${ex.scene.name}”`,
    )
  } catch (e) {
    state().notify('error', `${ex.id}: ${message(e)}`)
  }
}

export async function openFile(): Promise<void> {
  if (!confirmDiscard()) return
  try {
    const file = await openText()
    if (!file) return
    const doc = await parseScene(file.text)
    load(doc, { kind: 'file', name: file.name, handle: file.handle }, file.name)
  } catch (e) {
    state().notify('error', `Cannot open the file: ${message(e)}`)
  }
}

/** Opens a dropped or picked `File` (drag and drop onto the canvas). */
export async function openDroppedFile(file: File): Promise<void> {
  if (!confirmDiscard()) return
  try {
    load(await parseScene(await file.text()), { kind: 'file', name: file.name }, file.name)
  } catch (e) {
    state().notify('error', `Cannot open ${file.name}: ${message(e)}`)
  }
}

export async function openServerScene(id: string, name: string): Promise<boolean> {
  if (!confirmDiscard()) return false
  try {
    const doc = await fromScene(await api.getScene(id))
    load(doc, { kind: 'server', id, name }, `“${name}” from the server`)
    return true
  } catch (e) {
    state().notify('error', `Cannot open “${name}”: ${message(e)}`)
    return false
  }
}

/** Saves as a file (asks where, except with `handle`: overwrites it). */
export async function saveAsFile(handle?: FileSystemFileHandle): Promise<void> {
  try {
    const scene = await toScene(current())
    const saved = await saveText(
      JSON.stringify(scene, null, 2) + '\n',
      fileNameFor(scene.name),
      handle,
    )
    if (!saved) return
    state().markSaved({ kind: 'file', name: saved.name, handle: saved.handle })
    state().notify('info', `Saved ${saved.name}`)
  } catch (e) {
    state().notify('error', `Cannot save the file: ${message(e)}`)
  }
}

/** Saves to the server: updates the scene it came from, or adds a new one. */
export async function saveToServer(asNew = false): Promise<void> {
  try {
    const scene = await toScene(current())
    const src = state().source
    const saved =
      src?.kind === 'server' && !asNew
        ? await api.updateScene(src.id, scene)
        : await api.saveScene(scene)
    state().markSaved({ kind: 'server', id: saved.id, name: saved.name })
    state().notify('info', `Saved “${saved.name}” on the server`)
  } catch (e) {
    state().notify('error', `Cannot save to the server: ${message(e)}`)
  }
}

/** "Save" (Ctrl+S): to the file or server scene the document came from, else a new file. */
export async function save(): Promise<void> {
  const src = state().source
  if (src?.kind === 'server') return saveToServer()
  if (src?.kind === 'file' && src.handle) return saveAsFile(src.handle)
  return saveAsFile()
}
