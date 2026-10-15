"""Regression tests for review findings 1-3 on the merge gate.

Run: python3 -m unittest discover -s .claude/tests -v
"""
import importlib.util
import json
import pathlib
import tempfile
import unittest

BIN = pathlib.Path(__file__).resolve().parents[1] / 'bin'
spec = importlib.util.spec_from_file_location('merge_checks', BIN / 'super-board-merge-checks.py')
mc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mc)

HEAD = 'a' * 40
OLD = 'b' * 40
ALL_BUT_SQUASH = {'mergeCommitAllowed': True, 'squashMergeAllowed': False, 'rebaseMergeAllowed': False}


def code_of(fn, *args, **kw):
    try:
        fn(*args, **kw)
    except mc.Refuse as e:
        return e.code
    return 0


def rollup(*entries, head=HEAD):
    return {'headRefOid': head, 'statusCheckRollup': list(entries)}


def ok(name='ci'):
    return {'name': name, 'status': 'COMPLETED', 'conclusion': 'SUCCESS'}


class MergeMethod(unittest.TestCase):
    """Finding 1: the gate must not call --squash on a repo that disables it."""

    def test_default_is_merge_commit(self):
        self.assertEqual(mc.choose_method({}, ALL_BUT_SQUASH), 'merge')

    def test_squash_disabled_is_refused_not_used(self):
        self.assertEqual(code_of(mc.choose_method, {'merge_policy': {'method': 'squash'}}, ALL_BUT_SQUASH), 9)

    def test_no_silent_fallback_when_default_disallowed(self):
        only_squash = {'mergeCommitAllowed': False, 'squashMergeAllowed': True, 'rebaseMergeAllowed': False}
        self.assertEqual(code_of(mc.choose_method, {}, only_squash), 9)

    def test_unreadable_settings_fail_closed(self):
        self.assertEqual(code_of(mc.choose_method, {}, {}), 9)

    def test_unknown_method_rejected(self):
        self.assertEqual(code_of(mc.choose_method, {'merge_policy': {'method': 'fast-forward'}}, ALL_BUT_SQUASH), 9)

    def test_flag_mapping(self):
        self.assertEqual(mc.FLAGS['merge'], '--merge')


class Target(unittest.TestCase):
    """Finding 2: a PR targeting main must not merge on a staging-configured board."""

    def test_matching_base_passes(self):
        self.assertEqual(code_of(mc.check_target, 'staging', {'baseRefName': 'staging'}), 0)

    def test_main_targeting_pr_refused(self):
        self.assertEqual(code_of(mc.check_target, 'staging', {'baseRefName': 'main'}), 9)

    def test_retargeted_pr_refused(self):
        self.assertEqual(code_of(mc.check_target, 'staging', {'baseRefName': 'release'}), 9)

    def test_missing_base_fails_closed(self):
        self.assertEqual(code_of(mc.check_target, 'staging', {}), 12)


