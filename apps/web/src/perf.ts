// Opt-in measurement switches for docs/benchmarks.md, read once from the page's query string
// (before the hash: `http://127.0.0.1:7878/?perf#/`). Production builds carry them; nothing changes
// unless they are in the URL.
//
// - `?perf`: User Timing marks (`performance.mark`, read back with
//   `performance.getEntriesByType('mark')`) at Run and at every frame the viewer shows, a measure
//   of each draw's CPU time, and `window.__viewer` (otherwise only in development builds).
// - `?noWebCodecs`: the video export behaves as if `VideoEncoder` were missing (the MediaRecorder
//   fallback).

const params = new URLSearchParams(typeof location === 'undefined' ? '' : location.search)

export const perfEnabled: boolean = params.has('perf')
export const noWebCodecs: boolean = params.has('noWebCodecs')

/** A named timestamp, only with `?perf`. */
export function mark(name: string, detail?: unknown): void {
  if (perfEnabled) performance.mark(name, { detail })
}

/** The time since `start` (a `performance.now()`) as a measure, only with `?perf`. */
export function measureSince(name: string, start: number): void {
  if (perfEnabled) performance.measure(name, { start, end: performance.now() })
}
