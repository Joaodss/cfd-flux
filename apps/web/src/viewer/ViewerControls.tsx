// The viewer's controls (quantity, colour map, range, blocks, fit) and its legend: the colour bar
// with ticks and unit, the frame's time and the value under the pointer.
import { Maximize } from 'lucide-react'
import type { ReactNode } from 'react'
import { NumberInput } from '../components/fields.tsx'
import { Button } from '../components/ui/button.tsx'
import { formatValue } from '../panels/format.ts'
import { COLORMAPS, colormap, cssGradient } from './colormaps.ts'
import { RANGE_MODES, ticks, type RangeMode } from './ranges.ts'
import { colormapOf, currentQuantity, currentRange, modeOf, useViewer } from './store.ts'

const SELECT =
  'h-7 rounded-md border border-border bg-background px-1 text-xs outline-none focus:border-primary'

function Labelled(props: { label: string; children: ReactNode }) {
  return (
    <label className="flex items-center gap-1.5 text-xs text-muted-foreground">
      {props.label}
      {props.children}
    </label>
  )
}

export function ViewerToolbar() {
  const s = useViewer()
  const q = currentQuantity(s)
  if (!q) {
    return (
      <div className="flex h-10 shrink-0 items-center border-b border-border px-3 text-xs text-muted-foreground">
        Waiting for the first frame…
      </div>
    )
  }
  const mode = modeOf(s, q)
  const range = currentRange(s) ?? [0, 1]
  return (
    <div className="flex h-10 shrink-0 items-center gap-4 border-b border-border px-3">
      <Labelled label="Field">
        <select
          className={SELECT}
          data-testid="viewer-quantity"
          value={q.key}
          onChange={(e) => s.select(e.target.value)}
        >
          {s.quantities.map((x) => (
            <option key={x.key} value={x.key}>
              {x.label}
            </option>
          ))}
        </select>
      </Labelled>
      <Labelled label="Colours">
        <select
          className={SELECT}
          data-testid="viewer-colormap"
          value={colormapOf(s, q)}
          onChange={(e) => s.setColormap(q.key, e.target.value)}
        >
          {COLORMAPS.map((m) => (
            <option key={m.name} value={m.name}>
              {m.name}
            </option>
          ))}
        </select>
      </Labelled>
      <Labelled label="Range">
        <select
          className={SELECT}
          data-testid="viewer-range-mode"
          value={mode}
          onChange={(e) => s.setMode(q.key, e.target.value as RangeMode)}
        >
          {RANGE_MODES.map(([m, text]) => (
            <option key={m} value={m}>
              {text}
            </option>
          ))}
        </select>
      </Labelled>
      {mode === 'manual' && (
        <span className="flex w-56 items-center gap-1">
          <NumberInput
            label="Minimum"
            testId="viewer-range-min"
            value={range[0]}
            onCommit={(v) => s.setManual(q.key, [v, range[1]])}
          />
          <NumberInput
            label="Maximum"
            testId="viewer-range-max"
            value={range[1]}
            onCommit={(v) => s.setManual(q.key, [range[0], v])}
          />
        </span>
      )}
      <label
        className="flex items-center gap-1.5 text-xs text-muted-foreground"
        title="Show each value as a block, without interpolation"
      >
        <input
          type="checkbox"
          checked={s.nearest}
          onChange={(e) => s.setNearest(e.target.checked)}
        />
        Blocks
      </label>
      <Button variant="ghost" size="sm" className="ml-auto" onClick={s.fit} title="Fit the domain">
        <Maximize /> Fit
      </Button>
    </div>
  )
}

export function Legend() {
  const s = useViewer()
  const q = currentQuantity(s)
  const range = currentRange(s)
  const header = s.frame?.header
  const stats = q ? s.frame?.stats[q.key] : undefined
  return (
    <div className="flex h-12 shrink-0 items-center gap-4 border-t border-border px-3 text-xs tabular-nums">
      {q && range ? (
        <div className="flex w-80 flex-col gap-0.5" data-testid="viewer-legend">
          <div
            className="h-3 rounded-sm"
            style={{ background: cssGradient(colormap(colormapOf(s, q))) }}
          />
          <div className="flex justify-between text-[11px] text-muted-foreground">
            {ticks(range).map((t, i) => (
              <span key={i}>{formatValue(t)}</span>
            ))}
          </div>
        </div>
      ) : (
        <div className="w-80" />
      )}
      <span className="text-muted-foreground">{q?.unit}</span>
      {header && (
        <span data-testid="viewer-frame">
          t = {formatValue(header.simTime)} s · frame {header.frameIndex} · step{' '}
          {header.step.toLocaleString('en')}
        </span>
      )}
      {stats && stats.nonFinite > 0 && (
        <span className="text-amber-500" title="Values beyond the f16 range are drawn in magenta">
          {stats.nonFinite} values out of range
        </span>
      )}
      <span className="ml-auto text-muted-foreground" data-testid="viewer-cursor">
        {s.cursor
          ? `(${s.cursor.cell[0]}, ${s.cursor.cell[1]})${
              s.cursor.value !== null ? ` · ${formatValue(s.cursor.value)} ${q?.unit ?? ''}` : ''
            }`
          : ''}
      </span>
    </div>
  )
}
