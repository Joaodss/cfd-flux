// Left tool strip: the drawing tools, then the view controls.
import {
  Circle,
  Eraser,
  Hand,
  Maximize,
  PaintBucket,
  Pencil,
  Slash,
  Square,
  SquareDashed,
  ZoomIn,
  ZoomOut,
  type LucideIcon,
} from 'lucide-react'
import type { ReactNode } from 'react'
import { Button } from '../components/ui/button.tsx'
import { useEditor } from '../editor/store.ts'
import { setTool } from '../editor/tools/controller.ts'
import type { ToolId } from '../editor/tools/tools.ts'

const TOOLS: [ToolId, LucideIcon, string][] = [
  ['pencil', Pencil, 'Pencil (B)'],
  ['eraser', Eraser, 'Eraser (E)'],
  ['line', Slash, 'Line (L) — Shift: 0/45/90°'],
  ['rect', Square, 'Rectangle (R) — Shift: square, Alt: from the centre'],
  ['ellipse', Circle, 'Ellipse (O) — Shift: circle, Alt: from the centre'],
  ['bucket', PaintBucket, 'Fill (G)'],
  ['select', SquareDashed, 'Select (M) — drag to move, Alt+drag to copy, Ctrl+C/V, Delete'],
  ['hand', Hand, 'Hand (H) — or Space + drag, or the middle button'],
]

export function ToolButton(props: {
  label: string
  active?: boolean
  disabled?: boolean
  onClick: () => void
  children: ReactNode
  testId?: string
}) {
  return (
    <Button
      variant={props.active ? 'active' : 'ghost'}
      size="icon"
      title={props.label}
      aria-label={props.label}
      aria-pressed={props.active}
      disabled={props.disabled}
      onClick={props.onClick}
      data-testid={props.testId}
    >
      {props.children}
    </Button>
  )
}

export function Toolbar() {
  const tool = useEditor((s) => s.tool)
  // Actions never change, so reading them once (without subscribing) is enough.
  const { zoomStep, fit } = useEditor.getState()
  return (
    <nav className="flex w-11 shrink-0 flex-col items-center gap-1 border-r border-border py-2">
      {TOOLS.map(([id, Icon, label]) => (
        <ToolButton
          key={id}
          label={label}
          active={tool === id}
          onClick={() => setTool(id)}
          testId={`tool-${id}`}
        >
          <Icon />
        </ToolButton>
      ))}
      <div className="my-1 h-px w-6 bg-border" />
      <ToolButton label="Zoom in (+)" onClick={() => zoomStep(1)}>
        <ZoomIn />
      </ToolButton>
      <ToolButton label="Zoom out (−)" onClick={() => zoomStep(-1)}>
        <ZoomOut />
      </ToolButton>
      <ToolButton label="Fit (F)" onClick={fit}>
        <Maximize />
      </ToolButton>
    </nav>
  )
}
