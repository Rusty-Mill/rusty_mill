"""Runs the real merge gate against a fake `gh` and a throwaway git repo.

Covers review findings 1-3 end to end: merge flag, target branch, exact-head CI.
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


APPROVAL_STUB = r'''
import json, os, sys
open(os.environ['FAKE_APPROVAL_LOG'], 'a').write(' '.join(sys.argv[1:]) + '\n')
if '--request' in sys.argv:
    print('approval-request: stub')
else:
    print(json.dumps({'approved': os.environ['FAKE_APPROVED'] == '1'}))
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

    def run_gate(self, base='staging', ci=None, settings=None, cfg=None, dry=False, gate=GATE, env_extra=None):
        cfgfile = self.tmp / 'cfg.json'
        cfgfile.write_text(json.dumps({**self.cfg, **(cfg or {})}))
        green = {'headRefOid': self.sha, 'statusCheckRollup': [{'name': 'ci', 'status': 'COMPLETED', 'conclusion': 'SUCCESS'}]}
        env = {**os.environ, 'PATH': f'{self.bindir}{os.pathsep}{os.environ["PATH"]}',
               'FAKE_HEAD': self.sha, 'FAKE_BASE': base, 'FAKE_GH_LOG': str(self.log),
               'FAKE_CI': json.dumps(ci or green),
               'FAKE_SETTINGS': json.dumps(settings or {'mergeCommitAllowed': True, 'squashMergeAllowed': False,
                                                         'rebaseMergeAllowed': False}),
               'SB_GH': shlex.join([pathlib.Path(sys.executable).as_posix(), (self.bindir / 'fake_gh.py').as_posix()]),
               'SB_GITHUB_HALT_FILE': str(self.tmp / 'halt.json'), 'SB_GITHUB_RETRY_DELAY': '0',
               **(env_extra or {})}
        args = ['bash', str(gate), '--config', str(cfgfile), '--pr', '7', '--expect-head', self.sha, '--lock-timeout', '5']
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

    # merge_policy.handoff_to: the robot never merges, approved or not.
    def run_handoff(self, approved, handoff):
        # A copy of the gate whose approval helper is a stub, so "a valid trusted
        # approval exists" is a test input rather than forged GitHub comments.
        bin_copy = self.tmp / 'gate-bin'
        if not bin_copy.exists():
            shutil.copytree(GATE.parent, bin_copy)
            (bin_copy / 'super-board-approval.py').write_text(APPROVAL_STUB)
        policy = {'default': 'human', **({'handoff_to': 'dottiusmaximus'} if handoff else {})}
        r, calls = self.run_gate(cfg={'merge_policy': policy}, gate=bin_copy / GATE.name,
                                 env_extra={'FAKE_APPROVED': '1' if approved else '0',
                                            'FAKE_APPROVAL_LOG': str(self.approval_log)})
        asked = self.approval_log.read_text() if self.approval_log.exists() else ''
        return r, calls, asked

    @property
    def approval_log(self):
        return self.tmp / 'approval.log'

    def test_approved_pr_on_handoff_board_is_not_merged(self):
        r, calls, asked = self.run_handoff(approved=True, handoff=True)
        self.assertEqual(r.returncode, 13, r.stderr)
        self.assertIn('handoff: @dottiusmaximus reviews and merges PR #7', r.stdout)
        self.assertNotIn('pr merge', calls)
        self.assertEqual(asked, '')

    def test_unapproved_pr_on_handoff_board_is_handed_off_without_a_request(self):
        r, calls, asked = self.run_handoff(approved=False, handoff=True)
        self.assertEqual(r.returncode, 13, r.stderr)
        self.assertNotIn('approval-request', r.stdout)
        self.assertNotIn('pr merge', calls)
        self.assertEqual(asked, '')

    def test_without_handoff_an_approved_human_hold_still_merges(self):
        # Control: the stub really unlocks the generic path, so the two tests above
        # fail because of handoff_to and nothing else.
        r, calls, _ = self.run_handoff(approved=True, handoff=False)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn('pr merge', calls)

    def test_without_handoff_an_unapproved_human_hold_waits(self):
        r, calls, _ = self.run_handoff(approved=False, handoff=False)
        self.assertEqual(r.returncode, 7, r.stderr)
        self.assertIn('approval-request: stub', r.stdout)
        self.assertNotIn('pr merge', calls)


if __name__ == '__main__':
    unittest.main()
