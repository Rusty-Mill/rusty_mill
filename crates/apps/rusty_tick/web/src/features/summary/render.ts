/**
 * A `SummaryDoc` as the editor's HTML. Every piece of user text goes through
 * `escapeHtml`, so a task titled `<img onerror=…>` shows up as text.
 */
import type { SummaryDoc, SummaryItem } from './summary'

export function escapeHtml(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;').replace(/'/g, '&#39;')
}

const itemHtml = (i: SummaryItem): string =>
  `<li data-checked="${i.done}">${escapeHtml(i.title)}${i.meta.length > 0 ? ` <span class="meta">— ${escapeHtml(i.meta.join(' · '))}</span>` : ''}</li>`

/** Title and sections as headings (h1 title, h2 section, h3 group) with checklists under them. */
export function renderSummaryHtml(doc: SummaryDoc): string {
  const out = [`<h1>${escapeHtml(doc.title)}</h1>`, `<p>${escapeHtml(doc.subtitle)}</p>`]
  if (doc.total === 0) out.push('<p>No tasks match these filters.</p>')
  for (const s of doc.sections) {
    if (s.heading) out.push(`<h2>${escapeHtml(s.heading)}</h2>`)
    for (const g of s.groups) {
      if (g.heading) out.push(`<h3>${escapeHtml(g.heading)}</h3>`)
      out.push(`<ul data-checklist>${g.items.map(itemHtml).join('')}</ul>`)
    }
  }
  return out.join('')
}
