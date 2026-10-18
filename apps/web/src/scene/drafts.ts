// Draft autosave (design proposal §7.2): the document is written to the browser's IndexedDB a few
// seconds after each change, and offered back on the next start, so a crash or a reload never
// loses a drawing. IndexedDB is callback-based (requests with onsuccess/onerror, like .NET's old
// event-based async pattern); the helpers below wrap it in Promises.
import type { Scene } from '../generated/scene.ts'

const DB = 'cfd-flux'
const STORE = 'drafts'
const KEY = 'current'

export interface Draft {
  savedAt: number
  scene: Scene
}

function promised<T>(r: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result)
    r.onerror = () => reject(r.error)
  })
}

function openDb(): Promise<IDBDatabase> {
  const r = indexedDB.open(DB, 1)
  r.onupgradeneeded = () => r.result.createObjectStore(STORE)
  return promised(r)
}

async function withStore<T>(
  mode: IDBTransactionMode,
  f: (s: IDBObjectStore) => IDBRequest<T>,
): Promise<T> {
  const db = await openDb()
  try {
    return await promised(f(db.transaction(STORE, mode).objectStore(STORE)))
  } finally {
    db.close()
  }
}

export async function saveDraft(scene: Scene): Promise<void> {
  const draft: Draft = { savedAt: Date.now(), scene }
  await withStore('readwrite', (s) => s.put(draft, KEY))
}

export async function loadDraft(): Promise<Draft | null> {
  return ((await withStore('readonly', (s) => s.get(KEY))) as Draft | undefined) ?? null
}

export async function clearDraft(): Promise<void> {
  await withStore('readwrite', (s) => s.delete(KEY))
}
