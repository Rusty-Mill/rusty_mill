"""super-board-card.sh against a fake gh: review findings 5 (assignees) and 11 (uncertain writes).

Run: python3 -m unittest discover -s .claude/tests -v
"""
import json
import os
import pathlib
import shlex
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest

CARD = pathlib.Path(__file__).resolve().parents[1] / 'bin' / 'super-board-card.sh'

FAKE_GH = r'''
import json, os, sys
a = sys.argv[1:]
env = os.environ
log = env.get('FAKE_GH_LOG')
q = next((x[6:] for x in a if x.startswith('query=')), '')
if log:
    open(log, 'a').write(('MUTATION' if 'mutation(' in q else 'READ') + '\n')
if 'mutation(' in q:
    mode = env.get('FAKE_MUTATION', 'ok')
    n = os.path.join(env['FAKE_DIR'], 'mutations')
    count = int(open(n).read()) + 1 if os.path.exists(n) else 1
    open(n, 'w').write(str(count))
    if mode == 'ok' or (mode == 'reject-once' and count > 1):
        print(json.dumps({'data': {'updateProjectV2ItemFieldValue': {'projectV2Item': {'id': 'ITEM1'}}}}))
    elif mode in ('reject', 'reject-once'):
        print(json.dumps({'errors': [{'message': 'option not found'}]}))
        sys.exit(1)
    else:  # lost: no readable response
        print('error connecting to api.github.com')
        sys.exit(1)
elif 'issueOrPullRequest' in q:
    st = env.get('FAKE_STATUS', 'Ready')
    print(json.dumps({'data': {'repository': {'issueOrPullRequest': {'projectItems': {'nodes': [
        {'id': 'ITEM1', 'project': {'number': 1, 'owner': {'login': 'o'}}, 'fieldValueByName': {'name': st}}]}}}}}))
elif 'items(first:100' in q:
    def node(i, num, status, logins):
        return {'id': f'I{i}', 'fieldValueByName': {'name': status},
                'content': {'__typename': 'Issue', 'number': num, 'title': f't{num}', 'url': f'u{num}',
                            'repository': {'nameWithOwner': 'o/r'}, 'labels': {'nodes': []},
                            'assignees': {'nodes': [{'login': l} for l in logins]}}}
    nodes = [node(1, 1, 'Ready', ['bot']), node(2, 2, 'Building', ['bot']), node(3, 3, 'QA', ['bot']),
             node(4, 4, 'Review', ['bot']), node(5, 5, 'Ready', [])]
    page = {'data': {'repositoryOwner': {'projectV2': {'items': {'totalCount': 5, 'pageInfo': {'hasNextPage': False, 'endCursor': None}, 'nodes': nodes}}}}}
    print(json.dumps([page]))
elif 'projectV2(number' in q:
    print(json.dumps({'data': {'repositoryOwner': {'projectV2': {'id': 'P', 'field': {'id': 'F', 'options': [
        {'id': 'o1', 'name': 'Ready'}, {'id': 'o2', 'name': 'QA'}]}}}}}))
else:
    sys.exit(1)
'''


class Card(unittest.TestCase):
    def setUp(self):
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        bindir = self.tmp / 'bin'
        bindir.mkdir()
        (bindir / 'fake_gh.py').write_text(FAKE_GH)
        gh = bindir / 'gh'
        gh.write_text('#!/bin/sh\nexec python3 "$(dirname "$0")/fake_gh.py" "$@"\n')
        gh.chmod(gh.stat().st_mode | stat.S_IEXEC)
        (bindir / 'gh.cmd').write_text('@"' + sys.executable + '" "%~dp0fake_gh.py" %*' + chr(13) + chr(10))
        self.bindir = bindir
        self.log = self.tmp / 'gh.log'
        self.halt = self.tmp / 'halt.json'

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def card(self, *verb, mutation='ok', status='Ready'):
        env = {**os.environ, 'PATH': f'{self.bindir}{os.pathsep}{os.environ["PATH"]}',
               'SB_GH': shlex.join([pathlib.Path(sys.executable).as_posix(), (self.bindir / 'fake_gh.py').as_posix()]),
               'SB_GITHUB_HALT_FILE': str(self.halt), 'SB_GITHUB_RETRY_DELAY': '0',
               'SB_CARD_CACHE_DIR': str(self.tmp / 'cache'), 'FAKE_DIR': str(self.tmp),
               'FAKE_GH_LOG': str(self.log), 'FAKE_MUTATION': mutation, 'FAKE_STATUS': status}
        r = subprocess.run(['bash', str(CARD), '--owner', 'o', '--number', '1', '--repo', 'o/r', *verb],
                           env=env, capture_output=True, text=True, timeout=120)
        calls = self.log.read_text().split() if self.log.exists() else []
        return r, calls.count('MUTATION')

    def test_items_keep_assignees_for_every_active_column(self):
        r, _ = self.card('items')
        self.assertEqual(r.returncode, 0, r.stderr)
        items = {i['content']['number']: i for i in json.loads(r.stdout)['items']}
        for number, status in ((1, 'Ready'), (2, 'Building'), (3, 'QA'), (4, 'Review')):
            self.assertEqual(items[number]['status'], status)
            self.assertEqual(items[number]['content']['assignees'], ['bot'], f'#{number}')
        self.assertEqual(items[5]['content']['assignees'], [])

    def test_clean_move_is_one_mutation(self):
        r, mutations = self.card('move', '1', 'QA')
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(mutations, 1)

    def test_rejected_write_is_retried_once_after_refreshing_ids(self):
        r, mutations = self.card('move', '1', 'QA', mutation='reject-once')
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(mutations, 2)

    def test_always_rejected_write_stops_after_two_attempts(self):
        r, mutations = self.card('move', '1', 'QA', mutation='reject')
        self.assertEqual(r.returncode, 69)
        self.assertEqual(mutations, 2)

    def test_lost_response_not_replayed_and_halts_when_card_unchanged(self):
        r, mutations = self.card('move', '1', 'QA', mutation='lost', status='Ready')
        self.assertEqual(r.returncode, 79, r.stderr)
        self.assertEqual(mutations, 1, 'an uncertain write must never be replayed')
        self.assertTrue(self.halt.exists())

    def test_lost_response_that_actually_landed_is_confirmed_not_replayed(self):
        r, mutations = self.card('move', '1', 'QA', mutation='lost', status='QA')
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(mutations, 1)
        self.assertFalse(self.halt.exists())

    def test_halt_stops_warm_cache_move_before_any_mutation(self):
        self.card('ids')  # warm the cache
        self.halt.write_text(json.dumps({'reason': 'x', 'attempts': 1, 'at': 0, 'operation': None}))
        self.log.unlink(missing_ok=True)
        r, mutations = self.card('move', '1', 'QA')
        self.assertEqual(r.returncode, 79, r.stderr)
        self.assertEqual(mutations, 0)


if __name__ == '__main__':
    unittest.main()
