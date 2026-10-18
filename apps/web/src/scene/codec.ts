// Layer encoding inside the scene JSON (03 §3): base64 of raw little-endian bytes, optionally
// zstd-compressed. Decoding only for now; the editor's own encoding comes with saving (step 5).
import { decompress } from 'fzstd'
import type { LayerData } from '../generated/scene.ts'

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

function payload(layer: LayerData): Uint8Array {
  const bytes = base64Decode(layer.data)
  switch (layer.encoding) {
    case 'raw+base64':
      return bytes
    case 'zstd+base64':
      return decompress(bytes)
  }
}

function checkDtype(layer: LayerData, expected: LayerData['dtype'], name: string): void {
  if (layer.dtype !== expected) {
    throw new Error(`layer ${name}: expected dtype ${expected}, found ${layer.dtype}`)
  }
}

/** Decodes a `u8` layer and checks it has exactly `cells` values. */
export function decodeU8(layer: LayerData, cells: number, name: string): Uint8Array {
  checkDtype(layer, 'u8', name)
  const bytes = payload(layer)
  if (bytes.length !== cells) {
    throw new Error(`layer ${name}: expected ${cells} cells, found ${bytes.length}`)
  }
  return bytes
}

/** Decodes a little-endian `u16` layer and checks it has exactly `cells` values. */
export function decodeU16(layer: LayerData, cells: number, name: string): Uint16Array {
  checkDtype(layer, 'u16', name)
  const bytes = payload(layer)
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
