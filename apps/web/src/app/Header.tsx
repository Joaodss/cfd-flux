// Top bar: document actions (New, Examples) and the scene name.
import { ChevronDown, FilePlus, LayoutGrid } from 'lucide-react'
import { Button } from '../components/ui/button.tsx'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '../components/ui/dropdown-menu.tsx'
import { useEditor } from '../editor/store.ts'
import { newDocument } from '../scene/defaults.ts'
import { EXAMPLES } from '../scene/examples.ts'
import { fromScene } from '../scene/io.ts'

export function Header({ onError }: { onError: (message: string) => void }) {
  const name = useEditor((s) => s.table.name)
  const load = useEditor((s) => s.load)

  return (
    <header className="flex h-11 shrink-0 items-center gap-2 border-b border-border px-3">
      <span className="mr-3 font-semibold tracking-tight">cfd-flux</span>
      <Button variant="ghost" size="sm" onClick={() => load(newDocument())}>
        <FilePlus /> New
      </Button>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="sm">
            <LayoutGrid /> Examples <ChevronDown />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start">
          {EXAMPLES.map((ex) => (
            <DropdownMenuItem
              key={ex.id}
              onSelect={() => {
                try {
                  load(fromScene(ex.scene))
                } catch (e) {
                  onError(`${ex.id}: ${(e as Error).message}`)
                }
              }}
            >
              <span>{ex.scene.name}</span>
              <span className="text-xs text-muted-foreground">
                {ex.scene.grid.width} × {ex.scene.grid.height} cells
              </span>
            </DropdownMenuItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>
      <span className="ml-4 truncate text-sm text-muted-foreground" data-testid="scene-name">
        {name}
      </span>
    </header>
  )
}
