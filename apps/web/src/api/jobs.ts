// Simulation jobs (03 §7): submit, list, read, pause/resume/cancel, the files of a run, and the
// WebSocket stream of a job's status, progress and frames (binary messages, for the viewer).
import type { Scene } from '../generated/scene.ts'
import type { Columns, Rows } from '../viewer/series.ts'
import { api, type Estimate, type Issue } from './client.ts'

export type JobStatus = 'queued' | 'running' | 'paused' | 'completed' | 'failed' | 'cancelled'
export type FailReason = 'diverged' | 'error' | 'interrupted'

export interface Progress {
  step: number
  total: number
  /** Simulated time (s). */
  simTime: number
  percent: number
  mlups: number
  massDrift: number
  maxMach: number
}

/** A job as the server reports it (`GET /api/simulations/{id}`). */
export interface Job {
  id: string
  status: JobStatus
  reason?: FailReason
  message?: string
  /** The scene's name. */
  scene: string
  sceneId?: string
  backend: string
  device?: string
  stepsTotal: number
  stepsDone: number
  frames: number
  progress?: Progress
  warnings?: Issue[]
  notes?: string[]
  estimate?: Estimate
  mlups?: number
  createdAt: number
  startedAt?: number
  finishedAt?: number
}

export const ACTIVE: readonly JobStatus[] = ['queued', 'running', 'paused']

/** The parts of a run's `meta.json` (03 §6.2) the viewer uses; written when the job starts. */
export interface RunMeta {
  units: { dx: number; dt: number; rho0: number }
  dimensionless: { reference: { length: number; velocity: number; reynolds: number } }
  run: { outputEverySteps: number; sampleEverySteps: number; endTime: number }
}

const path = (id: string, rest = '') => `/api/simulations/${encodeURIComponent(id)}${rest}`

export const jobs = {
  submit: (scene: Scene) => api.request<Job>('POST', '/api/simulations', { scene }),
  list: () => api.request<Job[]>('GET', '/api/simulations?limit=100'),
  get: (id: string) => api.request<Job>('GET', `/api/simulations/${encodeURIComponent(id)}`),
  pause: (id: string) =>
    api.request<unknown>('POST', `/api/simulations/${encodeURIComponent(id)}/pause`),
  resume: (id: string) =>
    api.request<unknown>('POST', `/api/simulations/${encodeURIComponent(id)}/resume`),
  cancel: (id: string) =>
    api.request<unknown>('DELETE', `/api/simulations/${encodeURIComponent(id)}`),
  /** The exact scene of a run (`results/{id}/scene.json`). */
  scene: (id: string) =>
    api.request<Scene>('GET', `/api/simulations/${encodeURIComponent(id)}/files/scene.json`),
  /** The run's `meta.json` (404 until the job has started). */
  meta: (id: string) => api.request<RunMeta>('GET', path(id, '/files/meta.json')),
  /** A file of the run as text (e.g. `forces.csv`). */
  text: (id: string, name: string) => api.text(path(id, `/files/${name}`)),
  /** Cancels if needed, then deletes the job's results and record. */
  purge: (id: string) =>
    api.request<unknown>('DELETE', `/api/simulations/${encodeURIComponent(id)}?purge=true`),
}

/** The stream format a viewer gets (`hello.format`): f16 frames box-downsampled by `factor`. */
export interface StreamFormat {
  fields: string[] | null
  maxSize: number
  factor: number
  width: number
  height: number
  dtype: 'f16'
}

/** Text messages of the job stream (03 §7, "WebSocket messages"). */
export type StreamMessage =
  | {
      type: 'hello'
      job: Job
      /** `null` until the job has started. */
      grid?: { width: number; height: number; dx: number; dt: number; thermal: boolean } | null
      format?: StreamFormat | null
      /** Min/max of each field over the frames so far (velocity: of its magnitude). */
      ranges?: Record<string, [number, number]>
      /** The columns of `forces`, `probes` and `diagnostics` (the CSV headers); `null` until the job has started. */
      series?: Columns | null
    }
  | { type: 'status'; status: JobStatus; reason?: FailReason; message?: string }
  | ({ type: 'progress' } & Progress)
  /**
   * The series rows sampled since the last `series` message (sent before each `progress`);
   * `resync`: some were missed (the viewer fell behind), read the files again.
   */
  | { type: 'series'; rows: Rows; resync?: boolean }
  | { type: 'log'; message: string }
  | {
      type: 'done'
      status: JobStatus
      reason?: FailReason
      message?: string
      stepsDone: number
      frames: number
    }

/** The viewer format's longest side (design proposal §3.1): every frame field, at most this. */
export const VIEWER_MAX_SIZE = 1024

/**
 * The stream's URL: same host as the page (the dev server proxies WebSockets too). Every frame
 * field (no `fields`), downsampled to at most `maxSize` cells on a side.
 */
export function streamUrl(
  id: string,
  location: Pick<Location, 'protocol' | 'host'>,
  maxSize = VIEWER_MAX_SIZE,
): string {
  const scheme = location.protocol === 'https:' ? 'wss:' : 'ws:'
  return `${scheme}//${location.host}/api/simulations/${encodeURIComponent(id)}/stream?maxSize=${maxSize}`
}

/**
 * Follows a job: calls `onMessage` for each text message and `onFrame` for each frame (an LFFR
 * frame of the viewer format) until `done` or `close()`. Returns the function that closes the
 * socket. *(Like subscribing to an event: the browser calls back on the page's thread whenever a
 * message arrives.)*
 */
export function followJob(
  id: string,
  onMessage: (m: StreamMessage) => void,
  onClose: (normal: boolean) => void,
  onFrame: (frame: ArrayBuffer) => void = () => {},
): () => void {
  const ws = new WebSocket(streamUrl(id, window.location))
  ws.binaryType = 'arraybuffer'
  let done = false
  ws.onmessage = (e) => {
    if (e.data instanceof ArrayBuffer) {
      onFrame(e.data)
      return
    }
    if (typeof e.data !== 'string') return
    const m = JSON.parse(e.data) as StreamMessage
    if (m.type === 'done') done = true
    onMessage(m)
  }
  ws.onclose = () => onClose(done)
  return () => {
    done = true
    ws.close()
  }
}
