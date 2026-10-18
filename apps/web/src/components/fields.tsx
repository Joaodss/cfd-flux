// Form fields of the property panels. Values are committed on Enter or blur, never per keystroke,
// so each edit is one history entry; while a field is not focused it shows the store's value
// (undo updates it without any synchronisation code).
import { useId, useState, type ReactNode } from 'react'
import { formatNumber, parseNumber, type NumberRules } from '../lib/numbers.ts'
import { cn } from '../lib/utils.ts'

const INPUT =
  'h-7 w-full min-w-0 rounded-md border border-border bg-background px-2 text-xs tabular-nums outline-none focus:border-primary'

/** A labelled row: label on the left, control(s) on the right. */
export function Row(props: {
  label: string
  htmlFor?: string
  title?: string
  children: ReactNode
}) {
  return (
    <div className="grid grid-cols-[7.5rem_1fr] items-center gap-2" title={props.title}>
      <label htmlFor={props.htmlFor} className="truncate text-xs text-muted-foreground">
        {props.label}
      </label>
      <div className="flex min-w-0 items-center gap-1">{props.children}</div>
    </div>
  )
}

/** A number input with its unit; commits a valid value on Enter or blur, Escape reverts. */
export function NumberInput(props: {
  value: number
  onCommit: (v: number) => void
  unit?: string
  rules?: NumberRules
  id?: string
  label?: string
  testId?: string
}) {
  const [draft, setDraft] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const commit = () => {
    if (draft === null) return
    const v = parseNumber(draft, props.rules)
    if (typeof v === 'string') {
      setError(v)
      return
    }
    setDraft(null)
    setError(null)
    if (v !== props.value) props.onCommit(v)
  }
  return (
    <span className="flex min-w-0 flex-1 items-center gap-1">
      <input
        id={props.id}
        aria-label={props.label}
        data-testid={props.testId}
        inputMode="decimal"
        className={cn(INPUT, error && 'border-red-500')}
        value={draft ?? formatNumber(props.value)}
        title={error ?? undefined}
        aria-invalid={error !== null}
        onFocus={() => setDraft(formatNumber(props.value))}
        onChange={(e) => {
          setDraft(e.target.value)
          setError(null)
        }}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            commit()
            e.currentTarget.blur()
          } else if (e.key === 'Escape') {
            setDraft(null)
            setError(null)
            e.currentTarget.blur()
          }
        }}
      />
      {props.unit && (
        <span className="w-12 shrink-0 text-[11px] text-muted-foreground">{props.unit}</span>
      )}
    </span>
  )
}

export function NumberField(props: {
  label: string
  value: number
  onCommit: (v: number) => void
  unit?: string
  rules?: NumberRules
  title?: string
  testId?: string
}) {
  const id = useId()
  return (
    <Row label={props.label} htmlFor={id} title={props.title}>
      <NumberInput {...props} id={id} />
    </Row>
  )
}

/** Two numbers (x, y) with one unit, e.g. a velocity or gravity. */
export function VectorField(props: {
  label: string
  value: readonly [number, number]
  onCommit: (v: [number, number]) => void
  unit?: string
  testId?: string
}) {
  const [x, y] = props.value
  return (
    <Row label={props.label}>
      <NumberInput
        value={x}
        label={`${props.label} x`}
        testId={props.testId && `${props.testId}-x`}
        onCommit={(v) => props.onCommit([v, y])}
      />
      <NumberInput
        value={y}
        label={`${props.label} y`}
        testId={props.testId && `${props.testId}-y`}
        unit={props.unit}
        onCommit={(v) => props.onCommit([x, v])}
      />
    </Row>
  )
}

export function TextField(props: {
  label: string
  value: string
  onCommit: (v: string) => void
  multiline?: boolean
  testId?: string
}) {
  const id = useId()
  const [draft, setDraft] = useState<string | null>(null)
  const commit = () => {
    if (draft !== null && draft !== props.value) props.onCommit(draft)
    setDraft(null)
  }
  const common = {
    id,
    'data-testid': props.testId,
    value: draft ?? props.value,
    onFocus: () => setDraft(props.value),
    onBlur: commit,
  }
  return (
    <Row label={props.label} htmlFor={id}>
      {props.multiline ? (
        <textarea
          {...common}
          rows={2}
          className={cn(INPUT, 'h-auto py-1')}
          onChange={(e) => setDraft(e.target.value)}
        />
      ) : (
        <input
          {...common}
          className={INPUT}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') e.currentTarget.blur()
            if (e.key === 'Escape') {
              setDraft(null)
              e.currentTarget.blur()
            }
          }}
        />
      )}
    </Row>
  )
}

/** A native select (keyboard and screen-reader friendly, styled like the inputs). */
export function SelectField<T extends string>(props: {
  label: string
  value: T
  options: readonly (readonly [T, string])[]
  onChange: (v: T) => void
  title?: string
  testId?: string
}) {
  const id = useId()
  return (
    <Row label={props.label} htmlFor={id} title={props.title}>
      <select
        id={id}
        data-testid={props.testId}
        className={cn(INPUT, 'pr-1')}
        value={props.value}
        onChange={(e) => props.onChange(e.target.value as T)}
      >
        {props.options.map(([v, text]) => (
          <option key={v} value={v}>
            {text}
          </option>
        ))}
      </select>
    </Row>
  )
}

export function CheckField(props: {
  label: string
  checked: boolean
  onChange: (v: boolean) => void
  hint?: string
  testId?: string
}) {
  const id = useId()
  return (
    <Row label={props.label} htmlFor={id}>
      <input
        id={id}
        type="checkbox"
        data-testid={props.testId}
        className="size-3.5 accent-primary"
        checked={props.checked}
        onChange={(e) => props.onChange(e.target.checked)}
      />
      {props.hint && <span className="text-[11px] text-muted-foreground">{props.hint}</span>}
    </Row>
  )
}

/** A titled group of fields. */
export function Group(props: { title: string; children: ReactNode; actions?: ReactNode }) {
  return (
    <fieldset className="flex flex-col gap-1.5">
      <legend className="mb-1 flex w-full items-center text-xs font-medium tracking-wide text-muted-foreground uppercase">
        {props.title}
        <span className="ml-auto flex gap-1 normal-case">{props.actions}</span>
      </legend>
      {props.children}
    </fieldset>
  )
}
