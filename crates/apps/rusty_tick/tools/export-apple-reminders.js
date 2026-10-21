// Export Apple Reminders as CSV for rusty_tick's "Import from Other Apps".
//
//   osascript -l JavaScript export-apple-reminders.js > reminders.csv          # open reminders
//   osascript -l JavaScript export-apple-reminders.js --all > reminders.csv    # completed ones too
//
// macOS asks once for permission to control Reminders. Columns: List, Title,
// Notes, Due (ISO; a date alone is all day), Priority (high|medium|low|none),
// Tags ("flagged" for flagged reminders), Completed. Reminders' scripting
// interface does not expose tags, subtasks or repeat rules, so those are not
// carried over.

function run(argv) {
  const all = argv.includes('--all')
  const cell = (v) => (/[",\n\r]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v)
  const pad = (n) => String(n).padStart(2, '0')
  const day = (d) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`
  // Apple's scale: 1-4 high, 5 medium, 6-9 low, 0 none.
  const priority = (p) => (p === 0 ? 'none' : p < 5 ? 'high' : p === 5 ? 'medium' : 'low')
  const due = (allDay, at) => (allDay ? day(allDay) : at ? at.toISOString() : '')

  const rows = [['List', 'Title', 'Notes', 'Due', 'Priority', 'Tags', 'Completed']]
  for (const list of Application('Reminders').lists()) {
    const r = list.reminders
    const name = list.name()
    // One Apple event per property for the whole list, not one per reminder.
    const [names, bodies, done, dues, allDays, priorities, flagged] = [r.name(), r.body(), r.completed(), r.dueDate(), r.alldayDueDate(), r.priority(), r.flagged()]
    for (let i = 0; i < names.length; i++) {
      if (done[i] && !all) continue
      rows.push([name, names[i], bodies[i] || '', due(allDays[i], dues[i]), priority(priorities[i]), flagged[i] ? 'flagged' : '', done[i] ? 'true' : ''])
    }
  }
  return rows.map((row) => row.map((v) => cell(String(v))).join(',')).join('\n')
}
