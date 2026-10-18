import { readdirSync, readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'
import { CellType } from '../src/editor/layers.ts'
import type { Scene } from '../src/generated/scene.ts'
import { base64Decode } from '../src/scene/codec.ts'
import { fromScene } from '../src/scene/io.ts'
import { SCHEMA_VERSION } from '../src/scene/version.ts'

const repo = (rel: string) => fileURLToPath(new URL(`../../../${rel}`, import.meta.url))
const scenes: [string, Scene][] = readdirSync(repo('scenes'))
  .filter((f) => f.endsWith('.json'))
  .map((f) => [f, JSON.parse(readFileSync(repo(`scenes/${f}`), 'utf8'))])

const KIND_CELL: Record<string, number> = {
  wall: CellType.Solid,
  inlet: CellType.Inlet,
  outlet: CellType.Outlet,
  heatSource: CellType.HeatSource,
}

describe('base64Decode', () => {
  it('matches Node for every length and byte value', () => {
    const bytes = Uint8Array.from({ length: 300 }, (_, i) => (i * 37 + 11) & 0xff)
    for (let n = 0; n <= bytes.length; n += 7) {
      const b64 = Buffer.from(bytes.subarray(0, n)).toString('base64')
      expect(base64Decode(b64)).toEqual(bytes.subarray(0, n))
      expect(base64Decode(b64.replace(/=+$/, ''))).toEqual(bytes.subarray(0, n))
    }
  })

  it('rejects invalid input', () => {
    expect(() => base64Decode('ab$d')).toThrow(/character/)
    expect(() => base64Decode('abcde')).toThrow(/length/)
  })
})

describe('fromScene', () => {
  it.each(scenes)('decodes %s consistently with its elements', async (_, scene) => {
    const { table, layers } = await fromScene(scene)
    expect(table.schemaVersion).toBe(SCHEMA_VERSION)
    expect('layers' in table).toBe(false)
    expect([layers.width, layers.height]).toEqual([scene.grid.width, scene.grid.height])
    const kinds = new Map(scene.elements.map((e) => [e.id, KIND_CELL[e.kind]]))
    for (let i = 0; i < layers.cellType.length; i++) {
      const id = layers.elementId[i]
      if (id === 0) {
        expect([CellType.Fluid, CellType.Empty]).toContain(layers.cellType[i])
      } else {
        expect(layers.cellType[i]).toBe(kinds.get(id))
      }
    }
  })

  it('reads the cylinder where examples.rs drew it', async () => {
    const [, scene] = scenes.find(([f]) => f.startsWith('cylinder'))!
    const { layers } = await fromScene(scene)
    const at = (x: number, y: number) => [
      layers.cellType[layers.index(x, y)],
      layers.elementId[layers.index(x, y)],
    ]
    expect(at(10, 0)).toEqual([CellType.Solid, 1]) // bottom wall
    expect(at(0, 10)).toEqual([CellType.Inlet, 2])
    expect(at(441, 10)).toEqual([CellType.Outlet, 3])
    expect(at(41, 41)).toEqual([CellType.Solid, 4]) // cylinder centre (1 + 0.2 m / 5 mm)
    expect(at(41, 60)).toEqual([CellType.Fluid, 0])
  })

  it('rejects a layer of the wrong size', async () => {
    const [, scene] = scenes[0]
    const bad = { ...scene, grid: { ...scene.grid, width: scene.grid.width + 1 } }
    await expect(fromScene(bad)).rejects.toThrow(/cellType: expected/)
  })
})
