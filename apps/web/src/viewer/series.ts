// The run's time series (`forces.csv`, `probes.csv`, `diagnostics.csv` and, for thermal runs, `heat.csv`; 03 §6.2), live and
// afterwards (design proposal §7.1).
//
// A viewer that joins a running job reads the files (complete lines up to some step: the server
// flushes them before every `series` message) and appends the stream's rows after that step.
// Rows that arrive before the file has been read are kept and merged once it has. After the run,
// the files are the whole story.
//
// Values are stored column by column (`data[column][row]`), the layout uPlot draws from, in plain
// arrays that only grow; a version number tells React that something changed (the arrays are
// never part of React state, like the frames).

export const SERIES = ['forces', 'probes', 'diagnostics', 'heat'] as const
export type SeriesName = (typeof SERIES)[number]

/** A value of a row: `null` where the server wrote `NaN` (JSON has no NaN). */
export type Value = number | null

/** One series: its columns (the CSV header) and its values column by column. */
export interface Table {
  columns: string[]
  data: Value[][]
}

/** Rows of the stream's `series` message, per series: `[step, time, …]`. */
export type Rows = Partial<Record<SeriesName, Value[][]>>

/** Column names per series (`hello.series`), `null` until the job has started. */
export type Columns = Record<Exclude<SeriesName, 'heat'>, string[]> & { heat?: string[] }

/**
 * A CSV file of the run: the header and the complete lines (a last line without its newline is
 * still being written, and dropped). `NaN` becomes `null`.
 */
export function parseCsv(text: string): { columns: string[]; rows: Value[][] } | null {
  const end = text.lastIndexOf('\n')
  if (end < 0) return null // not even a complete header yet
  const lines = text.slice(0, end).split('\n')
  const columns = lines[0].trim().split(',')
  const rows: Value[][] = []
  for (let i = 1; i < lines.length; i++) {
    const line = lines[i].trim()
    if (line === '') continue
    rows.push(line.split(',').map((s) => (s === 'NaN' ? null : Number(s))))
  }
  return { columns, rows }
}

export class SeriesBuffer {
  private readonly tables = new Map<SeriesName, Table>()
  /** Stream rows of a series whose file has not been read yet. */
  private readonly pending = new Map<SeriesName, Value[][]>()
  private readonly fileRead = new Set<SeriesName>()
  /** Bumped on every change. */
  version = 0

  table(name: SeriesName): Table | null {
    return this.tables.get(name) ?? null
  }

  /** Whether the file of a series still has to be read. */
  needsFile(name: SeriesName): boolean {
    return !this.fileRead.has(name)
  }

  /** The column names announced by `hello` (rows can then be shown before the file is read). */
  setColumns(columns: Columns): void {
    for (const name of SERIES) {
      if (!this.tables.has(name) && columns[name]?.length) {
        this.tables.set(name, { columns: columns[name], data: columns[name].map(() => []) })
      }
    }
    this.version++
  }

  /**
   * Some stream rows were missed (`resync`): the files are read again, and the rows that arrive
   * meanwhile are appended after them.
   */
  forgetFiles(): void {
    this.fileRead.clear()
    this.pending.clear()
  }

  /** Rows from the stream: appended after the last step held. */
  append(rows: Rows): void {
    for (const name of SERIES) {
      const r = rows[name]
      if (!r?.length) continue
      if (!this.fileRead.has(name)) {
        const p = this.pending.get(name) ?? []
        p.push(...r)
        this.pending.set(name, p)
      }
      const t = this.tables.get(name)
      if (t) appendRows(t, r)
    }
    this.version++
  }

  /**
   * The text of a series' file: its rows, then the stream rows after its last step. A file
   * without a complete header (the job has not started) changes nothing and is read again later.
   */
  setFile(name: SeriesName, text: string): void {
    const parsed = parseCsv(text)
    if (!parsed) return
    const t: Table = { columns: parsed.columns, data: parsed.columns.map(() => []) }
    appendRows(t, parsed.rows)
    appendRows(t, this.pending.get(name) ?? [])
    this.tables.set(name, t)
    this.pending.delete(name)
    this.fileRead.add(name)
    this.version++
  }
}

/** Appends the rows whose step (column 0) is past the table's last one. */
function appendRows(t: Table, rows: Value[][]): void {
  const steps = t.data[0]
  let last = steps.length > 0 ? (steps[steps.length - 1] ?? -Infinity) : -Infinity
  for (const row of rows) {
    const step = row[0] ?? -Infinity
    if (step <= last || row.length !== t.columns.length) continue
    for (let c = 0; c < row.length; c++) t.data[c].push(row[c])
    last = step
  }
}
