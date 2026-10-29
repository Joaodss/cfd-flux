// Video export (design proposal §8): WebCodecs `VideoEncoder` feeding `mp4-muxer`, one encoded
// frame per output frame used (frame-exact), or a `MediaRecorder` WebM when WebCodecs is missing
// (a real-time recording, paused while frames load, so its length is close but not exact).
//
// The video's clock follows the flow's: a speed (real time, × N) and the simulated interval
// between output frames give the frame rate, and frames are skipped when it would pass a cap.
import { ArrayBufferTarget, Muxer } from 'mp4-muxer'
import { noWebCodecs } from '../../perf.ts'

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

/** Bits per pixel and frame: thin lines and particles on a saturated map need a high rate. */
export const QUALITY = { standard: 0.5, high: 2 } as const
export type Quality = keyof typeof QUALITY

/** Bits per second for a size, frame rate and quality, between 2 and 50 Mbit/s. */
export function bitrateFor(width: number, height: number, fps: number, quality: Quality): number {
  return Math.round(Math.min(50e6, Math.max(2e6, width * height * fps * QUALITY[quality])))
}

/** Speeds offered (simulated seconds per second of video; 1 is real time). */
export const SPEEDS = [0.25, 0.5, 1, 2, 5, 10, 30, 100] as const

/**
 * The default pixels per cell of a video of a grid: the largest of 4, 2, 1 that fits a 1920 × 1080
 * picture (a small default size is upscaled, and blurred, by a full-screen player).
 */
export function defaultScale(width: number, height: number): number {
  return [4, 2].find((n) => width * n <= 1920 && height * n <= 1080) ?? 1
}

/** How the output frames become video frames. */
export interface Timing {
  /** Video frames per second (not always a whole number: 1 / 0.03 s is 33.3). */
  fps: number
  /** Every `step`-th output frame is used. */
  step: number
  /** Simulated seconds per second of video, or null when the frames' interval is not known. */
  flowPerSecond: number | null
}

/**
 * The video's frame rate for a speed (1: real time) over frames `interval` simulated seconds
 * apart, at most `maxFps`: past it, frames are skipped (every 2nd, 3rd, …) instead. Without an
 * interval (`null`: no frame after the first has been seen), one frame per output frame at `maxFps`.
 */
export function videoTiming(interval: number | null, speed: number, maxFps: number): Timing {
  if (!(interval !== null && interval > 0)) return { fps: maxFps, step: 1, flowPerSecond: null }
  const ideal = speed / interval
  const step = Math.max(1, Math.ceil(ideal / maxFps - 1e-6))
  // Frame times are rounded (0.02 s comes back as 0.01999…): a rate this close to a whole
  // number is that number.
  const fps = ideal / step
  const whole = Math.round(fps)
  return { fps: Math.abs(fps - whole) < 1e-3 * fps ? whole : fps, step, flowPerSecond: speed }
}

