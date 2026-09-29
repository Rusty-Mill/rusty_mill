import { describe, expect, it } from 'vitest'
import { isEmptyContent, safeHref, toHtmlDocument, toMarkdown, toPlainText } from './exporters'
import { renderSummaryHtml } from './render'

const el = (html: string): HTMLElement => {
  const d = document.createElement('div')
  d.innerHTML = html
  return d
}

describe('toMarkdown', () => {
  it('turns generated summary markup into headings and task-list items', () => {
    const html = renderSummaryHtml({
      title: 'Daily report', subtitle: 'Sep 29', total: 2,
      sections: [{ heading: 'Completed (1)', groups: [{ heading: null, items: [{ title: 'Ship it', done: true, meta: ['Work'] }, { title: 'Later', done: false, meta: [] }] }] }],
    })
    expect(toMarkdown(el(html))).toBe('# Daily report\n\nSep 29\n\n## Completed (1)\n\n- [x] Ship it — Work\n- [ ] Later\n')
  })

  it('handles inline formatting, links, code, highlight and breaks', () => {
    const md = toMarkdown(el('<p><b>bold</b> <i>it</i> <s>gone</s> <mark>hi</mark> <code>a*b</code> <a href="https://x.dev">site</a><br>next</p>'))
    expect(md).toBe('**bold** *it* ~~gone~~ ==hi== `a*b` [site](https://x.dev)  \nnext\n')
  })

  it('escapes markdown characters in plain text', () => {
    expect(toMarkdown(el('<p>2 * 3 _x_ [y]</p>'))).toBe('2 \\* 3 \\_x\\_ \\[y\\]\n')
  })

  it('handles quotes, code blocks, rules, bullets, numbered and nested lists', () => {
    const md = toMarkdown(el('<blockquote>quoted</blockquote><pre>a\nb</pre><hr><ul><li>one<ul><li>inner</li></ul></li></ul><ol><li>x</li><li>y</li></ol>'))
    expect(md).toBe('> quoted\n\n```\na\nb\n```\n\n---\n\n- one\n  - inner\n\n1. x\n2. y\n')
  })

  it('treats the divs Chrome makes for lines as paragraphs, and bare text as a paragraph', () => {
    expect(toMarkdown(el('bare<div>one</div><div><br></div><div>two</div>'))).toBe('bare\n\none\n\ntwo\n')
  })

  it('drops links with unsafe schemes but keeps their text', () => {
    expect(toMarkdown(el('<p><a href="javascript:alert(1)">click</a></p>'))).toBe('click\n')
  })

  it('is just a newline for an empty editor', () => {
    expect(toMarkdown(el(''))).toBe('\n')
  })
})

describe('toPlainText', () => {
  it('keeps structure but no markup', () => {
    const txt = toPlainText(el('<h2>Title</h2><p><b>bold</b> *star*</p><ul data-checklist><li data-checked="true">a</li><li data-checked="false">b</li></ul>'))
    expect(txt).toBe('Title\n\nbold *star*\n\n- [x] a\n- [ ] b\n')
  })
})

describe('toHtmlDocument', () => {
  it('is a complete document with a title', () => {
    const html = toHtmlDocument(el('<h1>Hi</h1>'), 'My <summary>')
    expect(html.startsWith('<!doctype html>')).toBe(true)
    expect(html).toContain('<title>My &lt;summary&gt;</title>')
    expect(html).toContain('<h1>Hi</h1>')
  })

  it('renders checklists as disabled checkboxes', () => {
    const html = toHtmlDocument(el('<ul data-checklist><li data-checked="true">a</li><li data-checked="false">b</li></ul>'), 't')
    expect(html).toContain('<li><input type="checkbox" disabled checked> a</li>')
    expect(html).toContain('<li><input type="checkbox" disabled> b</li>')
  })

  it('strips scripts, event handlers, styles and unsafe links', () => {
    const html = toHtmlDocument(el('<p onclick="x()" style="color:red">t<script>alert(1)</script><img src=x onerror=y><a href="javascript:z()">l</a><a href="https://ok.dev" onclick="q">ok</a></p>'), 't')
    const body = html.slice(html.indexOf('<body>'))
    expect(body).not.toMatch(/onclick|onerror|style=|<img|javascript:/)
    expect(body).toContain('<a href="https://ok.dev" rel="noopener noreferrer">ok</a>')
    expect(body).toContain('<p>t')
  })
})

describe('helpers', () => {
  it('safeHref allows web and mail links only', () => {
    expect(safeHref('https://a.b')).toBe('https://a.b')
    expect(safeHref(' mailto:a@b.c')).toBe('mailto:a@b.c')
    expect(safeHref('javascript:alert(1)')).toBeNull()
    expect(safeHref('data:text/html,x')).toBeNull()
    expect(safeHref(null)).toBeNull()
  })
  it('isEmptyContent ignores whitespace and empty markup but counts a divider', () => {
    expect(isEmptyContent(el(' <div><br></div> '))).toBe(true)
    expect(isEmptyContent(el('<p>x</p>'))).toBe(false)
    expect(isEmptyContent(el('<hr>'))).toBe(false)
  })
})
