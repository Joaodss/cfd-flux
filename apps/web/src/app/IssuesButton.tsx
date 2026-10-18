// The validation status in the header, and the list of issues behind it. Clicking an issue shows
// its cell on the canvas, or selects its element.
import { CircleAlert, CircleCheck, LoaderCircle, TriangleAlert, Unplug } from 'lucide-react'
import type { Issue } from '../api/client.ts'
import { Button } from '../components/ui/button.tsx'
import { Popover, PopoverContent, PopoverTrigger } from '../components/ui/popover.tsx'
import { useEditor } from '../editor/store.ts'
import { cn } from '../lib/utils.ts'
import { issueText } from '../messages.ts'
import { currentValidation, messageContext, shownIssues } from './issues.ts'

function Summary({ issues, ok }: { issues: Issue[]; ok: boolean }) {
  const errors = issues.filter((i) => i.severity === 'error').length
  const warnings = issues.length - errors
  if (!ok || errors > 0) {
    return (
      <>
        <CircleAlert className="text-red-500" />
        {errors || 1} {errors === 1 || errors === 0 ? 'error' : 'errors'}
        {warnings > 0 && `, ${warnings} ${warnings === 1 ? 'warning' : 'warnings'}`}
      </>
    )
  }
  if (warnings > 0) {
    return (
      <>
        <TriangleAlert className="text-amber-500" />
        Ready · {warnings} {warnings === 1 ? 'warning' : 'warnings'}
      </>
    )
  }
  return (
    <>
      <CircleCheck className="text-green-500" /> Ready to run
    </>
  )
}

export function IssuesButton() {
  const validation = useEditor((s) => s.validation)
  const table = useEditor((s) => s.table)
  const ctx = messageContext({ table })
  const result = currentValidation(validation)
  const issues = result ? shownIssues(result) : []
  const pending = validation.status === 'pending' || validation.status === 'idle'

  const go = (i: Issue) => {
    const s = useEditor.getState()
    if (i.at) s.showCell(i.at)
    const el = i.params?.element
    if (typeof el === 'number' && s.table.elements.some((e) => e.id === el)) s.selectElement(el)
  }

  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button variant="ghost" size="sm" data-testid="issues" className="ml-auto">
          {validation.status === 'offline' ? (
            <>
              <Unplug className="text-muted-foreground" /> Server offline
            </>
          ) : result ? (
            <Summary issues={issues} ok={result.ok} />
          ) : (
            <>
              <LoaderCircle className="animate-spin" /> Checking…
            </>
          )}
          {pending && result && <LoaderCircle className="animate-spin text-muted-foreground" />}
        </Button>
      </PopoverTrigger>
      <PopoverContent>
        {validation.status === 'offline' && (
          <p className="p-1 text-sm text-muted-foreground">
            {validation.message}. Start it with <code>cargo run --release -p cfd-server</code>.
          </p>
        )}
        {result && issues.length === 0 && (
          <p className="p-1 text-sm">No issues: the scene can run.</p>
        )}
        {issues.length > 0 && (
          <ul className="flex max-h-80 flex-col overflow-y-auto" data-testid="issue-list">
            {issues.map((i, n) => (
              <li key={n}>
                <button
                  type="button"
                  onClick={() => go(i)}
                  title={i.message}
                  className={cn(
                    'flex w-full items-start gap-2 rounded-md px-2 py-1.5 text-left text-xs hover:bg-accent [&_svg]:mt-0.5 [&_svg]:size-3.5 [&_svg]:shrink-0',
                  )}
                >
                  {i.severity === 'error' ? (
                    <CircleAlert className="text-red-500" />
                  ) : (
                    <TriangleAlert className="text-amber-500" />
                  )}
                  <span className="flex-1">{issueText(i, ctx)}</span>
                  {i.at && (
                    <span className="text-muted-foreground tabular-nums">
                      ({i.at[0]}, {i.at[1]})
                    </span>
                  )}
                </button>
              </li>
            ))}
          </ul>
        )}
      </PopoverContent>
    </Popover>
  )
}