export interface VideoJob {
  /** Frame size in pixels, even (H.264 needs it). */
  width: number
  height: number
  fps: number
  quality: Quality
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

/** Whether the browser has WebCodecs video encoding (`?noWebCodecs` says no, to try the fallback). */
export function hasWebCodecs(): boolean {
  return !noWebCodecs && typeof VideoEncoder !== 'undefined' && typeof VideoFrame !== 'undefined'
}

export async function encodeVideo(job: VideoJob): Promise<VideoResult> {
  const bitrate = bitrateFor(job.width, job.height, job.fps, job.quality)
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
    video: {
      codec: chosen.codec,
      width: job.width,
      height: job.height,
      // The muxer's time scale: a whole frame rate keeps timestamps exact (else its default).
      ...(Number.isInteger(job.fps) ? { frameRate: job.fps } : {}),
    },
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
  // A key frame every 2 s of video (for seeking).
  const keyEvery = Math.max(1, Math.round(2 * job.fps))
  try {
    for (let i = 0; i < job.count; i++) {
      job.signal.throwIfAborted()
      if (failure) throw failure
      const canvas = await job.draw(i)
      const frame = new VideoFrame(canvas, {
        timestamp: Math.round((i * 1e6) / job.fps),
        duration: Math.round(1e6 / job.fps),
      })
      encoder.encode(frame, { keyFrame: i % keyEvery === 0 })
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

/** Drawn frames the recorder may hold ahead of the one it shows (RGBA bytes). */
export const RECORDER_BUFFER = 128 * 2 ** 20

/** Frames held ahead by the recorder at a size: as many as fit in `RECORDER_BUFFER`, at least 2. */
export function recorderFrames(width: number, height: number): number {
  return Math.max(2, Math.floor(RECORDER_BUFFER / (width * height * 4)))
}

/**
 * Fallback: record a canvas as it is redrawn, one frame per `1 / fps` s of recording. Frames are
 * drawn ahead into a buffer (`recorderFrames`) while the recorder runs; when it runs dry (frames
 * load slower than the video plays), the recorder is paused until the buffer is full again. Each
 * pause and resume costs the recording a few milliseconds, so there are a few long pauses rather
 * than one per frame. The time recorded is measured and every wait aims at the frame's due time,
 * so a timer that oversleeps is made up on the next frame.
 */
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
    videoBitsPerSecond: bitrateFor(job.width, job.height, job.fps, job.quality),
  })
  const parts: Blob[] = []
  recorder.ondataavailable = (e) => e.data.size > 0 && parts.push(e.data)
  const stopped = new Promise<void>((resolve) => (recorder.onstop = () => resolve()))

  // The frames drawn ahead, filled by `produce` while the loop below records them.
  const capacity = recorderFrames(job.width, job.height)
  const queue: ImageBitmap[] = []
  let produced = false
  let failure: unknown = null
  let stop = false
  // One waiter on each side: the recorder for a frame, the producer for room.
  let frameAdded: (() => void) | null = null
  let roomMade: (() => void) | null = null
  const wake = (w: (() => void) | null) => w?.()
  const produce = async () => {
    try {
      for (let i = 0; i < job.count && !stop; i++) {
        while (queue.length >= capacity && !stop) await new Promise<void>((r) => (roomMade = r))
        if (stop) break
        queue.push(await createImageBitmap(await job.draw(i)))
        wake(frameAdded)
      }
    } catch (e) {
      failure = e
    } finally {
      produced = true
      wake(frameAdded)
    }
  }
  /** Waits until the buffer is full or every frame has been drawn. */
  const filled = async () => {
    while (queue.length < capacity && !produced) {
      await new Promise<void>((r) => (frameAdded = r))
    }
  }
  const producing = produce()

  // Milliseconds recorded before the current stretch, and when that stretch began.
  let recorded = 0
  let since = 0
  try {
    await filled()
    for (let i = 0; i < job.count; i++) {
      job.signal.throwIfAborted()
      if (queue.length === 0 && !produced) {
        recorder.pause()
        recorded += performance.now() - since
        await filled()
        job.signal.throwIfAborted()
        recorder.resume()
        since = performance.now()
      }
      if (failure) throw failure
      const bitmap = queue.shift()
      if (!bitmap) throw new Error(`Frame ${i} was not drawn`)
      wake(roomMade)
      ctx.drawImage(bitmap, 0, 0, job.width, job.height)
      bitmap.close()
      if (i === 0) {
        recorder.start()
        since = performance.now()
      }
      track.requestFrame?.()
      const due = ((i + 1) * 1000) / job.fps - recorded - (performance.now() - since)
      await new Promise((r) => setTimeout(r, Math.max(0, due)))
      job.onProgress(i + 1)
    }
    // A recording ends where its last frame starts: the same image once more marks where the last
    // frame's time is up, so the video lasts `count / fps`. (Redrawn: a canvas that has not changed
    // gives no frame.) It needs a moment to reach the recorder before it stops.
    ctx.drawImage(surface, 0, 0)
    track.requestFrame?.()
    await new Promise((r) => setTimeout(r, 1000 / job.fps))
  } finally {
    stop = true
    wake(roomMade)
    await producing
    for (const b of queue.splice(0)) b.close()
    if (recorder.state !== 'inactive') recorder.stop()
  }
  await stopped
  return {
    blob: new Blob(parts, { type: 'video/webm' }),
    extension: 'webm',
    description: 'MediaRecorder · WebM (recorded in real time: length approximate)',
  }
}
