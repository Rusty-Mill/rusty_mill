/** Hand `content` to the browser as a file download (Blob + a temporary anchor). */
export function downloadFile(filename: string, mime: string, content: string): void {
  const url = URL.createObjectURL(new Blob([content], { type: `${mime};charset=utf-8` }))
  const a = document.createElement('a')
  a.href = url
  a.download = filename
  a.style.display = 'none'
  document.body.appendChild(a)
  a.click()
  a.remove()
  // Revoke after the click has been handled; some browsers cancel the download otherwise.
  setTimeout(() => URL.revokeObjectURL(url), 1000)
}

export type SaveFormat = 'md' | 'txt' | 'html'
export const SAVE_FORMATS: Record<SaveFormat, { label: string; mime: string }> = {
  md: { label: 'Markdown (.md)', mime: 'text/markdown' },
  txt: { label: 'Plain text (.txt)', mime: 'text/plain' },
  html: { label: 'HTML (.html)', mime: 'text/html' },
}
