// Simulation jobs (03 §7): submit, list, read, pause/resume/cancel, and the WebSocket stream of a
// job's status and progress. Frames (binary messages) are for the Phase 5 viewer; the job view of
// Phase 4 asks for the smallest stream and ignores them.
import type { Scene } from '../generated/scene.ts'
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
  /** Cancels if needed, then deletes the job's results and record. */
  purge: (id: string) =>
    api.request<unknown>('DELETE', `/api/simulations/${encodeURIComponent(id)}?purge=true`),
}

/** Text messages of the job stream (03 §7, "WebSocket messages"). */
export type StreamMessage =
  | { type: 'hello'; job: Job }
  | { type: 'status'; status: JobStatus; reason?: FailReason; message?: string }
  | ({ type: 'progress' } & Progress)
  | { type: 'log'; message: string }
  | {
      type: 'done'
      status: JobStatus
      reason?: FailReason
      message?: string
      stepsDone: number
      frames: number
    }

/** The stream's URL: same host as the page (the dev server proxies WebSockets too). */
export function streamUrl(id: string, location: Pick<Location, 'protocol' | 'host'>): string {
  const scheme = location.protocol === 'https:' ? 'wss:' : 'ws:'
  // Phase 4 needs no frames: the smallest stream (one field, at most 64 cells on a side).
  return `${scheme}//${location.host}/api/simulations/${encodeURIComponent(id)}/stream?fields=velocity&maxSize=64`
}

/**
 * Follows a job: calls `onMessage` for each text message until `done` or `close()`. Returns the
 * function that closes the socket. *(Like subscribing to an event: the browser calls back on the
 * page's thread whenever a message arrives.)*
 */
export function followJob(
  id: string,
  onMessage: (m: StreamMessage) => void,
  onClose: (normal: boolean) => void,
): () => void {
  const ws = new WebSocket(streamUrl(id, window.location))
  ws.binaryType = 'arraybuffer'
  let done = false
  ws.onmessage = (e) => {
    if (typeof e.data !== 'string') return // a frame: for the viewer (Phase 5)
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
