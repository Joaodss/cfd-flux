// The viewer's settings on the toolbar (design proposal §3.1, §3.2): the viewer format's longest
// side (another one reconnects the stream and empties the cache) and the cache budget.
import { Settings } from 'lucide-react'
import { Button } from '../components/ui/button.tsx'
import { Popover, PopoverContent, PopoverTrigger } from '../components/ui/popover.tsx'
import { boxFactor } from './mask.ts'
import {
  BUDGETS_MIB,
  MAX_SIZES,
  maxSizeParam,
  useViewerSettings,
  type MaxSize,
} from './settings.ts'
import { useViewer } from './store.ts'

const SELECT =
  'h-7 rounded-md border border-border bg-background px-1 text-xs outline-none focus:border-primary'

export function SettingsPopover() {
  const { maxSize, budget, setMaxSize, setBudget } = useViewerSettings()
  const grid = useViewer((s) => s.grid)
  const k = grid ? boxFactor(grid.width, grid.height, maxSizeParam(maxSize)) : null
  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button variant="ghost" size="sm" title="Viewer settings" data-testid="viewer-settings">
          <Settings />
        </Button>
      </PopoverTrigger>
      <PopoverContent className="flex w-80 flex-col gap-3 p-3 text-xs">
        <label className="flex items-center justify-between gap-2">
          <span>Frame resolution (longest side)</span>
          <select
            className={SELECT}
            data-testid="viewer-max-size"
            value={String(maxSize)}
            onChange={(e) => {
              const v = e.target.value
              setMaxSize((v === 'full' ? 'full' : Number(v)) as MaxSize)
            }}
          >
            {MAX_SIZES.map((m) => (
              <option key={m} value={String(m)}>
                {m === 'full' ? 'Full' : `${m} cells`}
              </option>
            ))}
          </select>
        </label>
        {grid && k !== null && (
          <p className="text-muted-foreground" data-testid="viewer-format">
            {k === 1
              ? `Every cell: ${grid.width} × ${grid.height}.`
              : `${Math.ceil(grid.width / k)} × ${Math.ceil(grid.height / k)} blocks of ${k} × ${k} cells (grid ${grid.width} × ${grid.height}).`}{' '}
            Changing it reconnects to the run and loads the frames again.
          </p>
        )}
        <label className="flex items-center justify-between gap-2">
          <span>Frame cache</span>
          <select
            className={SELECT}
            data-testid="viewer-budget"
            value={budget}
            onChange={(e) => setBudget(Number(e.target.value))}
          >
            {BUDGETS_MIB.map((m) => (
              <option key={m} value={m * 2 ** 20}>
                {m >= 1024 ? `${m / 1024} GiB` : `${m} MiB`}
              </option>
            ))}
          </select>
        </label>
        <p className="text-muted-foreground">
          Decoded frames kept in memory; the frames farthest from the playhead are dropped first and
          fetched again from the server when needed.
        </p>
      </PopoverContent>
    </Popover>
  )
}
