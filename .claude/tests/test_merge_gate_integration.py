"""Runs the real merge gate against a fake `gh` and a throwaway git repo.

Covers review findings 1-3 end to end: merge flag, target branch, exact-head CI.
Run: python3 -m unittest discover -s .claude/tests -v
"""
import json
import os
import pathlib
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest

GATE = pathlib.Path(__file__).resolve().parents[1] / 'bin' / 'super-board-merge-gate.sh'

FAKE_GH = r'''
import json, os, sys
a = sys.argv[1:]
env = os.environ
log = env.get('FAKE_GH_LOG')
if log:
    open(log, 'a').write(' '.join(a) + '\n')
sha = env['FAKE_HEAD']
j = a[a.index('--json') + 1] if '--json' in a else ''
if a[:2] == ['pr', 'merge']:
    print('merged')
elif a[:2] == ['pr', 'diff']:
    pass
elif a[:2] == ['repo', 'view']:
    print(env['FAKE_SETTINGS'])
elif a[:2] == ['pr', 'view']:
    if '-q' in a:
        print('feature ' + sha)
    elif j == 'baseRefName,headRefOid':
        print(json.dumps({'baseRefName': env['FAKE_BASE'], 'headRefOid': sha}))
    elif j == 'headRefOid,statusCheckRollup':
        print(env['FAKE_CI'])
    elif j.startswith('state'):
        print(json.dumps({'state': 'OPEN', 'mergeCommit': None, 'headRefOid': sha}))
    else:
        print(json.dumps({'labels': [], 'files': [], 'additions': 0, 'deletions': 0, 'changedFiles': 0, 'body': ''}))
else:
    sys.exit(1)
'''


def sh(*cmd, cwd=None):
    return subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, check=True).stdout.strip()


