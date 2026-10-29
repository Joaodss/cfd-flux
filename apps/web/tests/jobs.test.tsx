// @vitest-environment happy-dom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Job } from '../src/api/jobs.ts'
import { streamUrl } from '../src/api/jobs.ts'
import { INITIAL, reduce } from '../src/app/jobState.ts'
import { JobView } from '../src/app/JobView.tsx'
import { parseRoute, routeHash } from '../src/app/route.ts'
import { runScene } from '../src/app/run.ts'
import { useEditor } from '../src/editor/store.ts'
import { newDocument } from '../src/scene/defaults.ts'
import {
  DEFAULT_SETTINGS,
  loadSettings,
  maxSizeParam,
  saveSettings,
  useViewerSettings,
} from '../src/viewer/settings.ts'

const state = () => useEditor.getState()

const job: Job = {
  id: 'job-1',
  status: 'running',
  scene: 'Cylinder Re=100',
  backend: 'cuda',
  device: 'RTX 4060',
  stepsTotal: 48000,
  stepsDone: 0,
  frames: 0,
  createdAt: 0,
  startedAt: Date.now(),
}

describe('routes', () => {
  it('parse and print the hash', () => {
    expect(parseRoute('')).toEqual({ name: 'editor' })
    expect(parseRoute('#/')).toEqual({ name: 'editor' })
    expect(parseRoute('#/jobs')).toEqual({ name: 'jobs' })
    expect(parseRoute('#/jobs/a%2Fb')).toEqual({ name: 'job', id: 'a/b' })
    expect(routeHash({ name: 'job', id: 'a/b' })).toBe('#/jobs/a%2Fb')
    expect(streamUrl('x', { protocol: 'http:', host: 'localhost:5173' })).toBe(
      'ws://localhost:5173/api/simulations/x/stream?maxSize=1024',
    )
  })
})

describe('job state', () => {
  it('folds the stream messages', () => {
    let s = reduce(INITIAL, { kind: 'message', m: { type: 'hello', job } })
    expect(s.connection).toBe('open')
    s = reduce(s, {
      kind: 'message',
      m: {
        type: 'progress',
        step: 4800,
        total: 48000,
        simTime: 0.8,
        percent: 10,
        mlups: 3000,
        massDrift: 1e-6,
        maxMach: 0.09,
      },
    })
    expect(s.progress?.percent).toBe(10)
    s = reduce(s, { kind: 'message', m: { type: 'status', status: 'paused' } })
    expect(s.job?.status).toBe('paused')
    s = reduce(s, {
      kind: 'message',
      m: { type: 'log', message: 'stored frames reached the size limit' },
    })
    expect(s.log).toHaveLength(1)
    s = reduce(s, {
      kind: 'message',
      m: { type: 'done', status: 'cancelled', stepsDone: 5000, frames: 9 },
    })
    expect([s.job?.status, s.job?.stepsDone, s.connection]).toEqual(['cancelled', 5000, 'done'])
    // The socket closing after `done` is normal; before it, the connection was lost.
    expect(reduce(s, { kind: 'closed', normal: false }).connection).toBe('done')
    expect(
      reduce({ ...INITIAL, connection: 'open' }, { kind: 'closed', normal: false }).connection,
    ).toBe('lost')
  })
})

describe('Run', () => {
  let posts: string[]
  beforeEach(() => {
    posts = []
    window.location.hash = '#/'
    state().load(newDocument(40, 20))
    state().setValidation({
      status: 'done',
      result: {
        ok: true,
        issues: [{ code: 'domain.noOutlet', severity: 'warning', message: 'no outlet' }],
        estimate: null,
      },
    })
    vi.stubGlobal('fetch', async (url: string, init: RequestInit) => {
      posts.push(url)
      expect(JSON.parse(String(init.body)).scene.layers.cellType.encoding).toBe('deflate+base64')
      return new Response(JSON.stringify(job), { status: 201 })
    })
  })
  afterEach(() => vi.unstubAllGlobals())

  it('asks about warnings, then submits and opens the job', async () => {
    const asked: string[] = []
    expect(await runScene((t) => (asked.push(t), false))).toBe(false)
    expect(posts).toEqual([])
    expect(asked[0]).toMatch(/There is an inlet but no outlet/)
    expect(await runScene(() => true)).toBe(true)
    expect(posts).toEqual(['/api/simulations'])
    expect(window.location.hash).toBe('#/jobs/job-1')
  })

  it('refuses a scene with errors and reports a server refusal', async () => {
    state().setValidation({ status: 'done', result: { ok: false, issues: [], estimate: null } })
    expect(await runScene(() => true)).toBe(false)
    expect(posts).toEqual([])
    state().setValidation({ status: 'done', result: { ok: true, issues: [], estimate: null } })
    vi.stubGlobal(
      'fetch',
      async () =>
        new Response(
          JSON.stringify({ error: { code: 'queue.full', message: 'the queue is full' } }),
          {
            status: 429,
          },
        ),
    )
    expect(await runScene(() => true)).toBe(false)
    expect(state().notice?.text).toMatch(/the queue is full/)
  })
})

