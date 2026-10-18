// A typed client for the cfd-server API (03 §7). Relative URLs: the Vite dev server proxies
// `/api` to 127.0.0.1:7878, and a release build is served by cfd-server itself.
import type { Scene } from '../generated/scene.ts'

/** A validation issue as the server reports it. */
export interface Issue {
  code: string
  severity: 'error' | 'warning'
  /** English text; the editor words known codes itself from `params` (src/messages.ts). */
  message: string
  params?: Record<string, string | number>
  /** The cell the issue is about. */
  at?: [number, number]
}

/** What running the scene would take, and the numbers derived from it (03 §7). */
export interface Estimate {
  cells: number
  fluidCells: number
  steps: number
  frames: number
  thermal: boolean
  memoryBytes: number
  storedBytes: number
  engine: 'cpu' | 'wgpu' | 'cuda' | null
  seconds?: number
  numbers: {
    velocity: number
    length: number
    reynolds: number
    mach: number
    prandtl?: number
    rayleigh?: number
    tau: number
    tauThermal?: number
    dx: number
    dt: number
    latticeVelocity: number
  }
}

/** The answer of `POST /api/scenes/validate`. */
export interface Validation {
  ok: boolean
  code?: string
  message?: string
  issues: Issue[]
  estimate: Estimate | null
}

/** An API error with the server's stable code (`offline` when the server cannot be reached). */
export class ApiError extends Error {
  readonly code: string
  readonly status: number
  readonly issues: Issue[]

  constructor(code: string, message: string, status: number, issues: Issue[] = []) {
    super(message)
    this.code = code
    this.status = status
    this.issues = issues
  }
}

async function request<T>(
  method: string,
  path: string,
  body?: unknown,
  signal?: AbortSignal,
): Promise<T> {
  let r: Response
  try {
    r = await fetch(path, {
      method,
      signal,
      headers: body === undefined ? undefined : { 'content-type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    })
  } catch (e) {
    if ((e as Error).name === 'AbortError') throw e
    throw new ApiError('offline', 'the cfd-server cannot be reached (is it running?)', 0)
  }
  if (r.status === 204) return undefined as T
  const text = await r.text()
  let json: unknown = null
  try {
    json = text ? JSON.parse(text) : null
  } catch {
    // Not JSON: e.g. the Vite proxy's own error page when the server is down.
  }
  if (!r.ok) {
    const err = (json as { error?: { code: string; message: string; issues?: Issue[] } } | null)
      ?.error
    if (err) throw new ApiError(err.code, err.message, r.status, err.issues)
    if (r.status === 502 || r.status === 504) {
      throw new ApiError('offline', 'the cfd-server cannot be reached (is it running?)', r.status)
    }
    throw new ApiError('http', `${method} ${path}: HTTP ${r.status}`, r.status)
  }
  return json as T
}

/** A saved scene in the server's list. */
export interface SceneSummary {
  id: string
  name: string
  createdAt: number
  updatedAt: number
}

export interface SavedScene {
  id: string
  name: string
}

export const api = {
  listScenes: () => request<SceneSummary[]>('GET', '/api/scenes'),
  getScene: (id: string) => request<Scene>('GET', `/api/scenes/${encodeURIComponent(id)}`),
  saveScene: (scene: Scene) => request<SavedScene>('POST', '/api/scenes', scene),
  updateScene: (id: string, scene: Scene) =>
    request<SavedScene>('PUT', `/api/scenes/${encodeURIComponent(id)}`, scene),
  validate: (scene: Scene, signal?: AbortSignal) =>
    request<Validation>('POST', '/api/scenes/validate', scene, signal),
  deleteScene: (id: string) => request<void>('DELETE', `/api/scenes/${encodeURIComponent(id)}`),
  request,
}
