// The Export tab (design proposal §8): PNG of the view or of the whole domain, CSV (the run's own
// files, the force coefficients, the window statistics, the fields of the frame shown) and a video
// of a range of frames with the overlays.
import { useRef, useState } from 'react'
import { NumberInput } from '../components/fields.tsx'
import { Button } from '../components/ui/button.tsx'
import { coefficientsCsv, RUN_FILES, statisticsCsv } from './export/csv.ts'
import { canvasBlob, download, fileName, textBlob } from './export/download.ts'
import { hasWebCodecs } from './export/video.ts'
import { currentQuantity, useViewer } from './store.ts'
import { useElement, useSeries, useStatistics } from './seriesFeed.ts'
import type { Viewer } from './Viewer.ts'

const SELECT = 'h-7 rounded-md border border-border bg-background px-1.5 text-xs'

const SIZES: [string, string][] = [
  ['view', 'The view as shown'],
  ['1', 'Whole domain, 1 px per cell'],
  ['2', 'Whole domain, 2 px per cell'],
  ['4', 'Whole domain, 4 px per cell'],
]

function Section(props: { title: string; children: React.ReactNode }) {
  return (
    <section className="flex flex-col gap-2">
      <h3 className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
        {props.title}
      </h3>
      {props.children}
    </section>
  )
}

export function ExportPanel({ viewer, jobId }: { viewer: Viewer | null; jobId: string }) {
  const last = useViewer((s) => s.timeline.last)
  const index = useViewer((s) => s.timeline.index)
  const quantity = useViewer((s) => currentQuantity(s))
  const [size, setSize] = useState('view')
  const [footer, setFooter] = useState(true)
  const [message, setMessage] = useState<string | null>(null)
  const feed = useSeries((s) => s.feed)
  const { element } = useElement()
  const stats = useStatistics()
  const frame = index ?? 0
  const field = quantity?.key.replace('.', '-') ?? 'field'

  const fail = (e: unknown) => setMessage(e instanceof Error ? e.message : String(e))

  const savePng = async () => {
    if (!viewer) return
    try {
      const canvas = viewer.renderImage(size === 'view' ? null : Number(size), footer)
      const name = fileName(jobId, `${field}-frame${String(frame).padStart(4, '0')}`, 'png')
      download(await canvasBlob(canvas), name)
      setMessage(`Saved ${name} (${canvas.width} × ${canvas.height})`)
    } catch (e) {
      fail(e)
    }
  }

  const saveCoefficients = () => {
    const forces = feed?.buffer.table('forces')
    const meta = feed?.meta
    const csv = forces && meta && element ? coefficientsCsv(forces, element, meta) : null
    if (!csv) return setMessage('No force series for that element yet')
    download(textBlob(csv), fileName(jobId, `coefficients-${element}`, 'csv'))
  }

  const saveStatistics = () => {
    if (!stats || !element) return setMessage('No statistics yet (they need the force series)')
    download(
      textBlob(statisticsCsv(stats, element)),
      fileName(jobId, `statistics-${element}`, 'csv'),
    )
  }

  const saveFrame = () => {
    const csv = viewer?.frameCsv()
    if (!csv) return setMessage('No frame is shown yet')
    download(textBlob(csv), fileName(jobId, `fields-frame${String(frame).padStart(4, '0')}`, 'csv'))
  }

  return (
    <div className="flex flex-col gap-5 text-sm" data-testid="export">
      <Section title="Image (PNG)">
        <div className="flex flex-wrap items-center gap-2">
          <select
            className={SELECT}
            data-testid="export-size"
            value={size}
            onChange={(e) => setSize(e.target.value)}
          >
            {SIZES.map(([v, text]) => (
              <option key={v} value={v}>
                {text}
              </option>
            ))}
          </select>
          <label className="flex items-center gap-1.5 text-xs text-muted-foreground">
            <input
              type="checkbox"
              checked={footer}
              onChange={(e) => setFooter(e.target.checked)}
              data-testid="export-footer"
            />
            Legend and time
          </label>
        </div>
        <Button size="sm" onClick={savePng} disabled={!viewer || last < 0} data-testid="export-png">
          Save PNG
        </Button>
      </Section>
      <Section title="Data (CSV)">
        <ul className="flex flex-col gap-0.5 text-xs">
          {RUN_FILES.map((name) => (
            <li key={name}>
              <a
                className="underline"
                href={`/api/simulations/${encodeURIComponent(jobId)}/files/${name}`}
                download={fileName(jobId, name.replace(/\.[^.]+$/, ''), name.split('.').pop()!)}
              >
                {name}
              </a>
            </li>
          ))}
        </ul>
        <div className="flex flex-wrap gap-2">
          <Button variant="outline" size="sm" onClick={saveCoefficients} data-testid="export-cd">
            C_D, C_L of {element ?? '…'}
          </Button>
          <Button variant="outline" size="sm" onClick={saveStatistics} data-testid="export-stats">
            Window statistics
          </Button>
          <Button variant="outline" size="sm" onClick={saveFrame} data-testid="export-fields">
            Fields of frame {frame}
          </Button>
        </div>
      </Section>
      <VideoSection viewer={viewer} jobId={jobId} last={last} onMessage={setMessage} />
      {message && (
        <p className="text-xs text-muted-foreground" data-testid="export-message">
          {message}
        </p>
      )}
    </div>
  )
}