describe('JobView', () => {
  /** A WebSocket stand-in the test drives. */
  class FakeSocket {
    static last: FakeSocket
    url: string
    binaryType = 'blob'
    onmessage: ((e: { data: unknown }) => void) | null = null
    onclose: (() => void) | null = null
    closed = false
    constructor(url: string) {
      this.url = url
      FakeSocket.last = this
    }
    send(m: object) {
      act(() => this.onmessage?.({ data: JSON.stringify(m) }))
    }
    close() {
      this.closed = true
    }
  }
  let requests: string[]
  beforeEach(() => {
    requests = []
    vi.stubGlobal('WebSocket', FakeSocket)
    vi.stubGlobal('fetch', async (url: string, init: RequestInit) => {
      requests.push(`${init.method} ${url}`)
      return new Response(null, { status: 202 })
    })
  })
  afterEach(() => {
    cleanup()
    vi.unstubAllGlobals()
  })

  it('shows the stream and sends pause, resume and cancel', async () => {
    render(<JobView id="job-1" />)
    expect(FakeSocket.last.url).toMatch(/\/api\/simulations\/job-1\/stream\?/)
    expect(screen.getByText('Connecting…')).toBeTruthy()
    FakeSocket.last.send({ type: 'hello', job })
    FakeSocket.last.send({
      type: 'progress',
      step: 12000,
      total: 48000,
      simTime: 2,
      percent: 25,
      mlups: 3100,
      massDrift: 0,
      maxMach: 0.087,
    })
    expect(screen.getByTestId('job-status').textContent).toBe('running')
    expect(screen.getByTestId('job-percent').textContent).toBe('25.0 %')
    await act(async () => fireEvent.click(screen.getByTestId('job-pause')))
    FakeSocket.last.send({ type: 'status', status: 'paused' })
    await act(async () => fireEvent.click(screen.getByTestId('job-resume')))
    FakeSocket.last.send({ type: 'status', status: 'running' })
    await act(async () => fireEvent.click(screen.getByTestId('job-cancel')))
    FakeSocket.last.send({ type: 'done', status: 'cancelled', stepsDone: 12500, frames: 25 })
    expect(requests.filter((r) => !r.startsWith('GET'))).toEqual([
      'POST /api/simulations/job-1/pause',
      'POST /api/simulations/job-1/resume',
      'DELETE /api/simulations/job-1',
    ])
    expect(screen.getByTestId('job-status').textContent).toBe('cancelled')
    expect(screen.queryByTestId('job-cancel')).toBeNull()
  })

  it('closes the stream when it goes away', () => {
    const { unmount } = render(<JobView id="job-1" />)
    const socket = FakeSocket.last
    unmount()
    expect(socket.closed).toBe(true)
  })

  it('asks for the chosen viewer format and reconnects when it changes', () => {
    useViewerSettings.getState().setMaxSize(256)
    render(<JobView id="job-1" />)
    const first = FakeSocket.last
    expect(first.url).toMatch(/\/stream\?maxSize=256$/)
    first.send({ type: 'hello', job })
    act(() => useViewerSettings.getState().setMaxSize('full'))
    expect(first.closed).toBe(true)
    // The old stream is detached: nothing it says reaches the view any more.
    expect(first.onmessage).toBeNull()
    expect(FakeSocket.last).not.toBe(first)
    expect(FakeSocket.last.url).toMatch(/\/stream\?maxSize=16384$/)
    // The job's state survives the reconnect.
    expect(screen.getByTestId('job-status').textContent).toBe('running')
    act(() => useViewerSettings.getState().setMaxSize(1024))
  })
})

describe('viewer settings', () => {
  const store = (initial: Record<string, string> = {}) => {
    const items = new Map(Object.entries(initial))
    return {
      getItem: (k: string) => items.get(k) ?? null,
      setItem: (k: string, v: string) => void items.set(k, v),
      items,
    }
  }

  it('are remembered, and checked when read back', () => {
    const s = store()
    expect(loadSettings(s)).toEqual(DEFAULT_SETTINGS)
    saveSettings(s, { maxSize: 'full', budget: 2048 * 2 ** 20 })
    expect(loadSettings(s)).toEqual({ maxSize: 'full', budget: 2048 * 2 ** 20 })
    const bad = store({ 'cfd-flux.viewer.settings': '{"maxSize": 300, "budget": "lots"}' })
    expect(loadSettings(bad)).toEqual(DEFAULT_SETTINGS)
    expect(loadSettings(store({ 'cfd-flux.viewer.settings': 'not json' }))).toEqual(
      DEFAULT_SETTINGS,
    )
    expect(maxSizeParam('full')).toBe(16384)
    expect(maxSizeParam(512)).toBe(512)
  })

  it('survive a storage that throws', () => {
    const throwing = {
      getItem: () => {
        throw new Error('denied')
      },
      setItem: () => {
        throw new Error('quota')
      },
    }
    expect(loadSettings(throwing)).toEqual(DEFAULT_SETTINGS)
    expect(() => saveSettings(throwing, DEFAULT_SETTINGS)).not.toThrow()
    expect(loadSettings(null)).toEqual(DEFAULT_SETTINGS)
  })
})
