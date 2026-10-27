// Decoding of LFFR v2 frames (03 §6.1; written by `cfd-io`'s `frame.rs`), with no DOM
// dependency so the decode worker and the tests (Node) share it:
//
//   magic "LFFR" | version u16 = 2 | header length u32 | JSON header | payload
//   payload, codec "shuffle+lz4": per field, in header order: u32 length L | L bytes of one LZ4
//   block of the byte-shuffled field bytes; codec "none": the field bytes one after the other.
//
// Field bytes are `width × height × components` little-endian values (f16 bits or f32), row-major
// from the bottom-left corner, components interleaved.

export type Dtype = 'f16' | 'f32'
export type Codec = 'shuffle+lz4' | 'zstd' | 'none'

export interface FieldDesc {
  name: string
  components: number
  dtype: Dtype
}

/** The JSON header of a frame (`FrameHeader` in Rust). */
export interface FrameHeader {
  jobId: string
  /** Output index (a snapshot repeats the last one). */
  frameIndex: number
  step: number
  /** Simulated time (s). */
  simTime: number
  width: number
  height: number
  codec: Codec
  fields: FieldDesc[]
}

/** A decoded field: f16 values stay as their bits (`Uint16Array`), f32 ones are numbers. */
export type DecodedField =
  | { name: string; components: number; dtype: 'f16'; data: Uint16Array }
  | { name: string; components: number; dtype: 'f32'; data: Float32Array }

export interface DecodedFrame {
  header: FrameHeader
  fields: DecodedField[]
}

const MAGIC = [0x4c, 0x46, 0x46, 0x52] // "LFFR"
const PREFIX = 10

/** A frame that cannot be read; the message says why. */
export class FrameError extends Error {}

/** Reads the prefix and the JSON header; returns the header and where the payload starts. */
export function parseHeader(bytes: Uint8Array): { header: FrameHeader; payload: number } {
  if (bytes.length < PREFIX || MAGIC.some((m, i) => bytes[i] !== m)) {
    throw new FrameError('not a frame (bad magic)')
  }
  // DataView reads little-endian integers at any offset (like BinaryPrimitives in .NET).
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  const version = view.getUint16(4, true)
  if (version === 1) throw new FrameError('LFFR v1 frames (zstd) are not supported by the viewer')
  if (version !== 2) throw new FrameError(`unsupported frame version ${version}`)
  const length = view.getUint32(6, true)
  if (PREFIX + length > bytes.length) throw new FrameError('truncated frame header')
  let header: FrameHeader
  try {
    header = JSON.parse(new TextDecoder().decode(bytes.subarray(PREFIX, PREFIX + length)))
  } catch (e) {
    throw new FrameError(`invalid frame header: ${(e as Error).message}`)
  }
  return { header, payload: PREFIX + length }
}

/**
 * Decodes one raw LZ4 block (no frame header or checksum) into exactly `length` bytes. Each
 * sequence is a token (literal count in the high nibble, match length − 4 in the low one; 15
 * means "more bytes follow", each adding up to 255), the literals, then a 16-bit offset back into
 * the output and the match; the last sequence has literals only. Every read and write is checked,
 * so a corrupt block throws instead of producing garbage.
 */
/** Copies shorter than this are byte loops (measured on 1024² f16 frames). */
const SHORT = 32

export function decodeLz4Block(src: Uint8Array, length: number): Uint8Array {
  const dst = new Uint8Array(length)
  let s = 0
  let d = 0
  const end = src.length
  const extend = (n: number): number => {
    if (n !== 15) return n
    let b: number
    do {
      if (s >= end) throw new FrameError('truncated LZ4 block')
      b = src[s++]
      n += b
    } while (b === 255)
    return n
  }
  while (s < end) {
    const token = src[s++]
    const literals = extend(token >> 4)
    if (s + literals > end || d + literals > length) throw new FrameError('corrupt LZ4 literals')
    // Short runs (most of them in shuffled fields) byte by byte: a `subarray` per sequence costs
    // more than the copy.
    if (literals < SHORT) for (let k = 0; k < literals; k++) dst[d++] = src[s++]
    else {
      dst.set(src.subarray(s, s + literals), d)
      s += literals
      d += literals
    }
    if (s === end) break // the last sequence: literals only
    if (s + 2 > end) throw new FrameError('truncated LZ4 block')
    const offset = src[s] | (src[s + 1] << 8)
    s += 2
    const match = extend(token & 15) + 4
    if (offset === 0 || offset > d || d + match > length) {
      throw new FrameError('corrupt LZ4 match')
    }
    let from = d - offset
    if (offset >= match && match >= SHORT) {
      dst.copyWithin(d, from, from + match)
      d += match
    } else {
      // Short, or overlapping (e.g. offset 1 repeats a byte): byte by byte, reading what was just
      // written.
      for (let k = 0; k < match; k++) dst[d++] = dst[from++]
    }
  }
  if (d !== length) throw new FrameError(`LZ4 block has ${d} bytes, expected ${length}`)
  return dst
}