class Ci(unittest.TestCase):
    """Finding 3: merge only on green CI for the exact head."""

    def test_green(self):
        self.assertEqual(code_of(mc.evaluate_ci, rollup(ok(), ok('lint')), HEAD), 0)

    def test_skipped_and_neutral_count_as_ok(self):
        skipped = {'name': 's', 'status': 'COMPLETED', 'conclusion': 'SKIPPED'}
        neutral = {'name': 'n', 'status': 'COMPLETED', 'conclusion': 'NEUTRAL'}
        self.assertEqual(code_of(mc.evaluate_ci, rollup(ok(), skipped, neutral), HEAD), 0)

    def test_failing(self):
        bad = {'name': 'test', 'status': 'COMPLETED', 'conclusion': 'FAILURE'}
        self.assertEqual(code_of(mc.evaluate_ci, rollup(ok(), bad), HEAD), 11)

    def test_cancelled_and_timed_out_fail(self):
        for conclusion in ('CANCELLED', 'TIMED_OUT', 'ACTION_REQUIRED'):
            bad = {'name': 'x', 'status': 'COMPLETED', 'conclusion': conclusion}
            self.assertEqual(code_of(mc.evaluate_ci, rollup(bad), HEAD), 11, conclusion)

    def test_pending_check_run(self):
        running = {'name': 'test', 'status': 'IN_PROGRESS', 'conclusion': ''}
        self.assertEqual(code_of(mc.evaluate_ci, rollup(ok(), running), HEAD), 10)

    def test_pending_commit_status(self):
        self.assertEqual(code_of(mc.evaluate_ci, rollup({'context': 'ci/x', 'state': 'PENDING'}), HEAD), 10)

    def test_failing_beats_pending(self):
        bad = {'context': 'ci/x', 'state': 'FAILURE'}
        running = {'name': 'test', 'status': 'QUEUED'}
        self.assertEqual(code_of(mc.evaluate_ci, rollup(running, bad), HEAD), 11)

    def test_missing_rollup_fails_closed(self):
        self.assertEqual(code_of(mc.evaluate_ci, {'headRefOid': HEAD}, HEAD), 12)
        self.assertEqual(code_of(mc.evaluate_ci, rollup(), HEAD), 12)

    def test_older_head_evidence_refused(self):
        self.assertEqual(code_of(mc.evaluate_ci, rollup(ok(), head=OLD), HEAD), 12)

    def test_required_check_missing(self):
        self.assertEqual(code_of(mc.evaluate_ci, rollup(ok('lint')), HEAD, ['required-gate']), 12)

    def test_required_check_present(self):
        self.assertEqual(code_of(mc.evaluate_ci, rollup(ok('required-gate')), HEAD, ['required-gate']), 0)

    def test_nameless_entry_fails_closed(self):
        self.assertEqual(code_of(mc.evaluate_ci, rollup({'status': 'COMPLETED', 'conclusion': 'SUCCESS'}), HEAD), 12)


class Cli(unittest.TestCase):
    def _files(self, **docs):
        d = pathlib.Path(tempfile.mkdtemp())
        paths = {}
        for name, doc in docs.items():
            p = d / f'{name}.json'
            p.write_text(json.dumps(doc), encoding='utf-8')
            paths[name] = str(p)
        return paths

    def test_method_prints_flag(self):
        f = self._files(cfg={}, settings=ALL_BUT_SQUASH)
        self.assertEqual(mc.main(['method', '--config', f['cfg'], '--settings', f['settings']]), 0)

    def test_ci_exit_codes(self):
        f = self._files(green=rollup(ok()), red=rollup({'name': 'x', 'status': 'COMPLETED', 'conclusion': 'FAILURE'}))
        self.assertEqual(mc.main(['ci', '--head', HEAD, '--meta', f['green']]), 0)
        self.assertEqual(mc.main(['ci', '--head', HEAD, '--meta', f['red']]), 11)

    def test_ci_short_head_rejected(self):
        f = self._files(green=rollup(ok()))
        self.assertEqual(mc.main(['ci', '--head', 'abc', '--meta', f['green']]), 12)

    def test_ci_unreadable_file_fails_closed(self):
        self.assertEqual(mc.main(['ci', '--head', HEAD, '--meta', '/nonexistent/x.json']), 12)

    def test_ci_required_false_is_explicit_opt_out(self):
        f = self._files(red=rollup({'name': 'x', 'status': 'COMPLETED', 'conclusion': 'FAILURE'}), cfg={'ci_required': False})
        self.assertEqual(mc.main(['ci', '--head', HEAD, '--meta', f['red'], '--config', f['cfg']]), 0)

    def test_target_cli(self):
        f = self._files(main={'baseRefName': 'main'})
        self.assertEqual(mc.main(['target', '--expect', 'staging', '--meta', f['main']]), 9)


if __name__ == '__main__':
    unittest.main()
