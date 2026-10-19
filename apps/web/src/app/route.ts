// Hash routes (design proposal §9): `#/` the editor, `#/jobs` the job list, `#/jobs/{id}` a job.
// The hash part of the URL never reaches the server, so reload and back/forward work without
// server-side routes or a router library.
import { useSyncExternalStore } from 'react'

export type Route = { name: 'editor' } | { name: 'jobs' } | { name: 'job'; id: string }

export function parseRoute(hash: string): Route {
  const path = hash.replace(/^#/, '')
  const job = /^\/jobs\/([^/]+)$/.exec(path)
  if (job) return { name: 'job', id: decodeURIComponent(job[1]) }
  if (path === '/jobs') return { name: 'jobs' }
  return { name: 'editor' }
}

export function routeHash(r: Route): string {
  if (r.name === 'job') return `#/jobs/${encodeURIComponent(r.id)}`
  if (r.name === 'jobs') return '#/jobs'
  return '#/'
}

export function navigate(r: Route): void {
  window.location.hash = routeHash(r)
}

function subscribe(onChange: () => void): () => void {
  window.addEventListener('hashchange', onChange)
  return () => window.removeEventListener('hashchange', onChange)
}

/** The current route; the component re-renders when the hash changes. */
export function useRoute(): Route {
  // The snapshot must be a stable value between changes: the hash string, parsed after.
  const hash = useSyncExternalStore(subscribe, () => window.location.hash)
  return parseRoute(hash)
}
