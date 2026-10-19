import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'
import { DecodeError, FrameDecoder, type WorkerLike } from '../src/frames/decoder.ts'
import { halfTable, halfToFloat } from '../src/frames/half.ts'
import {
  decodeFrame,
  decodeLz4Block,
  FrameError,
  parseHeader,
  unshuffle,
  type FrameHeader,
} from '../src/frames/lffr.ts'
import { createHandler, transferables, type Response } from '../src/frames/protocol.ts'

const fixture = (name: string) =>
  readFileSync(fileURLToPath(new URL(`./fixtures/frames/${name}`, import.meta.url)))

describe('decodeLz4Block', () => {
  const bytes = (...b: number[]) => Uint8Array.from(b)

  it('copies a literal-only block', () => {
    expect(decodeLz4Block(bytes(0x30, 1, 2, 3), 3)).toEqual(bytes(1, 2, 3))
  })

  it('repeats bytes with overlapping matches (offset shorter than the match)', () => {
    // 'a', then a match of 4 + 3 = 7 at offset 1 → eight 'a's; then 2 literal bytes.
    const block = bytes(0x13, 97, 1, 0, 0x20, 98, 99)
    expect(new TextDecoder().decode(decodeLz4Block(block, 10))).toBe('aaaaaaaabc')
  })

  it('reads length continuation bytes (255 = more follows)', () => {
    // 15 + 255 + 10 = 280 literals, then a match of 4 + 15 + 255 + 0 = 274 at offset 280.
    const lit = Array.from({ length: 280 }, (_, i) => i % 251)
    const block = bytes(0xff, 255, 10, ...lit, 0x18, 0x01, 255, 0, 0x00)
    const out = decodeLz4Block(block, 280 + 274)
    expect(Array.from(out.subarray(0, 280))).toEqual(lit)
    expect(Array.from(out.subarray(280))).toEqual(lit.slice(0, 274))
  })

  it('rejects truncated and corrupt blocks', () => {
    expect(() => decodeLz4Block(bytes(0x30, 1, 2), 3)).toThrow(FrameError)
    expect(() => decodeLz4Block(bytes(0x10, 1, 5, 0, 0x00), 6)).toThrow(/match/) // offset > output
    expect(() => decodeLz4Block(bytes(0x10, 1, 0, 0, 0x00), 6)).toThrow(/match/) // offset 0
    expect(() => decodeLz4Block(bytes(0x30, 1, 2, 3), 4)).toThrow(/expected 4/) // too short
    expect(() => decodeLz4Block(bytes(0x30, 1, 2, 3), 2)).toThrow(FrameError) // too long
  })
})

describe('unshuffle', () => {
  it('inverts the byte shuffle of cfd-io', () => {
    const values = Uint8Array.from({ length: 4 * 5 }, (_, i) => (i * 37) % 256)
    const shuffled = new Uint8Array(values.length)
    for (let i = 0; i < 5; i++) for (let b = 0; b < 4; b++) shuffled[b * 5 + i] = values[i * 4 + b]
    expect(unshuffle(shuffled, 4)).toEqual(values)
  })
})

describe('half floats', () => {
  it('converts the special and ordinary values', () => {
    expect(halfToFloat(0x0000)).toBe(0)
    expect(Object.is(halfToFloat(0x8000), -0)).toBe(true)
    expect(halfToFloat(0x3c00)).toBe(1)
    expect(halfToFloat(0xc000)).toBe(-2)
    expect(halfToFloat(0x7bff)).toBe(65504)
    expect(halfToFloat(0x0001)).toBe(2 ** -24) // smallest subnormal
    expect(halfToFloat(0x0400)).toBe(2 ** -14) // smallest normal
    expect(halfToFloat(0x7c00)).toBe(Infinity)
    expect(halfToFloat(0xfc00)).toBe(-Infinity)
    expect(halfToFloat(0x7e00)).toBeNaN()
  })

  it('agrees with the platform on all 65,536 values, where it has f16 support', () => {
    const view = new DataView(new ArrayBuffer(2)) as DataView & {
      getFloat16?: (offset: number, littleEndian?: boolean) => number
    }
    if (!view.getFloat16) return
    const t = halfTable()
    for (let h = 0; h < 65536; h++) {
      view.setUint16(0, h, true)
      const v = view.getFloat16(0, true)
      if (Number.isNaN(v)) expect(t[h]).toBeNaN()
      else expect(Object.is(t[h], v)).toBe(true)
    }
  })
})

