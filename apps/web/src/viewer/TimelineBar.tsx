// The timeline (FR-22): play/pause, step, a slider over the stored frames, speed, loop, and
// "Live" to follow the newest frame while the run goes on. ← / → step one frame.
import { ChevronLeft, ChevronRight, Pause, Play, Radio } from 'lucide-react'
import { useEffect } from 'react'
import { Button } from '../components/ui/button.tsx'
import { formatValue } from '../panels/format.ts'
import { useViewer } from './store.ts'
import { SPEEDS } from './timeline.ts'
import type { Viewer } from './Viewer.ts'

export function TimelineBar(props: { viewer: Viewer | null; running: boolean }) {
  const { viewer, running } = props
  const t = useViewer((s) => s.timeline)
  const header = useViewer((s) => s.frame?.header)
  const setTimeline = useViewer((s) => s.setTimeline)

  useEffect(() => {
    if (!viewer) return
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null
      if (target && ['INPUT', 'SELECT', 'TEXTAREA'].includes(target.tagName)) return
      if (e.key === 'ArrowLeft') viewer.step(-1)
      else if (e.key === 'ArrowRight') viewer.step(1)
      else return
      e.preventDefault()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [viewer])

  const disabled = !viewer || t.last < 0
  const index = t.index ?? Math.max(t.last, 0)
  return (
    <div
      className="flex h-11 shrink-0 items-center gap-2 border-t border-border px-3 text-xs"
      data-testid="timeline"
    >
      {t.mode === 'playing' ? (
        <Button
          variant="ghost"
          size="sm"
          title="Pause playback"
          data-testid="timeline-pause"
          onClick={() => viewer?.pause()}
        >
          <Pause />
        </Button>
      ) : (
        <Button
          variant="ghost"
          size="sm"
          title="Play the stored frames"
          data-testid="timeline-play"
          disabled={disabled}
          onClick={() => viewer?.play()}
        >
          <Play />
        </Button>
      )}
      <Button
        variant="ghost"
        size="sm"
        title="Previous frame (←)"
        disabled={disabled}
        onClick={() => viewer?.step(-1)}
      >
        <ChevronLeft />
      </Button>
      <Button
        variant="ghost"
        size="sm"
        title="Next frame (→)"
        disabled={disabled}
        onClick={() => viewer?.step(1)}
      >
        <ChevronRight />
      </Button>
      <input
        type="range"
        className="min-w-24 flex-1 accent-primary"
        aria-label="Frame"
        data-testid="timeline-slider"
        min={0}
        max={Math.max(t.last, 0)}
        step={1}
        value={index}
        disabled={disabled}
        onChange={(e) => viewer?.seek(Number(e.target.value))}
      />
      <span className="w-40 tabular-nums text-muted-foreground" data-testid="timeline-position">
        {t.last < 0
          ? 'no frames yet'
          : `${index} / ${t.last}${header ? ` · ${formatValue(header.simTime)} s` : ''}`}
        {t.buffering && ' · loading'}
      </span>
      <select
        className="h-7 rounded-md border border-border bg-background px-1 outline-none"
        aria-label="Playback speed"
        value={t.speed}
        onChange={(e) => setTimeline({ speed: Number(e.target.value) })}
      >
        {SPEEDS.map((v) => (
          <option key={v} value={v}>
            {v} fps
          </option>
        ))}
      </select>
      <label className="flex items-center gap-1 text-muted-foreground">
        <input
          type="checkbox"
          checked={t.loop}
          onChange={(e) => setTimeline({ loop: e.target.checked })}
        />
        Loop
      </label>
      {running && (
        <Button
          variant={t.mode === 'live' ? 'default' : 'outline'}
          size="sm"
          title="Follow the newest frame"
          data-testid="timeline-live"
          onClick={() => viewer?.goLive()}
        >
          <Radio /> Live
        </Button>
      )}
      {t.note && <span className="text-amber-500">{t.note}</span>}
    </div>
  )
}