class Gate(unittest.TestCase):
    def setUp(self):
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        origin, repo = self.tmp / 'origin.git', self.tmp / 'repo'
        sh('git', 'init', '-q', '--bare', '-b', 'staging', str(origin))
        sh('git', 'init', '-q', '-b', 'staging', str(repo))
        for k, v in (('user.email', 't@t'), ('user.name', 't')):
            sh('git', 'config', k, v, cwd=repo)
        (repo / 'a.txt').write_text('a')
        sh('git', 'add', 'a.txt', cwd=repo)
        sh('git', 'commit', '-q', '-m', 'base', cwd=repo)
        sh('git', 'remote', 'add', 'origin', str(origin), cwd=repo)
        sh('git', 'push', '-q', 'origin', 'staging', cwd=repo)
        sh('git', 'switch', '-q', '-c', 'feature', cwd=repo)
        (repo / 'b.txt').write_text('b')
        sh('git', 'add', 'b.txt', cwd=repo)
        sh('git', 'commit', '-q', '-m', 'feat', cwd=repo)
        sh('git', 'push', '-q', 'origin', 'feature', cwd=repo)
        self.sha = sh('git', 'rev-parse', 'HEAD', cwd=repo)
        sh('git', 'switch', '-q', 'staging', cwd=repo)
        self.repo = repo
        bindir = self.tmp / 'bin'
        bindir.mkdir()
        (bindir / 'fake_gh.py').write_text(FAKE_GH)
        gh = bindir / 'gh'
        gh.write_text('#!/bin/sh\nexec python3 "$(dirname "$0")/fake_gh.py" "$@"\n')
        gh.chmod(gh.stat().st_mode | stat.S_IEXEC)
        # Windows resolves `gh` for subprocess via PATHEXT, so it needs a .cmd twin.
        (bindir / 'gh.cmd').write_text('@"' + sys.executable + '" "%~dp0fake_gh.py" %*' + chr(13) + chr(10))
        self.bindir = bindir
        self.log = self.tmp / 'gh.log'
        self.cfg = {'base_branch': 'staging', 'repo': {'remote': 'https://github.com/o/r', 'path': str(repo)},
                    'verify_commands': []}

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def run_gate(self, base='staging', ci=None, settings=None, cfg=None, dry=False):
        cfgfile = self.tmp / 'cfg.json'
        cfgfile.write_text(json.dumps({**self.cfg, **(cfg or {})}))
        green = {'headRefOid': self.sha, 'statusCheckRollup': [{'name': 'ci', 'status': 'COMPLETED', 'conclusion': 'SUCCESS'}]}
        env = {**os.environ, 'PATH': f'{self.bindir}{os.pathsep}{os.environ["PATH"]}',
               'FAKE_HEAD': self.sha, 'FAKE_BASE': base, 'FAKE_GH_LOG': str(self.log),
               'FAKE_CI': json.dumps(ci or green),
               'FAKE_SETTINGS': json.dumps(settings or {'mergeCommitAllowed': True, 'squashMergeAllowed': False,
                                                         'rebaseMergeAllowed': False}),
               'SB_GH': str(self.bindir / ('gh.cmd' if os.name == 'nt' else 'gh')),
               'SB_GITHUB_HALT_FILE': str(self.tmp / 'halt.json'), 'SB_GITHUB_RETRY_DELAY': '0'}
        args = ['bash', str(GATE), '--config', str(cfgfile), '--pr', '7', '--expect-head', self.sha, '--lock-timeout', '5']
        if dry:
            args.append('--dry-run')
        r = subprocess.run(args, env=env, capture_output=True, text=True, timeout=120)
        calls = self.log.read_text() if self.log.exists() else ''
        return r, calls

    def test_green_merge_uses_merge_commit_flag_never_squash(self):
        r, calls = self.run_gate()
        self.assertEqual(r.returncode, 0, r.stderr)
        merge = [c for c in calls.splitlines() if c.startswith('pr merge')]
        self.assertEqual(len(merge), 1, calls)
        self.assertIn('--merge', merge[0])
        self.assertNotIn('--squash', merge[0])
        self.assertIn(f'--match-head-commit {self.sha}', merge[0])

    def test_squash_only_repo_refuses_without_merging(self):
        r, calls = self.run_gate(settings={'mergeCommitAllowed': False, 'squashMergeAllowed': True, 'rebaseMergeAllowed': False})
        self.assertEqual(r.returncode, 9, r.stderr)
        self.assertNotIn('pr merge', calls)

    def test_main_targeting_pr_on_staging_board_never_merges(self):
        r, calls = self.run_gate(base='main')
        self.assertEqual(r.returncode, 9, r.stderr)
        self.assertNotIn('pr merge', calls)

    def test_failing_ci_blocks_merge(self):
        bad = {'headRefOid': self.sha, 'statusCheckRollup': [{'name': 'test', 'status': 'COMPLETED', 'conclusion': 'FAILURE'}]}
        r, calls = self.run_gate(ci=bad)
        self.assertEqual(r.returncode, 11, r.stderr)
        self.assertNotIn('pr merge', calls)

    def test_pending_ci_blocks_merge(self):
        run = {'headRefOid': self.sha, 'statusCheckRollup': [{'name': 'test', 'status': 'IN_PROGRESS'}]}
        r, calls = self.run_gate(ci=run)
        self.assertEqual(r.returncode, 10, r.stderr)
        self.assertNotIn('pr merge', calls)

    def test_missing_ci_blocks_merge(self):
        r, calls = self.run_gate(ci={'headRefOid': self.sha, 'statusCheckRollup': []})
        self.assertEqual(r.returncode, 12, r.stderr)
        self.assertNotIn('pr merge', calls)

    def test_ci_for_older_head_blocks_merge(self):
        stale = {'headRefOid': 'c' * 40, 'statusCheckRollup': [{'name': 'ci', 'status': 'COMPLETED', 'conclusion': 'SUCCESS'}]}
        r, calls = self.run_gate(ci=stale)
        self.assertEqual(r.returncode, 12, r.stderr)
        self.assertNotIn('pr merge', calls)

    def test_dry_run_reports_flag_and_does_not_merge(self):
        r, calls = self.run_gate(dry=True)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn('--merge', r.stderr)
        self.assertNotIn('pr merge', calls)


if __name__ == '__main__':
    unittest.main()
