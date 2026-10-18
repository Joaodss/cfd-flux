// The bar above the canvas: undo/redo and the current tool's options (brush, fill).
import { Redo2, Undo2 } from 'lucide-react'
import { useEditor, MAX_BRUSH } from '../editor/store.ts'
import { redo, undo } from '../editor/tools/controller.ts'
import { cn } from '../lib/utils.ts'
import { ToolButton } from './Toolbar.tsx'

function Toggle(props: { on: boolean; onClick: () => void; children: string; title: string }) {
  return (
    <button
      type="button"
      title={props.title}
      aria-pressed={props.on}
      onClick={props.onClick}
      className={cn(
        'h-7 rounded-md px-2 text-xs',
        props.on ? 'bg-accent text-accent-foreground' : 'text-muted-foreground hover:bg-accent/60',
      )}
    >
      {props.children}
    </button>
  )
}

export function ToolOptions() {
  const tool = useEditor((s) => s.tool)
  const brush = useEditor((s) => s.brush)
  const filled = useEditor((s) => s.filled)
  // Re-render when the history changes; the labels are read from the (mutable) history object.
  useEditor((s) => s.historyVersion)
  const floating = useEditor((s) => s.floating)
  const history = useEditor.getState().history
  const { setBrush, setFilled } = useEditor.getState()

  const usesBrush = tool === 'pencil' || tool === 'eraser' || tool === 'line'
  const shape = tool === 'rect' || tool === 'ellipse'
  return (
    <div className="flex h-9 shrink-0 items-center gap-1 border-b border-border px-2 text-xs">
      <ToolButton
        label={`Undo${history.undoLabel ? ` ${history.undoLabel}` : ''} (Ctrl+Z)`}
        disabled={!history.canUndo && !floating}
        onClick={undo}
        testId="undo"
      >
        <Undo2 />
      </ToolButton>
      <ToolButton
        label={`Redo${history.redoLabel ? ` ${history.redoLabel}` : ''} (Ctrl+Shift+Z)`}
        disabled={!history.canRedo}
        onClick={redo}
        testId="redo"
      >
        <Redo2 />
      </ToolButton>
      <div className="mx-2 h-5 w-px bg-border" />
      {(usesBrush || (shape && !filled)) && (
        <label
          className="flex items-center gap-2 text-muted-foreground"
          title="Brush size ([ and ])"
        >
          {shape ? 'Outline' : 'Brush'}
          <input
            type="range"
            min={1}
            max={MAX_BRUSH}
            value={brush.size}
            onChange={(e) => setBrush({ size: Number(e.target.value) })}
            className="w-28 accent-primary"
            aria-label="Brush size"
          />
          <span className="w-12 text-foreground tabular-nums">
            {brush.size} {brush.size === 1 ? 'cell' : 'cells'}
          </span>
        </label>
      )}
      {(tool === 'pencil' || tool === 'eraser') && (
        <>
          <Toggle title="Square brush" on={!brush.round} onClick={() => setBrush({ round: false })}>
            Square
          </Toggle>
          <Toggle title="Round brush" on={brush.round} onClick={() => setBrush({ round: true })}>
            Round
          </Toggle>
        </>
      )}
      {shape && (
        <>
          <Toggle title="Filled shape" on={filled} onClick={() => setFilled(true)}>
            Filled
          </Toggle>
          <Toggle title="Outline only" on={!filled} onClick={() => setFilled(false)}>
            Outline
          </Toggle>
        </>
      )}
      {tool === 'select' && (
        <span className="text-muted-foreground">
          Drag to select · drag inside to move, Alt to copy · Ctrl+C/X/V · Delete · arrows nudge ·
          Enter drops · Esc cancels
        </span>
      )}
    </div>
  )
}
