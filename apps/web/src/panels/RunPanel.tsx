// The Run tab: simulated time, output, method, precision and backend preference, then the
// estimate from the server's validation (steps, memory, time, Re, Ma, τ).
import { Group, NumberField, Row, SelectField } from '../components/fields.tsx'
import { useEditor } from '../editor/store.ts'
import type { BackendPreference, OutputField, Precision } from '../generated/scene.ts'
import { EstimatePanel } from './EstimatePanel.tsx'

const FIELDS: readonly (readonly [OutputField, string])[] = [
  ['velocity', 'Velocity'],
  ['pressure', 'Pressure'],
  ['vorticity', 'Vorticity'],
  ['density', 'Density'],
  ['temperature', 'Temperature'],
  ['fillFraction', 'Fill fraction'],
]

const update = useEditor.getState().updateTable

export function RunPanel() {
  const run = useEditor((s) => s.table.run)
  const thermal = useEditor((s) => s.table.physics.thermal)
  const freeSurface = useEditor((s) => s.table.physics.freeSurface ?? false)
  const frames = Math.floor(run.endTime / run.outputInterval + 1e-9) + 1
  return (
    <div className="flex flex-col gap-5" data-testid="run-panel">
      <EstimatePanel />
      <Group title="Time">
        <NumberField
          label="End time"
          unit="s"
          value={run.endTime}
          rules={{ positive: true }}
          testId="run-end-time"
          onCommit={(v) => update('End time', (t) => void (t.run.endTime = v))}
        />
        <NumberField
          label="Output interval"
          unit="s"
          value={run.outputInterval}
          rules={{ positive: true }}
          testId="run-output-interval"
          onCommit={(v) => update('Output interval', (t) => void (t.run.outputInterval = v))}
        />
        <Row label="Frames">
          <span className="text-xs tabular-nums">{frames.toLocaleString('en')}</span>
        </Row>
      </Group>
      <Group title="Output fields">
        {FIELDS.map(([field, name]) => {
          const unavailable =
            (field === 'temperature' && !thermal) || (field === 'fillFraction' && !freeSurface)
          return (
            <label key={field} className="flex items-center gap-2 text-xs">
              <input
                type="checkbox"
                className="size-3.5 accent-primary"
                checked={run.outputFields.includes(field)}
                onChange={(e) =>
                  update(`Output ${name.toLowerCase()}`, (t) => {
                    const fields = t.run.outputFields.filter((f) => f !== field)
                    // Keep the canonical order of FIELDS.
                    if (e.target.checked) fields.push(field)
                    t.run.outputFields = FIELDS.map(([f]) => f).filter((f) => fields.includes(f))
                  })
                }
              />
              {name}
              {unavailable && (
                <span className="text-[11px] text-muted-foreground">
                  ({field === 'temperature' ? 'needs temperature' : 'needs a free surface'})
                </span>
              )}
            </label>
          )
        })}
      </Group>
      <Group title="Solver">
        <SelectField
          label="Method"
          value={run.method}
          options={[['lbm', 'Lattice Boltzmann (LBM)']]}
          onChange={(v) => update('Method', (t) => void (t.run.method = v))}
        />
        <SelectField<Precision>
          label="Precision"
          value={run.precision}
          options={[
            ['f32', 'Single (f32)'],
            ['f64', 'Double (f64)'],
          ]}
          onChange={(v) => update('Precision', (t) => void (t.run.precision = v))}
        />
        <SelectField<BackendPreference>
          label="Backend"
          value={run.backend}
          options={[
            ['auto', 'Automatic (CUDA, GPU, CPU)'],
            ['cuda', 'CUDA'],
            ['wgpu', 'GPU (wgpu)'],
            ['cpu', 'CPU'],
          ]}
          testId="run-backend"
          onChange={(v) => update('Backend', (t) => void (t.run.backend = v))}
        />
      </Group>
    </div>
  )
}
