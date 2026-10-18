// Left tool strip. Step 1 has the hand tool and the view controls; drawing tools follow.
import { Hand, Maximize, ZoomIn, ZoomOut } from 'lucide-react'
import type { ReactNode } from 'react'
import { Button } from '../components/ui/button.tsx'
import { useEditor } from '../editor/store.ts'

function ToolButton(props: {
  label: string
  active?: boolean
  onClick: () => void
  children: ReactNode
}) {
  return (
    <Button
      variant={props.active ? 'active' : 'ghost'}
      size="icon"
      title={props.label}
      aria-label={props.label}
      aria-pressed={props.active}
      onClick={props.onClick}
    >
      {props.children}
    </Button>
  )
}

export function Toolbar() {
  const tool = useEditor((s) => s.tool)
  // Actions never change, so reading them once (without subscribing) is enough.
  const { setTool, zoomStep, fit } = useEditor.getState()
  return (
    <nav className="flex w-11 shrink-0 flex-col items-center gap-1 border-r border-border py-2">
      <ToolButton
        label="Hand (H) — or Space + drag, or the middle button"
        active={tool === 'hand'}
        onClick={() => setTool('hand')}
      >
        <Hand />
      </ToolButton>
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
