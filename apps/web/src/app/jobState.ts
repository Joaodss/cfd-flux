// The job view's state, folded from the stream's messages by a reducer (state + action → new
// state; React's useReducer, like a view model updated from a sequence of events).
import type { Job, Progress, StreamMessage } from '../api/jobs.ts'

export interface State {
  job: Job | null
  progress: Progress | null
  log: string[]
  /** `open`: following; `done`: the job finished; `lost`: the socket closed early. */
  connection: 'connecting' | 'open' | 'done' | 'lost'
}

export type Action =
  { kind: 'message'; m: StreamMessage } | { kind: 'closed'; normal: boolean } | { kind: 'reset' }

/** Folds the stream's messages into the view's state (a reducer: state + action → new state). */
export function reduce(s: State, a: Action): State {
  if (a.kind === 'reset') return INITIAL
  if (a.kind === 'closed')
    return { ...s, connection: a.normal || s.connection === 'done' ? 'done' : 'lost' }
  const m = a.m
  switch (m.type) {
    case 'hello':
      return { ...s, job: m.job, progress: m.job.progress ?? s.progress, connection: 'open' }
    case 'status':
      return s.job
        ? { ...s, job: { ...s.job, status: m.status, reason: m.reason, message: m.message } }
        : s
    case 'progress': {
      const { type: _, ...p } = m
      return { ...s, progress: p }
    }
    case 'log':
      return { ...s, log: [...s.log, m.message] }
    case 'series':
      return s // the series feed's (src/viewer/seriesFeed.ts)
    case 'done':
      return {
        ...s,
        connection: 'done',
        job: s.job && {
          ...s.job,
          status: m.status,
          reason: m.reason,
          message: m.message,
          stepsDone: m.stepsDone,
          frames: m.frames,
        },
      }
  }
}

export const INITIAL: State = { job: null, progress: null, log: [], connection: 'connecting' }
