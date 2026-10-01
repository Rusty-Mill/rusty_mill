import { SHORTCUTS } from '../shortcutList'
import { PaneTitle } from './controls'

const Key = ({ children }: { children: string }) => (
  <kbd className="inline-flex h-6 min-w-6 items-center justify-center rounded-[6px] border border-line bg-hover px-1.5 font-sans text-s text-text">{children}</kbd>
)

export function ShortcutsTab() {
  return (
    <>
      <PaneTitle>Shortcuts</PaneTitle>
      {SHORTCUTS.map((g) => (
        <table key={g.title} className="mb-5 w-full border-collapse text-left">
          <caption className="mb-1 text-left text-s font-semibold text-grey">{g.title}</caption>
          <thead className="sr-only">
            <tr>
              <th scope="col">Shortcut</th>
              <th scope="col">Action</th>
            </tr>
          </thead>
          <tbody>
            {g.rows.map((r) => (
              <tr key={r.action} className="border-b border-line last:border-b-0">
                <td className="w-[45%] py-2 pr-4 align-middle">
                  <span className="flex flex-wrap items-center gap-x-2 gap-y-1">
                    {r.keys.map((chord, i) => (
                      <span key={i} className="flex items-center gap-1">
                        {i > 0 && <span className="mr-1 text-s text-grey">or</span>}
                        {chord.map((k, j) => (
                          <span key={j} className="flex items-center gap-1">
                            {j > 0 && <span className="text-s text-grey">+</span>}
                            <Key>{k}</Key>
                          </span>
                        ))}
                      </span>
                    ))}
                  </span>
                </td>
                <td className="py-2 text-base">{r.action}</td>
              </tr>
            ))}
          </tbody>
        </table>
      ))}
    </>
  )
}
