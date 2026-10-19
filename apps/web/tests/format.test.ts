import { describe, expect, it } from 'vitest'
import { coloursFor, elementColour } from '../src/editor/colours.ts'
import type { Element } from '../src/generated/scene.ts'
import { formatLength, formatZoom } from '../src/panels/format.ts'

describe('formatLength', () => {
  it('picks a unit', () => {
    expect(formatLength(2.21)).toBe('2.21 m')
    expect(formatLength(0.2025)).toBe('0.2025 m')
    expect(formatLength(0.005)).toBe('5 mm')
    expect(formatLength(1.5e-5)).toBe('15 µm')
    expect(formatLength(0)).toBe('0 m')
  })
})

describe('formatZoom', () => {
  it('reads px/cell above 1 and cells/px below', () => {
    expect(formatZoom(8)).toBe('8 px/cell')
    expect(formatZoom(Math.SQRT2)).toBe('1.41 px/cell')
    expect(formatZoom(1 / 8)).toBe('8 cells/px')
  })
})

describe('element colours', () => {
  const wall = (id: number): Element => ({
    id,
    kind: 'wall',
    velocity: { type: 'noSlip' },
    thermal: { type: 'adiabatic' },
  })

  it('differ between elements of one kind and are cached per elements array', () => {
    const elements = [wall(1), wall(4)]
    const c = coloursFor(elements)
    expect(c.elements.get(1)).toEqual(elementColour('wall', 0))
    expect(c.elements.get(4)).toEqual(elementColour('wall', 1))
    expect(c.elements.get(1)).not.toEqual(c.elements.get(4))
    expect(coloursFor(elements)).toBe(c)
  })
})
