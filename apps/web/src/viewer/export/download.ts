// Saving a result to the user's disk: a blob behind a temporary link that is clicked once (the
// browser's download, which is what a <a download> does).

export function download(blob: Blob, name: string): void {
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = name
  document.body.append(a)
  a.click()
  a.remove()
  // The browser starts reading the blob asynchronously: release it a little later.
  setTimeout(() => URL.revokeObjectURL(url), 10_000)
}

export function canvasBlob(canvas: HTMLCanvasElement): Promise<Blob> {
  return new Promise((resolve, reject) =>
    canvas.toBlob((b) => (b ? resolve(b) : reject(new Error('PNG encoding failed'))), 'image/png'),
  )
}

export const textBlob = (text: string, type = 'text/csv'): Blob =>
  new Blob([text], { type: `${type};charset=utf-8` })

/** A file name for something of a job: the job's short id first, so exports sort together. */
export function fileName(jobId: string, what: string, extension: string): string {
  return `${jobId.slice(0, 8)}-${what}.${extension}`
}
