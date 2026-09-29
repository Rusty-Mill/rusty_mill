/** The keyboard shortcuts the app understands, for the Shortcuts tab. Keep in step with `features/tasks/shortcuts.ts`. */
export interface ShortcutGroup {
  title: string
  rows: { keys: string[][]; action: string }[]
}

/** Each entry of `keys` is one way to press it; `+` joins a chord. */
export const SHORTCUTS: ShortcutGroup[] = [
  {
    title: 'Tasks',
    rows: [
      { keys: [['N']], action: 'New task (focus quick add)' },
      { keys: [['J'], ['K']], action: 'Next / previous task' },
      { keys: [['Space']], action: 'Complete or reopen the selected task' },
      { keys: [['Delete'], ['Backspace']], action: 'Move the selected task to Trash' },
      { keys: [['1–4']], action: 'Set priority: High, Medium, Low, None' },
    ],
  },
  {
    title: 'Anywhere',
    rows: [
      { keys: [['Ctrl', 'K'], ['⌘', 'K']], action: 'Search' },
      { keys: [['Esc']], action: 'Close the open dialog, menu or detail pane' },
    ],
  },
  {
    title: 'Editing',
    rows: [
      { keys: [['Enter']], action: 'Add the task in quick add; add a checklist item' },
      { keys: [['Backspace']], action: 'Remove an empty checklist item' },
      { keys: [['Ctrl', 'B'], ['⌘', 'B']], action: 'Bold (Summary editor)' },
      { keys: [['Ctrl', 'I'], ['⌘', 'I']], action: 'Italic (Summary editor)' },
      { keys: [['Ctrl', 'Z'], ['⌘', 'Z']], action: 'Undo (Summary editor)' },
    ],
  },
]
