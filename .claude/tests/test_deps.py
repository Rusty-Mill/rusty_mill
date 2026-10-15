"""super-board-deps.sh: review finding 12 (a hidden older blocker comment).

Run: python3 -m unittest discover -s .claude/tests -v
"""
import json
import os
import pathlib
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest

DEPS = pathlib.Path(__file__).resolve().parents[1] / 'bin' / 'super-board-deps.sh'

FAKE_GH = r'''
import json, os, sys
a = sys.argv[1:]
env = os.environ
q = next((x[6:] for x in a if x.startswith('query=')), '')


def issue(n, body, comments, previous):
    return {'number': n, 'title': f'i{n}', 'body': body, 'state': 'OPEN', 'labels': {'nodes': []},
            'comments': {'pageInfo': {'hasPreviousPage': previous}, 'nodes': [{'body': c} for c in comments]}}


NONE = '## Blocked by\n- None.\n'
if 'issues(states:OPEN' in q:
    newest = ['chatter'] * 100
    nodes = [issue(9, NONE, [], False),
             issue(5, NONE, newest, True),     # an older blocked-by comment is hidden
             issue(6, NONE, newest, False)]    # same chatter, but the history is fully visible
    page = {'data': {'repository': {'issues': {'pageInfo': {'hasNextPage': False, 'endCursor': None}, 'nodes': nodes}}}}
    print(json.dumps([page]))
elif a[:2] == ['api', 'repos/o/r/issues/5/comments']:
    if env.get('FAKE_COMMENTS') == 'fail':
        print('HTTP 502 bad gateway')
        sys.exit(1)

    def c(i, body):
        return {'id': i, 'body': body, 'user': {'login': 'bot'}, 'created_at': '2026-01-01T00:00:00Z', 'updated_at': '2026-01-01T00:00:00Z'}
    print(json.dumps([[c(1, 'Reason tag: dependency\nblocked-by: 9')] + [c(i + 2, 'chatter') for i in range(100)]]))
else:
    sys.exit(1)
'''


class Deps(unittest.TestCase):
    def setUp(self):
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        (self.tmp / 'fake_gh.py').write_text(FAKE_GH)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def deps(self, comments='ok'):
        env = {**os.environ, 'SB_GITHUB_HALT_FILE': str(self.tmp / 'halt.json'), 'SB_GITHUB_RETRY_DELAY': '0',
               'SB_GH': shlex.join([pathlib.Path(sys.executable).as_posix(), (self.tmp / 'fake_gh.py').as_posix()]),
               'FAKE_COMMENTS': comments}
        return subprocess.run(['bash', str(DEPS), '--repo', 'o/r'], env=env, capture_output=True, text=True, timeout=120)

    def test_older_blocker_comment_is_found_and_holds_the_card(self):
        r = self.deps()
        self.assertEqual(r.returncode, 0, r.stderr)
        out = json.loads(r.stdout)
        self.assertEqual(out['5']['blockers'], [9])
        self.assertEqual(out['5']['openBlockers'], [9])
        self.assertFalse(out['5']['runnable'], 'eight later ordinary comments must not release a blocked card')

    def test_cards_with_visible_history_are_unchanged(self):
        out = json.loads(self.deps().stdout)
        self.assertTrue(out['6']['runnable'])
        self.assertTrue(out['9']['runnable'])

    def test_incomplete_history_fails_closed(self):
        r = self.deps(comments='fail')
        self.assertEqual(r.returncode, 79, r.stderr)
        self.assertEqual(r.stdout.strip(), '')


if __name__ == '__main__':
    unittest.main()
