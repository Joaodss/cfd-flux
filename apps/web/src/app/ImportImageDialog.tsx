// "Import image": a PNG (or any image the browser decodes) becomes a mask of cells, previewed
// here, then placed as a floating selection painted with the palette's current paint.
import { useEffect, useRef, useState } from 'react'
import { NumberField, Row, SelectField } from '../components/fields.tsx'
import { Button } from '../components/ui/button.tsx'
import { Dialog, DialogContent } from '../components/ui/dialog.tsx'
import { KIND_NAMES } from '../editor/elements.ts'
import { useEditor } from '../editor/store.ts'
import { floatMask } from '../editor/tools/controller.ts'
import { pickFile } from '../scene/files.ts'
import { imageMask, type Mask, type MaskOptions } from '../scene/mask.ts'

interface Picture {
  name: string
  width: number
  height: number
  data: Uint8ClampedArray
}

/** Decodes an image file into RGBA pixels. */
async function decode(file: File): Promise<Picture> {
  const bitmap = await createImageBitmap(file)
  const canvas = new OffscreenCanvas(bitmap.width, bitmap.height)
  const ctx = canvas.getContext('2d')!
  ctx.drawImage(bitmap, 0, 0)
  const { data } = ctx.getImageData(0, 0, bitmap.width, bitmap.height)
  return { name: file.name, width: bitmap.width, height: bitmap.height, data }
}

function Preview({ mask }: { mask: Mask }) {
  const ref = useRef<HTMLCanvasElement>(null)
  useEffect(() => {
    const c = ref.current
    if (!c) return
    c.width = mask.width
    c.height = mask.height
    const ctx = c.getContext('2d')!
    const img = ctx.createImageData(mask.width, mask.height)
    for (let y = 0; y < mask.height; y++) {
      for (let x = 0; x < mask.width; x++) {
        const on = mask.cells[y * mask.width + x] !== 0
        const v = on ? 40 : 235
        img.data.set([v, v, v, 255], ((mask.height - 1 - y) * mask.width + x) * 4)
      }
    }
    ctx.putImageData(img, 0, 0)
  }, [mask])
  return (
    <canvas
      ref={ref}
      className="max-h-64 w-full rounded-sm border border-border object-contain"
      style={{ imageRendering: 'pixelated' }}
    />
  )
}

function ImportForm({ onDone }: { onDone: () => void }) {
  const gridWidth = useEditor((s) => s.table.grid.width)
  const palette = useEditor((s) => s.palette)
  const [picture, setPicture] = useState<Picture | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [options, setOptions] = useState<MaskOptions>({
    mode: 'dark',
    threshold: 0.5,
    invert: false,
    width: 0,
  })
  const width = options.width || (picture ? Math.min(picture.width, gridWidth) : 1)
  const mask =
    picture && imageMask(picture.data, picture.width, picture.height, { ...options, width })
  const count = mask ? mask.cells.reduce((n, v) => n + v, 0) : 0

  const choose = async () => {
    const file = await pickFile('image/png,image/*')
    if (!file) return
    try {
      setPicture(await decode(file))
      setError(null)
      setOptions((o) => ({ ...o, width: 0 }))
    } catch {
      setError(`${file.name} is not an image the browser can read`)
    }
  }

  return (
    <div className="flex flex-col gap-3">
      <Row label="Image">
        <Button variant="outline" size="sm" onClick={choose} data-testid="import-choose">
          Choose…
        </Button>
        <span className="truncate text-xs text-muted-foreground">
          {picture ? `${picture.name} · ${picture.width} × ${picture.height} px` : 'none'}
        </span>
      </Row>
      {error && <p className="text-xs text-red-400">{error}</p>}
      <SelectField
        label="Cells from"
        value={options.mode}
        options={[
          ['dark', 'Dark pixels'],
          ['alpha', 'Opaque pixels'],
        ]}
        onChange={(mode) => setOptions((o) => ({ ...o, mode }))}
      />
      <Row label="Threshold">
        <input
          type="range"
          min={0.05}
          max={0.95}
          step={0.05}
          value={options.threshold}
          className="flex-1 accent-primary"
          aria-label="Threshold"
          onChange={(e) => setOptions((o) => ({ ...o, threshold: Number(e.target.value) }))}
        />
        <span className="w-10 text-xs tabular-nums">{options.threshold.toFixed(2)}</span>
      </Row>
      <Row label="Invert">
        <input
          type="checkbox"
          className="size-3.5 accent-primary"
          checked={options.invert}
          onChange={(e) => setOptions((o) => ({ ...o, invert: e.target.checked }))}
        />
      </Row>
      <NumberField
        label="Width"
        unit="cells"
        value={width}
        rules={{ integer: true, min: 1, max: 4096 }}
        onCommit={(v) => setOptions((o) => ({ ...o, width: v }))}
      />
      {mask && (
        <>
          <Preview mask={mask} />
          <p className="text-xs text-muted-foreground">
            {mask.width} × {mask.height} cells, {count.toLocaleString('en')} painted as{' '}
            <strong>{KIND_NAMES[palette]}</strong> (the palette's choice). Placed as a selection:
            drag it into place, then Enter.
          </p>
        </>
      )}
      <div className="flex justify-end gap-2">
        <Button
          disabled={!mask || count === 0}
          data-testid="import-place"
          onClick={() => {
            const problem = mask && floatMask(mask)
            if (problem) setError(problem)
            else onDone()
          }}
        >
          Place
        </Button>
      </div>
    </div>
  )
}

export function ImportImageDialog(props: { open: boolean; onOpenChange: (open: boolean) => void }) {
  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange}>
      <DialogContent
        title="Import an image as a mask"
        description="Dark (or opaque) pixels become cells of the palette's current type or element"
      >
        {props.open && <ImportForm onDone={() => props.onOpenChange(false)} />}
      </DialogContent>
    </Dialog>
  )
}
