// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ExampleGallery } from '../src/app/ExampleGallery.tsx'
import { ALL_EXAMPLES, EXAMPLES } from '../src/scene/examples.ts'

describe('examples', () => {
  it('group the quick variants under their example', () => {
    const ids = EXAMPLES.map((e) => e.id)
    expect(ids).toEqual(['channel', 'cylinder-re100', 'dam-break', 'heated-cavity-ra1e5'])
    for (const e of EXAMPLES) {
      if (e.id === 'dam-break') expect(e.quick).toBeUndefined()
      else expect(e.quick?.id).toBe(`${e.id}-quick`)
    }
    expect(ALL_EXAMPLES.length).toBe(EXAMPLES.length + 3)
  })

  it('give a quick variant the benchmark of its example', () => {
    const cyl = EXAMPLES.find((e) => e.id === 'cylinder-re100')!
    expect(cyl.benchmark).toBe('schaferTurek2D2')
    expect(cyl.quick?.benchmark).toBe('schaferTurek2D2')
    // Coarser, shorter, on the same domain.
    expect(cyl.quick!.scene.grid.width).toBeLessThan(cyl.scene.grid.width)
    expect(cyl.quick!.scene.run.endTime).toBeLessThan(cyl.scene.run.endTime)
  })
})

describe('ExampleGallery', () => {
  const requests: { url: string; method: string }[] = []
  let auto: 'cpu' | 'cuda' = 'cpu'

  beforeEach(() => {
    requests.length = 0
    window.location.hash = '#/'
    vi.stubGlobal('fetch', async (url: string, init?: RequestInit) => {
      requests.push({ url, method: init?.method ?? 'GET' })
      if (url === '/api/capabilities') {
        return new Response(JSON.stringify({ version: '0', engines: [], auto }))
      }
      return new Response(JSON.stringify({ id: 'job-7', status: 'completed' }), { status: 200 })
    })
  })
  afterEach(() => {
    cleanup()
    vi.unstubAllGlobals()
  })

  it('offers the quick variants on a CPU-only server and opens the results of one', async () => {
    render(<ExampleGallery open onOpenChange={() => {}} />)
    await screen.findByTestId('gallery-quick')
    expect(screen.getByTestId('example-cylinder-re100').textContent).toMatch(/\(quick\)/)
    // The dam break has no variant and cannot run yet.
    expect(screen.getByTestId('example-dam-break').textContent).not.toMatch(/\(quick\)/)
    expect((screen.getByTestId('results-dam-break') as HTMLButtonElement).disabled).toBe(true)

    fireEvent.click(screen.getByTestId('results-cylinder-re100'))
    await waitFor(() => expect(window.location.hash).toBe('#/jobs/job-7'))
    expect(requests.at(-1)).toEqual({
      url: '/api/simulations?reuse=completed',
      method: 'POST',
    })

    // Without the quick variants the full example is shown.
    fireEvent.click(screen.getByTestId('gallery-quick'))
    expect(screen.getByTestId('example-cylinder-re100').textContent).not.toMatch(/\(quick\)/)
  })

  it('shows the full examples on a GPU server', async () => {
    auto = 'cuda'
    render(<ExampleGallery open onOpenChange={() => {}} />)
    await waitFor(() => expect(requests.some((r) => r.url === '/api/capabilities')).toBe(true))
    expect(screen.queryByTestId('gallery-quick')).toBeNull()
    expect(screen.getByTestId('example-cylinder-re100').textContent).not.toMatch(/\(quick\)/)
    auto = 'cpu'
  })
})