/**
 * Inverse of the byte shuffle: the input holds byte 0 of every value, then byte 1 of every value,
 * …; the output has the values' bytes together again (`out[i·s + b] = in[b·m + i]`).
 */
export function unshuffle(src: Uint8Array, valueSize: number): Uint8Array {
  const m = src.length / valueSize
  // f16 and f32: one write per value, the bytes put together in a register (typed arrays use
  // the platform's byte order, little-endian everywhere the viewer runs, as the frames do).
  if (valueSize === 2) {
    const out = new Uint16Array(m)
    for (let i = 0; i < m; i++) out[i] = src[i] | (src[m + i] << 8)
    return new Uint8Array(out.buffer)
  }
  if (valueSize === 4) {
    const out = new Uint32Array(m)
    for (let i = 0; i < m; i++) {
      out[i] = src[i] | (src[m + i] << 8) | (src[2 * m + i] << 16) | (src[3 * m + i] << 24)
    }
    return new Uint8Array(out.buffer)
  }
  const out = new Uint8Array(src.length)
  for (let b = 0; b < valueSize; b++) {
    const plane = b * m
    for (let i = 0; i < m; i++) out[i * valueSize + b] = src[plane + i]
  }
  return out
}

const SIZE: Record<Dtype, number> = { f16: 2, f32: 4 }

/** A field from its bytes (a buffer of its own, offset 0, so typed views are aligned). */
function asField(desc: FieldDesc, bytes: Uint8Array): DecodedField {
  const { name, components } = desc
  return desc.dtype === 'f16'
    ? { name, components, dtype: 'f16', data: new Uint16Array(bytes.buffer, 0, bytes.length / 2) }
    : { name, components, dtype: 'f32', data: new Float32Array(bytes.buffer, 0, bytes.length / 4) }
}

/** Decodes a whole frame (codecs `shuffle+lz4` and `none`). */
export function decodeFrame(input: ArrayBuffer | Uint8Array): DecodedFrame {
  const bytes = input instanceof Uint8Array ? input : new Uint8Array(input)
  const { header, payload } = parseHeader(bytes)
  const cells = header.width * header.height
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  let at = payload
  const fields = header.fields.map((desc) => {
    const size = SIZE[desc.dtype]
    if (!size) throw new FrameError(`unknown dtype ${String(desc.dtype)}`)
    const length = cells * desc.components * size
    switch (header.codec) {
      case 'none': {
        if (at + length > bytes.length) throw new FrameError('truncated frame payload')
        // A copy, so typed views are aligned and independent of the input (not `slice`: Node's
        // Buffer, a Uint8Array subclass, overrides it to share memory).
        const raw = new Uint8Array(length)
        raw.set(bytes.subarray(at, at + length))
        at += length
        return asField(desc, raw)
      }
      case 'shuffle+lz4': {
        if (at + 4 > bytes.length) throw new FrameError('truncated frame payload')
        const blockLength = view.getUint32(at, true)
        at += 4
        if (at + blockLength > bytes.length) throw new FrameError('truncated frame payload')
        const shuffled = decodeLz4Block(bytes.subarray(at, at + blockLength), length)
        at += blockLength
        return asField(desc, unshuffle(shuffled, size))
      }
      default:
        throw new FrameError(`frame codec ${String(header.codec)} is not supported by the viewer`)
    }
  })
  if (at !== bytes.length) throw new FrameError('frame payload has trailing bytes')
  return { header, fields }
}
