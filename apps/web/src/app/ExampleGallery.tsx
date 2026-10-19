// The example gallery (FR-08): the scenes of `scenes/` as cards with a thumbnail of their cells.
import { useEffect, useRef } from 'react'
import { Dialog, DialogContent } from '../components/ui/dialog.tsx'
import { coloursFor } from '../editor/colours.ts'
import type { Layers } from '../editor/layers.ts'
import { formatLength } from '../panels/format.ts'
import { EXAMPLES, type Example } from '../scene/examples.ts'
import { fromScene } from '../scene/io.ts'
import { openExample } from './documents.ts'

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
  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange}>
      <DialogContent
        title="Examples"
        description="Validated scenes from the project's test suite; open one to run or change it"
        className="w-[min(48rem,calc(100vw-2rem))]"
      >
        <ul className="grid grid-cols-1 gap-3 overflow-y-auto sm:grid-cols-2" data-testid="gallery">
          {EXAMPLES.map((ex) => {
            const { grid, physics } = ex.scene
            const tags = [
              physics.thermal && 'thermal',
              physics.freeSurface && 'free surface',
              physics.buoyancy === 'boussinesq' && 'buoyancy',
            ].filter(Boolean)
            return (
              <li key={ex.id}>
                <button
                  type="button"
                  data-testid={`example-${ex.id}`}
                  className="flex w-full flex-col gap-1.5 rounded-md border border-border p-2 text-left hover:bg-accent"
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
                  <span className="text-[11px] text-muted-foreground tabular-nums">
                    {grid.width} × {grid.height} cells · {formatLength(grid.width * grid.cellSize)}{' '}
                    × {formatLength(grid.height * grid.cellSize)}
                    {tags.length > 0 && ` · ${tags.join(', ')}`}
                  </span>
                </button>
              </li>
            )
          })}
        </ul>
      </DialogContent>
    </Dialog>
  )
}
