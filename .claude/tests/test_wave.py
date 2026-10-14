"""super-board-wave.js lane handoff: review finding 6.

The workflow is run under node with `agent` and `pipeline` stubbed, and the prompts each
lane receives are inspected. Run: python3 -m unittest discover -s .claude/tests -v
"""
import json
import os
import pathlib
import shutil
import subprocess
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
WAVE = ROOT / 'workflows' / 'super-board-wave.js'

HARNESS = r'''
const fs = require('fs')
const scenario = JSON.parse(process.env.SCENARIO)
const src = fs.readFileSync(process.env.WAVE, 'utf8').replace(/^export const meta/m, 'const meta')
const prompts = []
const agent = async (prompt, opts) => {
  const label = opts.label
  prompts.push({ label, prompt })
  if (label.startsWith('preflight')) {
    return { verdicts: scenario.cards.map((c) => ({ number: c.number, verdict: 'proceed', detail: 'ok' })) }
  }
  if (label.startsWith('classify')) return { kind: 'feature', complexity: 'low' }
  const lane = label.split(':')[0]
  return scenario.lanes[lane]
}
const pipeline = async (items, ...stages) => {
  const out = []
  for (const item of items) {
    let prev = item
    let first = true
    for (const stage of stages) {
      prev = first ? await stage(item) : await stage(prev, item)
      first = false
    }
    out.push(prev)
  }
  return out
}
const log = () => {}
const body = new Function('args', 'agent', 'pipeline', 'log', 'return (async () => {' + src + '\n})()')
body({ configPath: 'cfg.json', cards: scenario.cards }, agent, pipeline, log).then(
  (r) => { console.log(JSON.stringify({ result: r, prompts })) },
  (e) => { console.error(String(e && e.stack || e)); process.exit(1) })
'''

ADVANCED_BUILD = {'status': 'advanced', 'column': 'QA', 'detail': 'draft PR opened', 'branch': 'super-board/5-thing',
                  'prUrl': 'https://github.com/o/r/pull/50'}
ADVANCED = {'status': 'advanced', 'column': 'Review', 'detail': 'ok'}
MERGED = {'status': 'advanced', 'column': 'Done', 'detail': 'merged'}


def run_wave(cards, lanes):
    r = subprocess.run(['node', '-e', HARNESS], capture_output=True, text=True, timeout=60,
                       env={**os.environ, 'WAVE': str(WAVE), 'SCENARIO': json.dumps({'cards': cards, 'lanes': lanes})})
    if r.returncode:
        raise AssertionError(r.stderr)
    out = json.loads(r.stdout)
    return {p['label']: p['prompt'] for p in out['prompts']}


@unittest.skipUnless(shutil.which('node'), 'node is required')
class Handoff(unittest.TestCase):
    def test_ready_feature_built_then_qa_tests_the_built_change(self):
        card = {'number': 5, 'status': 'Ready', 'title': 'thing', 'lane': 'build', 'labels': ['feature']}
        prompts = run_wave([card], {'build': ADVANCED_BUILD, 'qa': ADVANCED, 'review': MERGED})
        qa = prompts['qa:#5']
        self.assertNotIn('skips Building', qa, 'a built feature must not start from the base branch')
        self.assertIn('branch super-board/5-thing', qa)
        self.assertIn('https://github.com/o/r/pull/50', qa)
        self.assertIn('Test THAT change', qa)

    def test_review_receives_the_qa_handoff(self):
        card = {'number': 5, 'status': 'Ready', 'title': 'thing', 'lane': 'build', 'labels': ['feature']}
        prompts = run_wave([card], {'build': ADVANCED_BUILD, 'qa': ADVANCED, 'review': MERGED})
        self.assertIn('previous lane (qa) advanced', prompts['review:#5'])

    def test_qa_labelled_ready_card_starts_from_base(self):
        card = {'number': 7, 'status': 'Ready', 'title': 'verify', 'lane': 'qa', 'labels': ['qa']}
        prompts = run_wave([card], {'qa': ADVANCED, 'review': MERGED})
        self.assertIn('skips Building', prompts['qa:#7'])
        self.assertNotIn('Test THAT change', prompts['qa:#7'])

    def test_card_already_in_qa_gets_no_base_branch_startup(self):
        card = {'number': 8, 'status': 'QA', 'title': 'mid', 'lane': 'qa', 'labels': ['qa']}
        prompts = run_wave([card], {'qa': ADVANCED, 'review': MERGED})
        self.assertNotIn('skips Building', prompts['qa:#8'])


if __name__ == '__main__':
    unittest.main()
