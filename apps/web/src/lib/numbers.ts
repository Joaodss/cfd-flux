// Parsing and formatting of the numbers typed into the property panels.

/** The shortest text that reads back as the same number (`0.005`, `1.516e-5`, `293.15`). */
export function formatNumber(v: number): string {
  return String(v)
}

export interface NumberRules {
  min?: number
  max?: number
  /** Strictly positive (e.g. viscosity, cell size). */
  positive?: boolean
  integer?: boolean
}

/** Parses a number typed by the user; returns the value or why it is not acceptable. */
export function parseNumber(text: string, rules: NumberRules = {}): number | string {
  const t = text.trim().replace(',', '.')
  if (t === '') return 'enter a number'
  const v = Number(t)
  if (!Number.isFinite(v)) return 'not a number'
  if (rules.integer && !Number.isInteger(v)) return 'must be a whole number'
  if (rules.positive && !(v > 0)) return 'must be positive'
  if (rules.min !== undefined && v < rules.min) return `must be at least ${rules.min}`
  if (rules.max !== undefined && v > rules.max) return `must be at most ${rules.max}`
  return v
}
