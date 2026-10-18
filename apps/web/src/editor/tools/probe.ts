// The probe tool: click to place a measurement point, drag one to move it, Delete to remove the
// selected one. Probes are a list in the scene table, so each action is a table-only history entry.
import type { SceneTable } from '../../scene/io.ts'
import { useEditor } from '../store.ts'
import type { DrawTool } from './tools.ts'

/** The probe at or next to a cell: within `reach` cells on both axes, nearest first. */
export function probeAt(
  probes: SceneTable['probes'],
  [x, y]: [number, number],
  reach: number,
): number | null {
  let best: number | null = null
  let bestD = Infinity
  ;(probes ?? []).forEach((p, i) => {
    const dx = Math.abs(p.position[0] - x)
    const dy = Math.abs(p.position[1] - y)
    if (dx <= reach && dy <= reach && dx + dy < bestD) {
      best = i
      bestD = dx + dy
    }
  })
  return best
}

/** `Probe 3` style name, one past the largest number in use. */
export function nextProbeName(probes: SceneTable['probes']): string {
  let n = 0
  for (const p of probes ?? []) {
    const m = /^Probe (\d+)$/.exec(p.name)
    if (m) n = Math.max(n, Number(m[1]))
  }
  return `Probe ${n + 1}`
}

export class ProbeTool implements DrawTool {
  private dragging: number | null = null
  private before: SceneTable | null = null
  private label = ''

  down(p: [number, number]): void {
    const s = useEditor.getState()
    if (!s.layers.contains(p[0], p[1])) return
    // A probe within ~6 screen pixels (at least the clicked cell) is picked rather than a new one.
    const reach = Math.max(0, Math.floor(6 / s.view.zoom))
    const hit = probeAt(s.table.probes, p, reach)
    this.before = s.table
    if (hit !== null) {
      this.dragging = hit
      this.label = 'Move probe'
    } else {
      const name = nextProbeName(s.table.probes)
      s.editTable((t) => {
        t.probes = [...(t.probes ?? []), { name, position: [p[0], p[1]] }]
      })
      this.dragging = useEditor.getState().table.probes!.length - 1
      this.label = 'Add probe'
    }
    s.selectProbe(this.dragging)
  }

  move(p: [number, number]): void {
    const s = useEditor.getState()
    const i = this.dragging
    if (i === null) return
    const x = Math.min(Math.max(p[0], 0), s.layers.width - 1)
    const y = Math.min(Math.max(p[1], 0), s.layers.height - 1)
    const [px, py] = s.table.probes![i].position
    if (px === x && py === y) return
    s.editTable((t) => {
      t.probes![i].position = [x, y]
    })
  }

  up(p: [number, number]): void {
    this.move(p)
    if (this.before) useEditor.getState().commitTable(this.label, this.before)
    this.dragging = null
    this.before = null
  }

  cancel(): void {
    if (this.before) useEditor.getState().restoreTable(this.before)
    this.dragging = null
    this.before = null
  }

  /** Removes the selected probe (Delete). */
  deleteSelected(): void {
    const s = useEditor.getState()
    const i = s.selectedProbe
    if (i === null || !s.table.probes?.[i]) return
    const before = s.table
    s.editTable((t) => {
      t.probes!.splice(i, 1)
    })
    s.commitTable('Delete probe', before)
    s.selectProbe(null)
  }
}
