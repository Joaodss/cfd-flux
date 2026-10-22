// Video export (design proposal §8): WebCodecs `VideoEncoder` feeding `mp4-muxer`, one encoded
// frame per output frame (frame-exact), or a `MediaRecorder` WebM when WebCodecs is missing
// (real-time recording: the frame count is then approximate).
import { ArrayBufferTarget, Muxer } from 'mp4-muxer'

export type VideoCodec = 'avc' | 'vp9' | 'av1'

/** Codecs in order of preference, with the WebCodecs string for a frame size. */
export function codecCandidates(
  width: number,
  height: number,
): { codec: VideoCodec; string: string }[] {
  const big = width * height > 2048 * 1152
  return [
    { codec: 'avc', string: big ? 'avc1.640033' : 'avc1.64002A' },
    { codec: 'vp9', string: big ? 'vp09.00.51.08' : 'vp09.00.41.08' },
    { codec: 'av1', string: 'av01.0.08M.08' },
  ]
}

/** What `VideoEncoder.isConfigSupported` is asked (a stand-in in tests). */
export type Support = (config: VideoEncoderConfig) => Promise<boolean>

/** The first candidate the platform can encode at this size, or null. */
export async function chooseCodec(
  width: number,
  height: number,
  fps: number,
  bitrate: number,
  supported: Support,
): Promise<{ codec: VideoCodec; string: string } | null> {
  for (const c of codecCandidates(width, height)) {
    if (await supported({ codec: c.string, width, height, bitrate, framerate: fps })) return c
  }
  return null
}

/** Bits per second for a size and frame rate: about 0.2 bits per pixel, between 1 and 40 Mbit/s. */
export function bitrateFor(width: number, height: number, fps: number): number {
  return Math.round(Math.min(40e6, Math.max(1e6, width * height * fps * 0.2)))
}

export interface VideoJob {
  /** Frame size in pixels, even (H.264 needs it). */
  width: number
  height: number
  fps: number
  /** Number of frames; `draw(i)` shows frame `i` and returns the canvas holding the image. */
  count: number
  draw(i: number): Promise<HTMLCanvasElement>
  onProgress(done: number): void
  signal: AbortSignal
}

export interface VideoResult {
  blob: Blob
  extension: 'mp4' | 'webm'
  /** What was used, for the interface: `H.264 · MP4`, `VP9 · MP4` or `MediaRecorder · WebM`. */
  description: string
}

const NAMES: Record<VideoCodec, string> = { avc: 'H.264', vp9: 'VP9', av1: 'AV1' }

/** Whether the browser has WebCodecs video encoding. */
export function hasWebCodecs(): boolean {
  return typeof VideoEncoder !== 'undefined' && typeof VideoFrame !== 'undefined'
}

export async function encodeVideo(job: VideoJob): Promise<VideoResult> {
  const bitrate = bitrateFor(job.width, job.height, job.fps)
  const codec = hasWebCodecs()
    ? await chooseCodec(job.width, job.height, job.fps, bitrate, async (c) => {
        try {
          return (await VideoEncoder.isConfigSupported(c)).supported === true
        } catch {
          return false
        }
      })
    : null
  return codec ? encodeWebCodecs(job, codec, bitrate) : encodeRecorder(job)
}

async function encodeWebCodecs(
  job: VideoJob,
  chosen: { codec: VideoCodec; string: string },
  bitrate: number,
): Promise<VideoResult> {
  const target = new ArrayBufferTarget()
  const muxer = new Muxer({
    target,
    video: { codec: chosen.codec, width: job.width, height: job.height, frameRate: job.fps },
    fastStart: 'in-memory',
  })
  let failure: Error | null = null
  const encoder = new VideoEncoder({
    output: (chunk, meta) => muxer.addVideoChunk(chunk, meta),
    error: (e) => (failure = e),
  })
  encoder.configure({
    codec: chosen.string,
    width: job.width,
    height: job.height,
    bitrate,
    framerate: job.fps,
  })
  try {
    for (let i = 0; i < job.count; i++) {
      job.signal.throwIfAborted()
      if (failure) throw failure
      const canvas = await job.draw(i)
      const frame = new VideoFrame(canvas, {
        timestamp: Math.round((i * 1e6) / job.fps),
        duration: Math.round(1e6 / job.fps),
      })
      encoder.encode(frame, { keyFrame: i % 60 === 0 })
      frame.close()
      // Let the encoder catch up instead of queueing every frame in memory.
      while (encoder.encodeQueueSize > 6) await new Promise((r) => setTimeout(r, 2))
      job.onProgress(i + 1)
    }
    await encoder.flush()
    if (failure) throw failure
    muxer.finalize()
  } finally {
    if (encoder.state !== 'closed') encoder.close()
  }
  return {
    blob: new Blob([target.buffer], { type: 'video/mp4' }),
    extension: 'mp4',
    description: `${NAMES[chosen.codec]} · MP4`,
  }
}

/** Fallback: record a canvas as it is redrawn, one frame per `1 / fps` s. */
async function encodeRecorder(job: VideoJob): Promise<VideoResult> {
  if (typeof MediaRecorder === 'undefined') {
    throw new Error('This browser can neither encode video (WebCodecs) nor record a canvas')
  }
  const surface = document.createElement('canvas')
  surface.width = job.width
  surface.height = job.height
  const ctx = surface.getContext('2d')!
  const stream = surface.captureStream(0)
  const track = stream.getVideoTracks()[0] as MediaStreamTrack & { requestFrame?: () => void }
  const recorder = new MediaRecorder(stream, {
    videoBitsPerSecond: bitrateFor(job.width, job.height, job.fps),
  })
  const parts: Blob[] = []
  recorder.ondataavailable = (e) => e.data.size > 0 && parts.push(e.data)
  const stopped = new Promise<void>((resolve) => (recorder.onstop = () => resolve()))
  recorder.start()
  try {
    for (let i = 0; i < job.count; i++) {
      job.signal.throwIfAborted()
      const canvas = await job.draw(i)
      ctx.drawImage(canvas, 0, 0, job.width, job.height)
      track.requestFrame?.()
      await new Promise((r) => setTimeout(r, 1000 / job.fps))
      job.onProgress(i + 1)
    }
  } finally {
    if (recorder.state !== 'inactive') recorder.stop()
  }
  await stopped
  return {
    blob: new Blob(parts, { type: 'video/webm' }),
    extension: 'webm',
    description: 'MediaRecorder · WebM (real time, frame count approximate)',
  }
}