function VideoSection(props: {
  viewer: Viewer | null
  jobId: string
  last: number
  onMessage: (m: string) => void
}) {
  const { viewer, jobId, last, onMessage } = props
  const [from, setFrom] = useState(0)
  const [to, setTo] = useState<number | null>(null)
  const [fps, setFps] = useState(30)
  const [scale, setScale] = useState(2)
  const [progress, setProgress] = useState<[number, number] | null>(null)
  const abort = useRef<AbortController | null>(null)
  const end = Math.min(to ?? last, last)

  const record = async () => {
    if (!viewer) return
    const controller = new AbortController()
    abort.current = controller
    setProgress([0, end - from + 1])
    try {
      const result = await viewer.exportVideo(
        { from, to: end, fps, scale },
        (done, total) => setProgress([done, total]),
        controller.signal,
      )
      download(result.blob, fileName(jobId, `frames${from}-${end}`, result.extension))
      onMessage(`Saved ${end - from + 1} frames as ${result.description}`)
    } catch (e) {
      onMessage(
        controller.signal.aborted ? 'Recording cancelled' : `Video failed: ${(e as Error).message}`,
      )
    } finally {
      setProgress(null)
      abort.current = null
    }
  }

  return (
    <Section title="Video">
      <div className="grid grid-cols-2 gap-2">
        <label className="flex items-center gap-1.5 text-xs text-muted-foreground">
          From
          <NumberInput
            label="First frame"
            testId="video-from"
            value={from}
            rules={{ min: 0, max: Math.max(0, last), integer: true }}
            onCommit={(v) => setFrom(Math.min(v, end))}
          />
        </label>
        <label className="flex items-center gap-1.5 text-xs text-muted-foreground">
          To
          <NumberInput
            label="Last frame"
            testId="video-to"
            value={end}
            rules={{ min: 0, max: Math.max(0, last), integer: true }}
            onCommit={(v) => setTo(Math.max(v, from))}
          />
        </label>
        <label className="flex items-center gap-1.5 text-xs text-muted-foreground">
          Frames/s
          <NumberInput
            label="Frame rate"
            testId="video-fps"
            value={fps}
            rules={{ min: 1, max: 120, integer: true }}
            onCommit={setFps}
          />
        </label>
        <select
          className={SELECT}
          data-testid="video-size"
          value={scale}
          onChange={(e) => setScale(Number(e.target.value))}
        >
          {[1, 2, 4].map((n) => (
            <option key={n} value={n}>
              Domain, {n} px per cell
            </option>
          ))}
        </select>
      </div>
      <p className="text-xs text-muted-foreground">
        {hasWebCodecs()
          ? 'One video frame per output frame (WebCodecs), with the overlays and the legend.'
          : 'No WebCodecs here: the canvas is recorded in real time as WebM.'}
      </p>
      {progress ? (
        <div className="flex items-center gap-2">
          <div className="h-2 flex-1 overflow-hidden rounded-full bg-muted">
            <div
              className="h-full bg-primary"
              style={{ width: `${(100 * progress[0]) / Math.max(1, progress[1])}%` }}
              data-testid="video-progress"
            />
          </div>
          <span className="text-xs tabular-nums">
            {progress[0]} / {progress[1]}
          </span>
          <Button variant="outline" size="sm" onClick={() => abort.current?.abort()}>
            Cancel
          </Button>
        </div>
      ) : (
        <Button
          size="sm"
          onClick={record}
          disabled={!viewer || last < 0}
          data-testid="video-record"
        >
          Record video
        </Button>
      )}
    </Section>
  )
}
