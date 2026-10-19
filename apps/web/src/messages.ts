// The editor's wording of validation issues, keyed by their stable codes (ADR-027) and filled
// from their `params`. English only for now: a translation is another table like this one.
// Codes not listed here fall back to the server's English message.
import type { Issue } from './api/client.ts'

/** What the messages may look up in the document (element names rather than ids). */
export interface MessageContext {
  elementName(id: number): string
  fluidName(id: number): string
}

type Params = NonNullable<Issue['params']>
type Format = (p: Params, c: MessageContext) => string

/** Three significant digits, without trailing zeros. */
export function num(v: unknown): string {
  return typeof v === 'number' ? String(Number(v.toPrecision(3))) : String(v)
}

const element = (p: Params, c: MessageContext) =>
  typeof p.element === 'number' ? c.elementName(p.element) : 'an element'
const fluid = (p: Params, c: MessageContext) =>
  typeof p.fluid === 'number' ? c.fluidName(p.fluid) : 'a fluid'

const CELL_PROBLEMS: Record<string, Format> = {
  noElement: () => 'A wall, inlet, outlet or heat-source cell has no element',
  missingElement: (p) => `A cell belongs to element ${p.element}, which does not exist`,
  kindMismatch: (p, c) => `A cell's type does not match ${element(p, c)}`,
  noFluidNeighbour: (p, c) => `${element(p, c)} has a cell that touches no fluid cell`,
  unexpectedElement: (p, c) => `A fluid or empty cell carries ${element(p, c)}`,
}

const MESSAGES: Record<string, Format> = {
  'scene.version': (p) =>
    `This scene's format version ${p.version} is not supported (${p.expected})`,
  'grid.empty': () => 'The grid needs at least one cell on each side',
  'grid.tooLarge': (p) => `The grid is larger than ${p.max} cells on a side`,
  'grid.cellSize': () => 'The cell size must be a positive length',
  'grid.edges': () => 'Periodic edges must come in opposite pairs',
  'fluid.properties': (p, c) => `${fluid(p, c)} needs a positive density and viscosity`,
  'fluid.thermal': (p, c) =>
    `${fluid(p, c)} needs a conductivity and specific heat when temperature is solved`,
  'fluid.expansion': (p, c) => `${fluid(p, c)} needs an expansion coefficient for buoyancy`,
  'initial.fluid': (p) => `The initial fluid (${p.fluid}) does not exist`,
  'element.fluid': (p, c) => `${element(p, c)} uses fluid ${p.fluid}, which does not exist`,
  'element.material': (p, c) =>
    `${element(p, c)} uses material ${p.material}, which does not exist`,
  'id.zero': (p) => `A ${p.what} has id 0 (ids start at 1)`,
  'id.duplicate': (p) => `Two ${p.what}s share the id ${p.id}`,
  'run.time': () => 'The output interval must be positive and at most the end time',
  'reference.scale': (p) =>
    `The reference ${p.scale === 'temperatureDifference' ? 'temperature difference' : p.scale} must be a positive number`,
  'layer.cellType': () => 'The cell-type layer cannot be read (wrong size or data)',
  'layer.elementId': () => 'The element layer cannot be read (wrong size or data)',
  'layer.fluidId': () => 'The fluid layer cannot be read or uses missing fluids',
  'cell.type': (p) => `A cell has an unknown type (${p.value})`,
  'cell.empty': () => 'Empty (gas) cells need a free surface (Scene → Physics)',
  'cell.element': (p, c) =>
    (CELL_PROBLEMS[String(p.problem)] ?? (() => 'A cell is inconsistent'))(p, c),
  'cell.more': (p) => `…and ${p.count} more cell problems`,
  'domain.noFluid': () => 'There are no fluid cells',
  'domain.noOutlet': () => 'There is an inlet but no outlet: mass will accumulate',
  'domain.noPressureReference': () =>
    'Velocity inlets with only zero-gradient outlets leave the pressure undetermined: use a pressure outlet',
  'probe.position': (p) => `Probe “${p.probe}” is outside the grid`,
  'stability.mach': (p) =>
    Number(p.limit) >= 0.5 // the error limit (the warning starts lower)
      ? `Lattice Mach number ${num(p.mach)} is too high (limit ${num(p.limit)})`
      : `Lattice Mach number ${num(p.mach)} is above ${num(p.limit)}: compressibility errors grow`,
  'stability.tau': (p) =>
    Number(p.limit) <= 0.51 // below the lower limits, not above the upper one
      ? `Viscous relaxation time τ = ${num(p.tau)} is too close to 0.5: refine the grid or slow the flow down`
      : `Viscous relaxation time τ = ${num(p.tau)} is large: accuracy drops; coarsen the grid`,
  'stability.tauThermal': (p) =>
    `Thermal relaxation time τ = ${num(p.tau)} is outside the comfortable range (limit ${num(p.limit)})`,
  'stability.gridRe': (p) =>
    `The grid Reynolds number U·Δx/ν = ${num(p.gridRe)} is high (above ${num(p.limit)}): refine the grid`,
  'support.freeSurface': () => 'Free-surface flows are not supported by the solver yet',
  'support.multiFluid': () => 'Only one fluid per simulation is supported yet',
  'support.precision': () => 'The solver runs in f32 (f64 is not supported yet)',
  'physics.buoyancy': () => 'Boussinesq buoyancy needs temperature: it is ignored',
  'support.slip': (p, c) => `${element(p, c)}: slip walls are not supported yet`,
  'support.inletFluid': (p, c) =>
    `${element(p, c)} brings ${fluid(p, c)}, but the run uses the initial fluid`,
  'support.convective': (p, c) => `${element(p, c)}: convective boundaries are not supported yet`,
  'support.inletFlux': (p, c) =>
    `${element(p, c)}: an inlet takes a fixed temperature or adiabatic, not a flux`,
}

/** The editor's sentence for an issue (the server's message for codes it does not know). */
export function issueText(issue: Issue, c: MessageContext): string {
  const f = MESSAGES[issue.code]
  return f ? f(issue.params ?? {}, c) : issue.message
}

/** Codes this table words itself (tests check every code the solver emits is here). */
export const KNOWN_CODES: readonly string[] = Object.keys(MESSAGES)
