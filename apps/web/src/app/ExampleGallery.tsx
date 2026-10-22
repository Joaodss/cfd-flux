// The example gallery (FR-08): the scenes of `scenes/` as cards with a thumbnail of their cells.
import { useEffect, useRef, useState } from 'react'
import { api, type Capabilities } from '../api/client.ts'
import { Button } from '../components/ui/button.tsx'
import { Dialog, DialogContent } from '../components/ui/dialog.tsx'
import { coloursFor } from '../editor/colours.ts'
import type { Layers } from '../editor/layers.ts'
import { formatLength } from '../panels/format.ts'
import { EXAMPLES, type Example } from '../scene/examples.ts'
import { fromScene } from '../scene/io.ts'
import { BENCHMARKS, isBenchmark } from '../viewer/references.ts'
import { openExample } from './documents.ts'
import { openExampleWithResults } from './run.ts'

/** Draws the cells into an RGBA image, one pixel per cell (y up → image rows from the top). */
function drawCells(
  canvas: HTMLCanvasElement,
  layers: Layers,
  elements: Example['scene']['elements'],
) {
  const { width, height } = layers
  canvas.width = width
  canvas.height = height
  const ctx = canvas.getContext('2d')
  if (!ctx) return
  const colours = coloursFor(elements)
  const img = ctx.createImageData(width, height)
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const i = layers.index(x, y)
      const c = colours.elements.get(layers.elementId[i]) ?? colours.cellTypes[layers.cellType[i]]
      img.data.set(c, ((height - 1 - y) * width + x) * 4)
    }
  }
  ctx.putImageData(img, 0, 0)
}

function Thumbnail({ example }: { example: Example }) {
  const ref = useRef<HTMLCanvasElement>(null)
  useEffect(() => {
    let cancelled = false
    void fromScene(example.scene).then((doc) => {
      if (!cancelled && ref.current) drawCells(ref.current, doc.layers, example.scene.elements)
    })
    return () => {
      cancelled = true
    }
  }, [example])
  return (
    <canvas
      ref={ref}
      className="h-24 w-full rounded-sm border border-border bg-muted object-contain"
      style={{ imageRendering: 'pixelated' }}
    />
  )
}

export function ExampleGallery(props: { open: boolean; onOpenChange: (open: boolean) => void }) {
  // The quick variants are for a server without a GPU: with the CPU as the best engine, a full
  // example takes minutes (the cylinder ≈ 1 min) and the variant seconds (ADR-009).
  const [caps, setCaps] = useState<Capabilities | null>(null)
  const [quick, setQuick] = useState(true)
  const [starting, setStarting] = useState<string | null>(null)
  useEffect(() => {
    if (!props.open) return
    let cancelled = false
    api
      .capabilities()
      .then((c) => !cancelled && setCaps(c))
      .catch(() => !cancelled && setCaps(null))
    return () => {
      cancelled = true
    }
  }, [props.open])
  const cpuOnly = caps?.auto === 'cpu'
  const useQuick = cpuOnly && quick
  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange}>
      <DialogContent
        title="Examples"
        description="Validated scenes from the project's test suite; open one to run or change it"
        className="w-[min(48rem,calc(100vw-2rem))]"
      >
        {cpuOnly && (
          <label className="flex items-center gap-2 text-xs text-muted-foreground">
            <input
              type="checkbox"
              data-testid="gallery-quick"
              checked={quick}
              onChange={(e) => setQuick(e.target.checked)}
            />
            Quick variants (this server runs on the CPU: a coarser grid and a shorter run, seconds
            instead of minutes)
          </label>
        )}
        <ul className="grid grid-cols-1 gap-3 overflow-y-auto sm:grid-cols-2" data-testid="gallery">
          {EXAMPLES.map((full) => {
            const ex = useQuick && full.quick ? full.quick : full
            const { grid, physics } = ex.scene
            const tags = [
              physics.thermal && 'thermal',
              physics.freeSurface && 'free surface',
              physics.buoyancy === 'boussinesq' && 'buoyancy',
            ].filter(Boolean)
            return (
              <li
                key={full.id}
                className="flex flex-col gap-1.5 rounded-md border border-border p-2"
              >
                <button
                  type="button"
                  data-testid={`example-${full.id}`}
                  className="flex w-full flex-col gap-1.5 rounded-sm text-left hover:bg-accent"
                  onClick={async () => {
                    props.onOpenChange(false)
                    await openExample(ex)
                  }}
                >
                  {props.open && <Thumbnail example={ex} />}
                  <span className="text-sm font-medium">{ex.scene.name}</span>
                  {ex.scene.description && (
                    <span className="line-clamp-2 text-xs text-muted-foreground">
                      {ex.scene.description}
                    </span>
                  )}
                  {isBenchmark(ex.benchmark) && (
                    <span className="text-[11px] text-muted-foreground">
                      Benchmark: {BENCHMARKS[ex.benchmark].title}
                    </span>
                  )}
                  <span className="text-[11px] text-muted-foreground tabular-nums">
                    {grid.width} × {grid.height} cells · {formatLength(grid.width * grid.cellSize)}{' '}
                    × {formatLength(grid.height * grid.cellSize)}
                    {tags.length > 0 && ` · ${tags.join(', ')}`}
                  </span>
                </button>
                {/* The dam break needs the free surface (Phase 6a): the server cannot run it. */}
                <Button
                  variant="outline"
                  size="sm"
                  data-testid={`results-${full.id}`}
                  disabled={physics.freeSurface || starting !== null}
                  title={
                    physics.freeSurface
                      ? 'The free-surface solver comes later'
                      : 'Run it on the server, or reuse its latest completed run, and open the results'
                  }
                  onClick={async () => {
                    setStarting(full.id)
                    const ok = await openExampleWithResults(ex)
                    setStarting(null)
                    if (ok) props.onOpenChange(false)
                  }}
                >
                  {starting === full.id ? 'Starting…' : 'Open with results'}
                </Button>
              </li>
            )
          })}
        </ul>
      </DialogContent>
    </Dialog>
  )
}
