"""Hook regressions: review findings 8 (cleanup-wt), 9 (guard-delete-outside), 10 (guard-protected-push).

Run: python3 -m unittest discover -s .claude/tests -v
"""
import importlib.util
import os
import pathlib
import shutil
import subprocess
import tempfile
import unittest
from unittest import mock

HOOKS = pathlib.Path(__file__).resolve().parents[1] / 'hooks'


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, HOOKS / file)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


cleanup = load('cleanup_wt', 'cleanup-wt.py')
delete_guard = load('guard_delete_outside', 'guard-delete-outside.py')
push_guard = load('guard_protected_push', 'guard-protected-push.py')


def git(cwd, *args):
    r = subprocess.run(['git', '-C', str(cwd), *args], capture_output=True, text=True)
    if r.returncode:
        raise AssertionError(f'git {args}: {r.stderr}')
    return r.stdout.strip()


class MergeCommitEvidence(unittest.TestCase):
    """Finding 8: git cherry skips merge commits, so it cannot prove a merge's content is in the base."""

    def setUp(self):
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        self.repo = self.tmp / 'r'
        self.repo.mkdir()
        git(self.repo, 'init', '-q', '-b', 'main')
        git(self.repo, 'config', 'user.email', 't@t')
        git(self.repo, 'config', 'user.name', 't')
        (self.repo / 'base.txt').write_text('base')
        git(self.repo, 'add', '.')
        git(self.repo, 'commit', '-q', '-m', 'base')

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def commit(self, name, text):
        (self.repo / name).write_text(text)
        git(self.repo, 'add', name)
        git(self.repo, 'commit', '-q', '-m', f'add {name}')
        return git(self.repo, 'rev-parse', 'HEAD')

    def bases(self):
        return [{'ref': 'refs/heads/main', 'name': 'main', 'sha': git(self.repo, 'rev-parse', 'main')}]

    def test_unique_merge_resolution_is_not_reported_merged(self):
        git(self.repo, 'switch', '-q', '-c', 'feat')
        a = self.commit('a.txt', 'a')
        git(self.repo, 'switch', '-q', '-c', 'side', 'main')
        s = self.commit('b.txt', 'b')
        git(self.repo, 'switch', '-q', 'feat')
        git(self.repo, 'merge', '-q', '--no-ff', '--no-commit', 'side')
        (self.repo / 'only-in-merge.txt').write_text('unique conflict resolution')
        git(self.repo, 'add', 'only-in-merge.txt')
        git(self.repo, 'commit', '-q', '-m', 'merge side with a manual fix')
        tip = git(self.repo, 'rev-parse', 'HEAD')
        git(self.repo, 'switch', '-q', 'main')
        git(self.repo, 'cherry-pick', a)
        git(self.repo, 'cherry-pick', s)
        # the ordinary commits are all in main (cherry says '-'), the merge's own content is not
        cherry = git(self.repo, 'cherry', 'main', tip).splitlines()
        self.assertTrue(cherry and all(l.startswith('-') for l in cherry), cherry)
        self.assertIsNone(cleanup.merged_state(str(self.repo), tip, self.bases()))

    def test_plain_cherry_picked_branch_is_still_recognised(self):
        git(self.repo, 'switch', '-q', '-c', 'feat')
        a = self.commit('a.txt', 'a')
        tip = self.commit('b.txt', 'b')
        git(self.repo, 'switch', '-q', 'main')
        git(self.repo, 'cherry-pick', a)
        git(self.repo, 'cherry-pick', tip)
        self.assertEqual(cleanup.merged_state(str(self.repo), tip, self.bases()), 'every patch already in main')


class DeleteGuard(unittest.TestCase):
    """Finding 9: a symlinked parent must not let an outside deletion pass as 'inside the project'."""

    PROJECT = os.path.normpath('/proj')

    def fake_realpath(self, mapping):
        real = os.path.realpath

        def rp(p):
            p = os.path.normpath(p)
            for src, dst in mapping.items():
                if p == src or p.startswith(src + os.sep):
                    return os.path.normpath(dst + p[len(src):])
            return real(p)
        return rp

    def verdict(self, path, mapping=None):
        with mock.patch.object(os.path, 'realpath', self.fake_realpath(mapping or {})):
            return delete_guard.verdict(os.path.normpath(path), self.PROJECT, {})

    def test_symlinked_parent_pointing_outside_is_denied(self):
        outside = os.path.normpath('/outside/data')
        why = self.verdict('/proj/cache/report', {os.path.normpath('/proj/cache'): outside})
        self.assertIsNotNone(why)
        self.assertIn('outside the project', why)

    def test_plain_path_inside_project_is_allowed(self):
        self.assertIsNone(self.verdict('/proj/build/out'))

    def test_deleting_the_symlink_itself_is_allowed(self):
        # /proj/link -> /outside: rm removes the link; the final component is not followed
        self.assertIsNone(self.verdict('/proj/link', {os.path.normpath('/proj/link'): os.path.normpath('/outside')}))

    def test_symlink_that_stays_inside_is_allowed(self):
        self.assertIsNone(self.verdict('/proj/cache/report', {os.path.normpath('/proj/cache'): os.path.normpath('/proj/real-cache')}))


class PushGuard(unittest.TestCase):
    """Finding 10: --repo supplies the remote, so every positional is a refspec."""

    PROTECTED = {'main', 'master', 'staging'}

    def check(self, command, current='feature'):
        return push_guard.check(command, '/work', self.PROTECTED, lambda d: current)

    def test_repo_option_with_protected_refspec_is_blocked(self):
        self.assertIsNotNone(self.check('git push --repo origin HEAD:staging'))

    def test_repo_option_delete_is_blocked(self):
        self.assertIsNotNone(self.check('git push --repo origin --delete staging'))

    def test_repo_equals_form_is_blocked(self):
        self.assertIsNotNone(self.check('git push --repo=origin HEAD:main'))

    def test_repo_option_feature_branch_is_allowed(self):
        self.assertIsNone(self.check('git push --repo origin HEAD:feature-x'))

    def test_repo_option_bare_push_from_protected_branch_is_blocked(self):
        self.assertIsNotNone(self.check('git push --repo origin', current='staging'))

    def test_existing_positional_remote_still_works(self):
        self.assertIsNotNone(self.check('git push origin HEAD:staging'))
        self.assertIsNone(self.check('git push -u origin feature-x'))


if __name__ == '__main__':
    unittest.main()
