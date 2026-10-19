// Running the scene (design proposal §9): submit it to the server and open the job's view.
import { ApiError } from '../api/client.ts'
import { jobs } from '../api/jobs.ts'
import { useEditor } from '../editor/store.ts'
import { selectTool } from '../editor/tools/controller.ts'
import { issueText } from '../messages.ts'
import { toScene } from '../scene/io.ts'
import { currentValidation, messageContext } from './issues.ts'
import { navigate } from './route.ts'

/** Submits the scene (after confirming its warnings, if any) and shows the job; false if not. */
export async function runScene(
  confirm: (text: string) => boolean = window.confirm,
): Promise<boolean> {
  selectTool.drop()
  const s = useEditor.getState()
  const v = currentValidation(s.validation)
  if (!v?.ok) {
    s.notify('error', 'The scene has errors: see the list under the header')
    return false
  }
  const warnings = v.issues.filter((i) => i.severity === 'warning')
  if (warnings.length > 0) {
    const ctx = messageContext(s)
    const list = warnings.map((w) => `• ${issueText(w, ctx)}`).join('\n')
    if (
      !confirm(`Run with ${warnings.length === 1 ? 'this warning' : 'these warnings'}?\n\n${list}`)
    ) {
      return false
    }
  }
  try {
    const job = await jobs.submit(
      await toScene({ table: s.table, layers: s.layers, fluidId: s.fluidId }),
    )
    navigate({ name: 'job', id: job.id })
    return true
  } catch (e) {
    const err = e as ApiError
    const detail = err.issues?.length ? `: ${issueText(err.issues[0], messageContext(s))}` : ''
    s.notify('error', `The server did not accept the run (${err.message}${detail})`)
    return false
  }
}
