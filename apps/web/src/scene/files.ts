// Reading and writing `.cfdscene` files in the browser. Chromium has the File System Access API
// (a save dialog whose handle lets "Save" overwrite the same file); elsewhere saving downloads a
// file and opening uses a file input.

/** The parts of the File System Access API used here (not yet in TypeScript's DOM types). */
interface FilePickerOptions {
  suggestedName?: string
  types?: { description: string; accept: Record<string, string[]> }[]
  excludeAcceptAllOption?: boolean
}
type PickerWindow = Window & {
  showSaveFilePicker?: (o?: FilePickerOptions) => Promise<FileSystemFileHandle>
  showOpenFilePicker?: (o?: FilePickerOptions) => Promise<FileSystemFileHandle[]>
}

const TYPES = [
  { description: 'cfd-flux scene', accept: { 'application/json': ['.cfdscene', '.json'] } },
]

export const SCENE_EXTENSION = '.cfdscene'

/** A file name from a scene name: `Cylinder Re=100` → `Cylinder Re=100.cfdscene`. */
export function fileNameFor(sceneName: string): string {
  const base = sceneName.replace(/[\\/:*?"<>|]+/g, '_').trim() || 'scene'
  return base.endsWith(SCENE_EXTENSION) ? base : `${base}${SCENE_EXTENSION}`
}

function isAbort(e: unknown): boolean {
  return e instanceof DOMException && e.name === 'AbortError'
}

/**
 * Saves `text` to a file. With a `handle` (from an earlier save or open) it overwrites that file;
 * otherwise asks where (or downloads). Returns the file name and handle, or null if cancelled.
 */
export async function saveText(
  text: string,
  suggestedName: string,
  handle?: FileSystemFileHandle,
): Promise<{ name: string; handle?: FileSystemFileHandle } | null> {
  const w = window as PickerWindow
  try {
    let h = handle
    if (!h && w.showSaveFilePicker) h = await w.showSaveFilePicker({ suggestedName, types: TYPES })
    if (h) {
      const writable = await h.createWritable()
      await writable.write(text)
      await writable.close()
      return { name: h.name, handle: h }
    }
  } catch (e) {
    if (isAbort(e)) return null
    throw e
  }
  // Fallback: a download (the browser decides where it goes).
  const url = URL.createObjectURL(new Blob([text], { type: 'application/json' }))
  const a = document.createElement('a')
  a.href = url
  a.download = suggestedName
  a.click()
  setTimeout(() => URL.revokeObjectURL(url), 10_000)
  return { name: suggestedName }
}

/** Asks for a file and returns its text, name and (where available) handle; null if cancelled. */
export async function openText(
  accept = '.cfdscene,.json,application/json',
): Promise<{ text: string; name: string; handle?: FileSystemFileHandle } | null> {
  const w = window as PickerWindow
  if (w.showOpenFilePicker && accept.includes('.cfdscene')) {
    try {
      const [handle] = await w.showOpenFilePicker({ types: TYPES })
      const file = await handle.getFile()
      return { text: await file.text(), name: file.name, handle }
    } catch (e) {
      if (isAbort(e)) return null
      throw e
    }
  }
  const file = await pickFile(accept)
  return file && { text: await file.text(), name: file.name }
}

/** A file chosen through a hidden `<input type="file">` (null if the dialog was cancelled). */
export function pickFile(accept: string): Promise<File | null> {
  return new Promise((resolve) => {
    const input = document.createElement('input')
    input.type = 'file'
    input.accept = accept
    input.onchange = () => resolve(input.files?.[0] ?? null)
    input.oncancel = () => resolve(null)
    input.click()
  })
}
