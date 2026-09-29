import { describe, expect, it } from 'vitest'
import { escapeHtml, renderSummaryHtml } from './render'
import type { SummaryDoc } from './summary'

const doc = (over: Partial<SummaryDoc> = {}): SummaryDoc => ({
  title: 'Weekly report',
  subtitle: 'Sep 28 – Oct 4',
  total: 2,
  sections: [
    { heading: 'Completed (1)', groups: [{ heading: 'Work', items: [{ title: 'Ship it', done: true, meta: ['Due Sep 29', 'High priority'] }] }] },
    { heading: null, groups: [{ heading: null, items: [{ title: 'Later', done: false, meta: [] }] }] },
  ],
  ...over,
})

describe('escapeHtml', () => {
  it('escapes the five characters that matter', () => {
    expect(escapeHtml(`<a href="x" onclick='y'>&`)).toBe('&lt;a href=&quot;x&quot; onclick=&#39;y&#39;&gt;&amp;')
  })
})

describe('renderSummaryHtml', () => {
  it('lays out headings and a checklist per group', () => {
    const html = renderSummaryHtml(doc())
    expect(html).toContain('<h1>Weekly report</h1><p>Sep 28 – Oct 4</p>')
    expect(html).toContain('<h2>Completed (1)</h2><h3>Work</h3>')
    expect(html).toContain('<li data-checked="true">Ship it <span class="meta">— Due Sep 29 · High priority</span></li>')
    expect(html).toContain('<li data-checked="false">Later</li>')
    expect(html.match(/<ul data-checklist>/g)).toHaveLength(2)
  })

  it('says so when there is nothing', () => {
    expect(renderSummaryHtml(doc({ total: 0, sections: [] }))).toContain('No tasks match these filters.')
  })

  it('never lets task text become markup', () => {
    const evil = doc({ sections: [{ heading: '<b>x</b>', groups: [{ heading: null, items: [{ title: '<img src=x onerror=alert(1)>', done: false, meta: ['<script>'] }] }] }] })
    const html = renderSummaryHtml(evil)
    const host = document.createElement('div')
    host.innerHTML = html
    expect(host.querySelector('img, script, b')).toBeNull()
    expect(host.querySelector('li')!.textContent).toContain('<img src=x onerror=alert(1)>')
  })
})
