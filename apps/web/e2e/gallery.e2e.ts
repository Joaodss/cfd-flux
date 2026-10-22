// The example gallery against a real cfd-server (ADR-009): "Open with results" runs an example (its
// quick variant on a CPU-only server, like the CI's) and opens the job view, a second click
// reopens the same run, and the heated cavity's statistics have the Nusselt number checked
// against de Vahl Davis (design proposal §7.3).
import { expect, test } from '@playwright/test'

test('Open with results runs the heated cavity once and reads its Nusselt number', async ({
  page,
}) => {
  await page.goto('/')
  const open = async () => {
    await page.getByTestId('open-gallery').click()
    await expect(page.getByTestId('gallery')).toBeVisible()
    await page.getByTestId('results-heated-cavity-ra1e5').click()
    await expect(page).toHaveURL(/#\/jobs\/.+/)
    return page.url()
  }
  const first = await open()
  await expect(page.getByTestId('job-status')).toHaveText('completed', { timeout: 110_000 })

  // The Nusselt number of the hot wall, within the suite's 2 % of de Vahl Davis' 4.509.
  await page.getByTestId('job-tab-stats').click()
  await expect(page.getByTestId('stats-benchmark')).toHaveValue('deVahlDavis1e5')
  const check = page.getByTestId('nu-Hot_wall-check')
  await expect(check).toHaveText(/^[+-]\d\.\d % ✓$/)
  const hot = Number(
    (await page.getByTestId('nu-Hot_wall').locator('td').nth(1).textContent())!.trim(),
  )
  expect(hot).toBeGreaterThan(4.4)
  expect(hot).toBeLessThan(4.6)
  // The cold wall takes in what the hot one gives.
  const cold = Number(
    (await page.getByTestId('nu-Cold_wall').locator('td').nth(1).textContent())!.trim(),
  )
  expect(Math.abs(cold + hot) / hot).toBeLessThan(0.02)

  // The same example again opens the same run: nothing is computed twice.
  await page.goto('/')
  expect(await open()).toBe(first)
})
