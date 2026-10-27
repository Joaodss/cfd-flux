// @vitest-environment happy-dom
// Component tests: the property panels rendered into a simulated DOM (happy-dom), driven like a
// user would (Testing Library's fireEvent).
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { NumberField } from '../src/components/fields.tsx'
import { useEditor } from '../src/editor/store.ts'
import { ElementPanel } from '../src/panels/ElementPanel.tsx'
import { RunPanel } from '../src/panels/RunPanel.tsx'
import { ScenePropertiesPanel } from '../src/panels/ScenePropertiesPanel.tsx'
import { newDocument } from '../src/scene/defaults.ts'

const state = () => useEditor.getState()

afterEach(cleanup)

describe('NumberField', () => {
  it('commits valid text on Enter, refuses invalid text, reverts on Escape', () => {
    const commits: number[] = []
    render(
      <NumberField
        label="Cell size"
        unit="m"
        value={0.005}
        rules={{ positive: true }}
        onCommit={(v) => commits.push(v)}
        testId="f"
      />,
    )
    const input = screen.getByTestId('f') as HTMLInputElement
    expect(input.value).toBe('0.005')
    fireEvent.focus(input)
    fireEvent.change(input, { target: { value: '-1' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(commits).toEqual([])
    expect(input.getAttribute('aria-invalid')).toBe('true')
    fireEvent.change(input, { target: { value: '2.5e-3' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(commits).toEqual([0.0025])
    fireEvent.focus(input)
    fireEvent.change(input, { target: { value: '9' } })
    fireEvent.keyDown(input, { key: 'Escape' })
    expect(commits).toEqual([0.0025])
    expect(input.value).toBe('0.005') // the prop value (the parent did not change it)
  })
})

describe('ElementPanel', () => {
  beforeEach(() => state().load(newDocument(40, 20)))

  it('shows the fields of the inlet profile chosen, and edits are undoable', () => {
    const id = state().createElement('inlet')!
    render(<ElementPanel />)
    expect(screen.getByTestId('element-name')).toHaveProperty('value', 'Inlet 1')
    const vx = screen.getByTestId('inlet-velocity-x') as HTMLInputElement
    expect(vx.value).toBe('0.1')
    fireEvent.focus(vx)
    fireEvent.change(vx, { target: { value: '1.5' } })
    fireEvent.blur(vx)
    const inlet = () => state().table.elements.find((e) => e.id === id)!
    expect(inlet()).toMatchObject({ velocity: { type: 'uniform', value: [1.5, 0] } })
    fireEvent.change(screen.getByTestId('inlet-profile'), { target: { value: 'parabolic' } })
    expect(inlet()).toMatchObject({ velocity: { type: 'parabolic', peak: [1.5, 0] } })
    expect(screen.getByText('Peak velocity')).toBeTruthy()
    state().undo()
    expect(inlet()).toMatchObject({ velocity: { type: 'uniform', value: [1.5, 0] } })
  })

  it('switches the thermal form with the condition type', () => {
    state().createElement('wall')
    render(<ElementPanel />)
    fireEvent.change(screen.getByTestId('element-thermal'), { target: { value: 'convective' } })
    expect(state().table.elements[0]).toMatchObject({
      thermal: { type: 'convective', h: 10, ambient: 293.15 },
    })
    expect(screen.getByText('Coefficient h')).toBeTruthy()
  })
})

describe('RunPanel', () => {
  beforeEach(() => state().load(newDocument(40, 20)))

  it('edits the run settings and shows the frame count', () => {
    render(<RunPanel />)
    expect(screen.getByText('201')).toBeTruthy() // 10 s every 0.05 s, plus the initial frame
    const end = screen.getByTestId('run-end-time')
    fireEvent.focus(end)
    fireEvent.change(end, { target: { value: '2' } })
    fireEvent.keyDown(end, { key: 'Enter' })
    expect(state().table.run.endTime).toBe(2)
    expect(screen.getByText('41')).toBeTruthy()
    fireEvent.change(screen.getByTestId('run-backend'), { target: { value: 'cpu' } })
    expect(state().table.run.backend).toBe('cpu')
  })
})

describe('Scene tab: domain edges', () => {
  beforeEach(() => {
    state().load(newDocument(40, 20))
    state().setPanelTab('scene')
  })

  it('shows each edge kind, and makes an edge an inlet in one click', () => {
    render(<ScenePropertiesPanel />)
    for (const side of ['left', 'right', 'bottom', 'top']) {
      expect(screen.getByTestId(`edge-${side}-summary`).textContent).toBe('wall')
    }
    const inlet = screen.getByTestId('make-inlet-left') as HTMLButtonElement
    fireEvent.click(inlet)
    expect(state().table.elements.map((e) => e.name)).toEqual(['Inlet 1'])
    expect(screen.getByTestId('edge-left-summary').textContent).toBe('Inlet 1')
    expect(inlet.disabled).toBe(true) // already an inlet
    expect(inlet.title).toBe('Already an inlet: Inlet 1')
    // The name opens the element in the Element tab.
    fireEvent.click(screen.getByTestId('edge-left-element-1'))
    expect(state().panelTab).toBe('element')
    state().undo()
    expect(state().table.elements).toHaveLength(0)
  })

  it('makes the channel walls one wall element, until a new one is started', () => {
    render(<ScenePropertiesPanel />)
    fireEvent.click(screen.getByTestId('make-wall-bottom'))
    fireEvent.click(screen.getByTestId('make-wall-top'))
    expect(screen.getByTestId('edge-top-summary').textContent).toBe('Wall 1')
    const top = screen.getByTestId('make-wall-top') as HTMLButtonElement
    expect(top.disabled).toBe(true)
    expect(top.title).toMatch(/^Already Wall 1/)
    act(() => state().setActiveElement('wall', null)) // "+" or N in the palette
    expect(top.disabled).toBe(false)
    expect(top.title).toMatch(/a new wall/)
  })

  it('disables the buttons of periodic edges', () => {
    render(<ScenePropertiesPanel />)
    fireEvent.change(screen.getByTestId('edge-top-kind'), { target: { value: 'periodic' } })
    expect(state().table.grid.edges).toMatchObject({ top: 'periodic', bottom: 'periodic' })
    expect(screen.getByTestId('edge-bottom-summary').textContent).toBe('periodic')
    expect((screen.getByTestId('make-outlet-bottom') as HTMLButtonElement).disabled).toBe(true)
    expect((screen.getByTestId('make-outlet-left') as HTMLButtonElement).disabled).toBe(false)
  })
})