// Contract with Rust (design proposal §2.2): the frames and their expected values are written
// by `cfd-io`'s test `browser_fixtures`, which fails when they are out of date.
describe('frames written by cfd-io', () => {
  const manifest = JSON.parse(fixture('frames.json').toString('utf8')) as Record<
    string,
    {
      header: FrameHeader
      fields: { name: string; components: number; dtype: string; bits: number[] }[]
    }
  >

  it.each(Object.keys(manifest))('%s decodes bit for bit', (name) => {
    const expected = manifest[name]
    const frame = decodeFrame(fixture(name))
    expect(frame.header).toEqual(expected.header)
    expect(frame.fields.map((f) => [f.name, f.components, f.dtype])).toEqual(
      expected.fields.map((f) => [f.name, f.components, f.dtype]),
    )
    frame.fields.forEach((f, i) => {
      const bits = f.dtype === 'f16' ? f.data : new Uint32Array(f.data.buffer)
      expect(bits.length).toBe(frame.header.width * frame.header.height * f.components)
      expect(Array.from(bits)).toEqual(expected.fields[i].bits)
    })
  })

  it('covers both codecs and both precisions', () => {
    const headers = Object.values(manifest).map((m) => m.header)
    expect(new Set(headers.map((h) => h.codec))).toEqual(new Set(['shuffle+lz4', 'none']))
    expect(new Set(headers.flatMap((h) => h.fields.map((f) => f.dtype)))).toEqual(
      new Set(['f16', 'f32']),
    )
  })

  it('rejects damaged frames with a reason', () => {
    const good = new Uint8Array(fixture('stream-f16.lffr'))
    expect(() => decodeFrame(good.slice(0, good.length - 3))).toThrow(FrameError)
    const badMagic = good.slice()
    badMagic[0] = 0
    expect(() => parseHeader(badMagic)).toThrow(/magic/)
    const v1 = good.slice()
    v1[4] = 1
    expect(() => parseHeader(v1)).toThrow(/v1/)
    const extra = new Uint8Array(good.length + 1)
    extra.set(good)
    expect(() => decodeFrame(extra)).toThrow(/trailing/)
  })
})

describe('FrameDecoder (worker protocol)', () => {
  /** A stand-in worker that runs the real handler in this thread, asynchronously. */
  function fakeWorker(fetchImpl: typeof fetch): WorkerLike {
    let listener: ((e: MessageEvent<Response>) => void) | null = null
    const handle = createHandler(fetchImpl)
    return {
      postMessage(req) {
        void handle(req).then(
          (r) => r && listener?.({ data: r.response } as MessageEvent<Response>),
        )
      },
      addEventListener(_type, l) {
        listener = l
      },
      terminate() {},
    }
  }
  const bytes = () => {
    const b = fixture('stream-f16.lffr')
    return b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength) as ArrayBuffer
  }

  it('decodes buffers and fetched frames, and reports failures with the status', async () => {
    const fetchImpl = (async (url: string) =>
      url.endsWith('/0')
        ? new globalThis.Response(bytes())
        : new globalThis.Response(
            JSON.stringify({ error: { code: 'notFound', message: 'no frame 9' } }),
            {
              status: 404,
            },
          )) as typeof fetch
    const d = new FrameDecoder(fakeWorker(fetchImpl))
    const [a, b] = await Promise.all([d.decode(bytes()), d.fetch('/frames/0')])
    expect(a.frame.header).toEqual(b.frame.header)
    expect(a.frame.header.frameIndex).toBe(3)
    expect(Object.keys(a.stats)).toEqual(['velocity', 'velocity.x', 'velocity.y', 'pressure'])
    const err = await d.fetch('/frames/9').catch((e: DecodeError) => e)
    expect(err).toBeInstanceOf(DecodeError)
    expect([(err as DecodeError).message, (err as DecodeError).status]).toEqual(['no frame 9', 404])
    await expect(d.decode(new ArrayBuffer(4))).rejects.toThrow(/magic/)
    expect(d.busy).toBe(0)
  })

  it('lists every field buffer for transfer', () => {
    const frame = decodeFrame(fixture('stored-f32.lffr'))
    expect(transferables(frame)).toEqual(frame.fields.map((f) => f.data.buffer))
    expect(new Set(transferables(frame)).size).toBe(frame.fields.length)
  })
})
