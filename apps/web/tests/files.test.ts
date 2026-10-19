import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'
import { CellType } from '../src/editor/layers.ts'
import type { Scene } from '../src/generated/scene.ts'
import {
  base64Decode,
  base64Encode,
  decodeU16,
  decodeU8,
  encodeU16,
  encodeU8,
  inflateRaw,
} from '../src/scene/codec.ts'
import { fileNameFor } from '../src/scene/files.ts'
import { fromScene, parseScene, toScene } from '../src/scene/io.ts'
import { imageMask } from '../src/scene/mask.ts'

const repo = (rel: string) => fileURLToPath(new URL(`../../../${rel}`, import.meta.url))
const example = (name: string): Scene =>
  JSON.parse(readFileSync(repo(`scenes/${name}.json`), 'utf8'))

describe('base64Encode', () => {
  it('matches Node for every length, and round-trips', () => {
    const bytes = Uint8Array.from({ length: 70_000 }, (_, i) => (i * 131 + 7) & 0xff)
    for (const n of [0, 1, 2, 3, 4, 5, 100, 24_575, 24_576, 24_577, 70_000]) {
      const b64 = base64Encode(bytes.subarray(0, n))
      expect(b64).toBe(Buffer.from(bytes.subarray(0, n)).toString('base64'))
      expect(base64Decode(b64)).toEqual(bytes.subarray(0, n))
    }
  })
})

describe('layer encodings', () => {
  it('round-trip u8 and u16 layers as raw and deflate', async () => {
    const u8 = Uint8Array.from({ length: 5000 }, (_, i) => Math.floor(i / 700))
    const u16 = Uint16Array.from({ length: 5000 }, (_, i) => (i * 37) % 900)
    for (const enc of ['raw+base64', 'deflate+base64'] as const) {
      expect(await decodeU8(await encodeU8(u8, enc), 5000, 'a')).toEqual(u8)
      expect(await decodeU16(await encodeU16(u16, enc), 5000, 'b')).toEqual(u16)
    }
  })

  it('compress drawn layers well', async () => {
    const layer = await encodeU8(new Uint8Array(512 * 512))
    expect(layer.encoding).toBe('deflate+base64')
    expect(layer.data.length).toBeLessThan((512 * 512) / 50)
  })

  it('refuse data that inflates past the layer size', async () => {
    const layer = await encodeU8(new Uint8Array(10_000))
    await expect(decodeU8(layer, 100, 'cellType')).rejects.toThrow(/cellType: inflates to more/)
    await expect(inflateRaw(base64Decode(layer.data), 10_000)).resolves.toHaveLength(10_000)
  })

  it('decode the sample cfd-core tests (same bytes in both directions)', async () => {
    const layer = { encoding: 'deflate+base64', dtype: 'u8', data: 'Y2eHAU4wAAA=' } as const
    expect(Array.from(await decodeU8(layer, 16, 'x'))).toEqual([
      ...Array(10).fill(7),
      ...Array(6).fill(9),
    ])
  })
})

