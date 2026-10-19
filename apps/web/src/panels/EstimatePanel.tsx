// The Run tab's estimate: what the server says running the scene takes (steps, memory, time,
// engine) and the numbers it derives from it (Re, Ma, τ, Δt…), from the latest validation.
import type { ReactNode } from 'react'
import { currentValidation } from '../app/issues.ts'
import { Group } from '../components/fields.tsx'
import { useEditor } from '../editor/store.ts'
import { formatBytes, formatDuration, formatLength, formatValue } from './format.ts'

const ENGINES = { cuda: 'CUDA', wgpu: 'GPU (wgpu)', cpu: 'CPU' } as const

function Item(props: { label: ReactNode; children: ReactNode; title?: string }) {
  return (
    <>
      <dt className="text-muted-foreground" title={props.title}>
        {props.label}
      </dt>
      <dd className="tabular-nums">{props.children}</dd>
    </>
  )
}

export function EstimatePanel() {
  const validation = useEditor((s) => s.validation)
  const result = currentValidation(validation)
  const e = result?.estimate
  if (validation.status === 'offline') {
    return (
      <Group title="Estimate">
        <p className="text-[11px] text-muted-foreground">
          The estimate comes from the server, which cannot be reached.
        </p>
      </Group>
    )
  }
  if (!e) {
    return (
      <Group title="Estimate">
        <p className="text-[11px] text-muted-foreground">
          {result ? 'Fix the errors listed in the header to get an estimate.' : 'Checking…'}
        </p>
      </Group>
    )
  }
  const n = e.numbers
  const r = n.reference
  const failure = result && !result.ok && result.code !== 'scene.invalid' ? result.message : null
  return (
    <Group title="Estimate">
      {failure && <p className="text-[11px] text-red-400">{failure}</p>}
      <dl className="grid grid-cols-[7.5rem_1fr] gap-x-2 gap-y-0.5 text-xs" data-testid="estimate">
        <Item label="Engine">{e.engine ? ENGINES[e.engine] : 'none can run it'}</Item>
        <Item label="Steps">{e.steps.toLocaleString('en')}</Item>
        <Item label="Frames">{e.frames.toLocaleString('en')}</Item>
        <Item label="Cells">
          {e.cells.toLocaleString('en')} ({e.fluidCells.toLocaleString('en')} fluid)
        </Item>
        <Item label="Memory" title="Solver memory on the device">
          {formatBytes(e.memoryBytes)}
        </Item>
        <Item label="Stored frames" title="Full-resolution f32 frames, before compression">
          {formatBytes(e.storedBytes)}
        </Item>
        <Item label="Time">
          {e.seconds !== undefined
            ? formatDuration(e.seconds)
            : 'known after a first run on this engine'}
        </Item>
        <Item label="Reynolds" title="U·L/ν with the reference scales (Scene tab)">
          <span data-testid="estimate-re">{formatValue(r.reynolds)}</span>
        </Item>
        {r.reynolds !== n.reynolds && (
          <Item
            label="Re (solver scales)"
            title="U·L/ν with the solver's characteristic velocity and length (below)"
          >
            <span data-testid="estimate-re-solver">{formatValue(n.reynolds)}</span>
          </Item>
        )}
        <Item label="Mach (lattice)" title="Keep it below ≈ 0.17 for accuracy">
          <span className={n.mach > 0.17 ? 'text-amber-500' : undefined}>
            {formatValue(n.mach)}
          </span>
        </Item>
        {n.prandtl !== undefined && <Item label="Prandtl">{formatValue(n.prandtl)}</Item>}
        {r.rayleigh !== undefined && <Item label="Rayleigh">{formatValue(r.rayleigh)}</Item>}
        <Item label="τ" title="Relaxation time (viscous; thermal)">
          {formatValue(n.tau)}
          {n.tauThermal !== undefined && `; ${formatValue(n.tauThermal)}`}
        </Item>
        <Item label="Reference U, L" title="Reference velocity and length (Scene tab)">
          {formatValue(r.velocity)} m/s, {formatLength(r.length)}
        </Item>
        <Item label="U, L (solver)" title="Characteristic velocity and length: they set Δt">
          {formatValue(n.velocity)} m/s, {formatLength(n.length)}
        </Item>
        <Item label="Δx, Δt">
          {formatLength(n.dx)}, {formatValue(n.dt)} s
        </Item>
        <Item label="Lattice velocity">{formatValue(n.latticeVelocity)}</Item>
      </dl>
    </Group>
  )
}
