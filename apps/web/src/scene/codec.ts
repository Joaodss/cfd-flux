// Layer encoding inside the scene JSON (03 §3): base64 of raw little-endian bytes, optionally
// compressed. The editor writes `deflate+base64` (the browser's native CompressionStream) and reads
// all three encodings (`zstd+base64` with fzstd: the CLI and the examples write it).
//
// Compression streams are asynchronous (they work in chunks without blocking the page), so
// encoding and decoding return Promises — `async`/`await` here works like C#'s Task-based async.
import { decompress } from 'fzstd'
import type { LayerData, LayerEncoding } from '../generated/scene.ts'

const ALPHABET = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'
const LOOKUP = (() => {
  const t = new Int16Array(128).fill(-1)
  for (let i = 0; i < ALPHABET.length; i++) t[ALPHABET.charCodeAt(i)] = i
  return t
})()

/**
 * Decodes standard base64 (with or without `=` padding) into bytes. Hand-written rather than
 * `atob`, which returns a string of one UTF-16 unit per byte: twice the memory for a 48 MB layer.
 */
export function base64Decode(text: string): Uint8Array {
  let end = text.length
  while (end > 0 && text.charCodeAt(end - 1) === 61 /* '=' */) end--
  if (end % 4 === 1) throw new Error('invalid base64 length')
  const out = new Uint8Array(Math.floor((end * 3) / 4))
  let o = 0
  let acc = 0
  let bits = 0
  for (let i = 0; i < end; i++) {
    const c = text.charCodeAt(i)
    const v = c < 128 ? LOOKUP[c] : -1
    if (v < 0) throw new Error(`invalid base64 character at ${i}`)
    acc = (acc << 6) | v
    bits += 6
    if (bits >= 8) {
      bits -= 8
      out[o++] = (acc >> bits) & 0xff
    }
  }
  return out
}

/** Standard base64 with padding, built in chunks (no giant intermediate strings). */
export function base64Encode(bytes: Uint8Array): string {
  const parts: string[] = []
  const CHUNK = 3 * 8192
  const codes = new Uint16Array((CHUNK / 3) * 4)
  for (let start = 0; start < bytes.length; start += CHUNK) {
    const end = Math.min(start + CHUNK, bytes.length)
    let o = 0
    for (let i = start; i < end; i += 3) {
      const n = end - i
      const v = (bytes[i] << 16) | (n > 1 ? bytes[i + 1] << 8 : 0) | (n > 2 ? bytes[i + 2] : 0)
      codes[o++] = ALPHABET.charCodeAt((v >> 18) & 63)
      codes[o++] = ALPHABET.charCodeAt((v >> 12) & 63)
      codes[o++] = n > 1 ? ALPHABET.charCodeAt((v >> 6) & 63) : 61
      codes[o++] = n > 2 ? ALPHABET.charCodeAt(v & 63) : 61
    }
    parts.push(String.fromCharCode(...codes.subarray(0, o)))
  }
  return parts.join('')
}

/** Raw DEFLATE (RFC 1951), as `cfd-core` decodes it. */
export async function deflateRaw(bytes: Uint8Array): Promise<Uint8Array> {
  const stream = new Blob([bytes as BlobPart])
    .stream()
    .pipeThrough(new CompressionStream('deflate-raw'))
  return new Uint8Array(await new Response(stream).arrayBuffer())
}

/** Inflates raw DEFLATE, failing (without reading on) once more than `limit` bytes come out. */
export async function inflateRaw(bytes: Uint8Array, limit: number): Promise<Uint8Array> {
  const stream = new Blob([bytes as BlobPart])
    .stream()
    .pipeThrough(new DecompressionStream('deflate-raw'))
  const reader = stream.getReader()
  const out = new Uint8Array(limit)
  let n = 0
  for (;;) {
    const { done, value } = await reader.read()
    if (done) break
    if (n + value.length > limit) {
      await reader.cancel()
      throw new Error(`inflates to more than the expected ${limit} bytes`)
    }
    out.set(value, n)
    n += value.length
  }
  return out.subarray(0, n)
}

async function payload(layer: LayerData, limit: number): Promise<Uint8Array> {
  const bytes = base64Decode(layer.data)
  switch (layer.encoding) {
    case 'raw+base64':
      return bytes
    case 'zstd+base64':
      return decompress(bytes)
    case 'deflate+base64':
      return inflateRaw(bytes, limit)
  }
}

function checkDtype(layer: LayerData, expected: LayerData['dtype'], name: string): void {
  if (layer.dtype !== expected) {
    throw new Error(`layer ${name}: expected dtype ${expected}, found ${layer.dtype}`)
  }
}

/** Decodes a `u8` layer and checks it has exactly `cells` values. */
export async function decodeU8(layer: LayerData, cells: number, name: string): Promise<Uint8Array> {
  checkDtype(layer, 'u8', name)
  const bytes = await payload(layer, cells).catch((e: Error) => {
    throw new Error(`layer ${name}: ${e.message}`)
  })
  if (bytes.length !== cells) {
    throw new Error(`layer ${name}: expected ${cells} cells, found ${bytes.length}`)
  }
  return bytes
}

/** Decodes a little-endian `u16` layer and checks it has exactly `cells` values. */
export async function decodeU16(
  layer: LayerData,
  cells: number,
  name: string,
): Promise<Uint16Array> {
  checkDtype(layer, 'u16', name)
  const bytes = await payload(layer, cells * 2).catch((e: Error) => {
    throw new Error(`layer ${name}: ${e.message}`)
  })
  if (bytes.length !== cells * 2) {
    throw new Error(`layer ${name}: expected ${cells} cells, found ${bytes.length / 2}`)
  }
  // DataView reads little-endian whatever the machine's byte order is (a Uint16Array view of the
  // bytes would use the machine's order, which is little-endian on every browser platform anyway).
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  const out = new Uint16Array(cells)
  for (let i = 0; i < cells; i++) out[i] = view.getUint16(i * 2, true)
  return out
}

async function encode(
  bytes: Uint8Array,
  dtype: LayerData['dtype'],
  encoding: LayerEncoding,
): Promise<LayerData> {
  if (encoding === 'zstd+base64') throw new Error('the editor cannot write zstd layers')
  const data = encoding === 'deflate+base64' ? await deflateRaw(bytes) : bytes
  return { encoding, dtype, data: base64Encode(data) }
}

export function encodeU8(values: Uint8Array, encoding: LayerEncoding = 'deflate+base64') {
  return encode(values, 'u8', encoding)
}

export function encodeU16(values: Uint16Array, encoding: LayerEncoding = 'deflate+base64') {
  const bytes = new Uint8Array(values.length * 2)
  const view = new DataView(bytes.buffer)
  for (let i = 0; i < values.length; i++) view.setUint16(i * 2, values[i], true)
  return encode(bytes, 'u16', encoding)
}