describe('scene files', () => {
  const names = readdirSync(repo('scenes')).map((f) => f.replace(/\.json$/, ''))

  it.each(names)('%s round-trips: file text → document → file text', async (name) => {
    const original = example(name)
    const doc = await fromScene(original)
    const scene = await toScene(doc)
    expect(scene.layers.cellType.encoding).toBe('deflate+base64')
    const { layers: _l1, ...table } = scene
    const { layers: _l2, ...originalTable } = original
    expect(table).toEqual(originalTable)
    const back = await parseScene(JSON.stringify(scene))
    expect(back.layers.cellType).toEqual(doc.layers.cellType)
    expect(back.layers.elementId).toEqual(doc.layers.elementId)
  })

  it('rejects files that are not scenes, with a readable reason', async () => {
    await expect(parseScene('{ nope')).rejects.toThrow(/not a JSON file/)
    await expect(parseScene('{"name": "x"}')).rejects.toThrow(/not a cfd-flux scene: .*required/)
    const bad = { ...example('channel'), grid: { ...example('channel').grid, colour: 'red' } }
    await expect(parseScene(JSON.stringify(bad))).rejects.toThrow(/not a cfd-flux scene/)
  })

  it('names files after the scene', () => {
    expect(fileNameFor('Cylinder Re=100')).toBe('Cylinder Re=100.cfdscene')
    expect(fileNameFor('a/b: c?')).toBe('a_b_ c_.cfdscene')
    expect(fileNameFor('  ')).toBe('scene.cfdscene')
  })

  // Contract with Rust (design proposal §10): this file is written by the editor's code and read
  // by `cfd-core`'s test `editor_files_decode_in_rust`. It is regenerated only when missing or
  // with UPDATE_FIXTURES=1, so its compressed bytes need not be reproducible across zlib versions.
  it('the editor-written fixture holds the example cylinder', async () => {
    const path = repo('apps/web/tests/fixtures/cylinder-re100.cfdscene')
    if (process.env.UPDATE_FIXTURES || !existsSync(path)) {
      mkdirSync(repo('apps/web/tests/fixtures'), { recursive: true })
      const scene = await toScene(await fromScene(example('cylinder-re100')))
      writeFileSync(path, JSON.stringify(scene, null, 2) + '\n')
    }
    const fixture = JSON.parse(readFileSync(path, 'utf8')) as Scene
    expect(fixture.layers.elementId.encoding).toBe('deflate+base64')
    const a = await fromScene(fixture)
    const b = await fromScene(example('cylinder-re100'))
    expect(a.layers.cellType).toEqual(b.layers.cellType)
    expect(a.layers.elementId).toEqual(b.layers.elementId)
  })
})

describe('imageMask', () => {
  /** A w × h RGBA image from rows of '#' (black), '.' (white), ' ' (transparent), top row first. */
  function image(rows: string[]): [Uint8ClampedArray, number, number] {
    const h = rows.length
    const w = rows[0].length
    const data = new Uint8ClampedArray(w * h * 4)
    rows.forEach((r, y) =>
      [...r].forEach((c, x) => {
        const v = c === '#' ? 0 : 255
        data.set([v, v, v, c === ' ' ? 0 : 255], (y * w + x) * 4)
      }),
    )
    return [data, w, h]
  }
  const picture = (m: { width: number; height: number; cells: Uint8Array }) => {
    const rows: string[] = []
    for (let y = m.height - 1; y >= 0; y--) {
      let r = ''
      for (let x = 0; x < m.width; x++) r += m.cells[y * m.width + x] ? '#' : '.'
      rows.push(r)
    }
    return rows
  }
  const opts = { mode: 'dark' as const, threshold: 0.5, invert: false, width: 4 }

  it('maps dark pixels to cells, top image row to the top cell row', () => {
    const img = image(['#...', '.#..', '..#.'])
    expect(picture(imageMask(...img, opts))).toEqual(['#...', '.#..', '..#.'])
    expect(picture(imageMask(...img, { ...opts, invert: true }))).toEqual(['.###', '#.##', '##.#'])
  })

  it('uses opacity in alpha mode and treats transparency as background', () => {
    const img = image(['  ..', '##  '])
    expect(picture(imageMask(...img, { ...opts, mode: 'alpha' }))).toEqual(['..##', '##..'])
    expect(picture(imageMask(...img, opts))).toEqual(['....', '##..'])
  })

  it('resamples to the requested width, keeping the aspect ratio', () => {
    const img = image(['##..', '##..', '....', '....'])
    const m = imageMask(...img, { ...opts, width: 2 })
    expect([m.width, m.height]).toEqual([2, 2])
    expect(picture(m)).toEqual(['#.', '..'])
    expect(CellType.Solid).toBe(1) // the mask is painted with the palette's paint, not a type
  })
})
