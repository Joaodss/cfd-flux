import { readdirSync, readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Validation } from '../src/api/client.ts'
import { shownIssues } from '../src/app/issues.ts'
import { startValidation, validateNow } from '../src/app/validation.ts'
import { useEditor } from '../src/editor/store.ts'
import { issueText, KNOWN_CODES, num } from '../src/messages.ts'
import { formatBytes, formatDuration, formatValue } from '../src/panels/format.ts'
import { newDocument } from '../src/scene/defaults.ts'

const repo = (rel: string) => fileURLToPath(new URL(`../../../${rel}`, import.meta.url))
const state = () => useEditor.getState()
const ctx = {
  elementName: (id: number) => (id === 4 ? 'Cylinder' : `element ${id}`),
  fluidName: (id: number) => (id === 1 ? 'Air' : `fluid ${id}`),
}

describe('messages', () => {
  it('word every issue code cfd-core emits', () => {
    const dir = repo('crates/cfd-core/src')
    const source = readdirSync(dir)
      .filter((f) => f.endsWith('.rs'))
      .map((f) => readFileSync(`${dir}/${f}`, 'utf8'))
      .join('\n')
    // r.error("code", …), report.warning(\n "code", …), and check_tau's codes.
    const codes = new Set(
      [...source.matchAll(/(?:error|warning)\(\s*"([a-zA-Z]+\.[a-zA-Z]+)"/g)].map((m) => m[1]),
    )
    for (const m of source.matchAll(/"(stability\.tau[a-zA-Z]*)"/g)) codes.add(m[1])
    expect(codes.size).toBeGreaterThan(30)
    expect([...codes].filter((c) => !KNOWN_CODES.includes(c))).toEqual([])
  })

  it('are filled from params and the document', () => {
    expect(
      issueText(
        {
          code: 'cell.element',
          severity: 'error',
          message: 'Inlet cell (0, 3) does not touch any fluid cell',
          params: { problem: 'noFluidNeighbour', element: 4 },
          at: [0, 3],
        },
        ctx,
      ),
    ).toBe('Cylinder has a cell that touches no fluid cell')
    expect(
      issueText(
        {
          code: 'stability.mach',
          severity: 'warning',
          message: '…',
          params: { mach: 0.23456, limit: 0.17 },
        },
        ctx,
      ),
    ).toBe('Lattice Mach number 0.235 is above 0.17: compressibility errors grow')
    expect(
      issueText(
        {
          code: 'stability.mach',
          severity: 'error',
          message: '…',
          params: { mach: 0.6, limit: 0.5 },
        },
        ctx,
      ),
    ).toMatch(/too high/)
    // Unknown codes keep the server's English.
    expect(issueText({ code: 'new.code', severity: 'error', message: 'Server text' }, ctx)).toBe(
      'Server text',
    )
    expect(num(0.000123456)).toBe('0.000123')
  })

  it('list a limit failure before the scene issues', () => {
    const v: Validation = {
      ok: false,
      code: 'limits.steps',
      message: '200000000 steps exceed the server limit',
      issues: [{ code: 'domain.noOutlet', severity: 'warning', message: 'x' }],
      estimate: null,
    }
    expect(shownIssues(v).map((i) => i.code)).toEqual(['limits.steps', 'domain.noOutlet'])
    expect(shownIssues({ ...v, code: 'scene.invalid' }).map((i) => i.code)).toEqual([
      'domain.noOutlet',
    ])
  })
})

describe('formatting', () => {
  it('bytes, durations and values', () => {
    expect(formatBytes(512)).toBe('512 B')
    expect(formatBytes(3_456_000)).toBe('3.46 MB')
    expect(formatDuration(0.2)).toBe('< 1 s')
    expect(formatDuration(200)).toBe('3 min 20 s')
    expect(formatDuration(7500)).toBe('2 h 05 min')
    expect(formatValue(100.04)).toBe('100')
    expect(formatValue(2.5e-5)).toBe('2.50e-5')
  })
})

describe('validation runner', () => {
  const answer: Validation = { ok: true, issues: [], estimate: null }
  let calls: { signal: AbortSignal | null | undefined; body: string }[]

  beforeEach(() => {
    calls = []
    state().load(newDocument(40, 20))
    vi.stubGlobal('fetch', (url: string, init: RequestInit) => {
      expect(url).toBe('/api/scenes/validate')
      calls.push({ signal: init.signal, body: String(init.body) })
      return new Promise<Response>((resolve, reject) => {
        const t = setTimeout(() => resolve(new Response(JSON.stringify(answer))), 30)
        init.signal?.addEventListener('abort', () => {
          clearTimeout(t)
          reject(new DOMException('aborted', 'AbortError'))
        })
      })
    })
  })
  afterEach(() => vi.unstubAllGlobals())

  it('posts the encoded scene and stores the answer', async () => {
    await validateNow()
    expect(JSON.parse(calls[0].body).layers.cellType.encoding).toBe('deflate+base64')
    expect(state().validation).toEqual({ status: 'done', result: answer })
  })

  it('reports an unreachable server as offline', async () => {
    vi.stubGlobal('fetch', () => Promise.reject(new TypeError('Failed to fetch')))
    await validateNow()
    expect(state().validation.status).toBe('offline')
  })

  it('waits for a quiet moment and aborts outdated requests', async () => {
    const stop = startValidation(20)
    // The first validation starts at once; three quick edits replace it with one more.
    for (let i = 0; i < 3; i++) {
      state().updateTable(`edit ${i}`, (t) => void (t.run.endTime = 5 + i))
      await new Promise((r) => setTimeout(r, 5))
    }
    await new Promise((r) => setTimeout(r, 100))
    stop()
    expect(calls).toHaveLength(2)
    expect(calls[0].signal?.aborted).toBe(true)
    expect(JSON.parse(calls[1].body).run.endTime).toBe(7)
    expect(state().validation.status).toBe('done')
  })
})
