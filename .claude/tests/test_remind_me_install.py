"""install_cloud_capture.py: puts the capture hook into another repository (ADR-0029).

Synthetic only: scratch repositories in a temporary directory.
Run: python3 -m unittest discover -s .claude/tests -v
"""
import contextlib
import importlib.util
import io
import json
import os
import pathlib
import stat
import sys
import tempfile
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location(
    'install_cloud_capture',
    ROOT / 'crates' / 'apps' / 'rusty_remind_me' / 'scripts' / 'install_cloud_capture.py')
installer = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = installer  # dataclasses resolve their module by name
spec.loader.exec_module(installer)


def run(*argv):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = installer.main([str(a) for a in argv])
    return code, out.getvalue(), err.getvalue()


class Install(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.repo = pathlib.Path(scratch.name)
        (self.repo / '.git').mkdir()
        self.settings = self.repo / '.claude' / 'settings.json'
        self.hook = self.repo / '.claude' / 'hooks' / installer.HOOK_NAME

    def commands(self, event):
        hooks = json.loads(self.settings.read_text())['hooks'][event]
        return [h['command'] for group in hooks for h in group['hooks']]

    def test_the_source_is_this_repositorys_hook(self):
        self.assertEqual(installer.SOURCE, ROOT / '.claude' / 'hooks' / installer.HOOK_NAME)
        self.assertTrue(installer.SOURCE.is_file())

    def test_a_bare_repository_gets_the_hook_on_both_events(self):
        self.assertEqual(run(self.repo)[0], 0)
        self.assertEqual(self.hook.read_text(), installer.SOURCE.read_text())
        for event in installer.EVENTS:
            self.assertEqual(self.commands(event), [installer.COMMAND])

    def test_existing_settings_and_hooks_are_kept_and_nothing_is_added_twice(self):
        self.settings.parent.mkdir()
        mine = {'permissions': {'allow': ['Bash(ls)']},
                'hooks': {'Stop': [{'hooks': [{'type': 'command', 'command': 'echo mine'}]}]}}
        self.settings.write_text(json.dumps(mine))
        run(self.repo)
        run(self.repo)
        settings = json.loads(self.settings.read_text())
        self.assertEqual(settings['permissions'], mine['permissions'])
        self.assertEqual(self.commands('Stop'), ['echo mine', installer.COMMAND])
        self.assertEqual(self.commands('PreCompact'), [installer.COMMAND])

    def test_check_reports_drift_without_writing_then_passes_once_current(self):
        code, out, _ = run('--check', self.repo)
        self.assertEqual(code, 1)
        self.assertIn('stale', out)
        self.assertFalse(self.hook.exists() or self.settings.exists())
        run(self.repo)
        self.assertEqual(run('--check', self.repo)[0], 0)
        self.hook.write_text('# an older copy\n')
        self.assertEqual(run('--check', self.repo)[0], 1)

    def test_a_target_it_cannot_safely_edit_is_refused_untouched(self):
        self.settings.parent.mkdir()
        for broken in ('{not json', '[]', '{"hooks": []}', '{"hooks": null}',
                       '{"hooks": {"Stop": {}}}', '{"hooks": {"Stop": [42]}}',
                       '{"hooks": {"Stop": [{"hooks": "x"}]}}',
                       '{"hooks": {"PreCompact": [{"hooks": [1]}]}}'):
            self.settings.write_text(broken)
            code, _, err = run(self.repo)
            self.assertEqual(code, 2, broken)
            self.assertIn('error:', err)
            self.assertEqual(self.settings.read_text(), broken)
            self.assertFalse(self.hook.exists())

    def test_a_directory_that_is_not_a_repository_root_is_refused(self):
        code, _, err = run(self.repo / 'missing')
        self.assertEqual(code, 2)
        self.assertIn('not a git repository root', err)

    def test_a_registration_that_differs_is_repaired_once_and_other_hooks_stay(self):
        self.settings.parent.mkdir()
        mine = {'type': 'command', 'command': 'echo mine'}
        self.settings.write_text(json.dumps({'hooks': {
            'PreCompact': [{'matcher': 'manual', 'hooks': [mine, dict(installer.HOOK_ENTRY)]}],
            'Stop': [{'hooks': [dict(installer.HOOK_ENTRY, timeout=5)]},
                     {'hooks': [dict(installer.HOOK_ENTRY)]}],
        }}))
        self.assertEqual(run(self.repo)[0], 0)
        written = self.settings.read_text()
        self.assertEqual(run('--check', self.repo)[0], 0)
        run(self.repo)
        self.assertEqual(self.settings.read_text(), written, 'a second run changes nothing')
        hooks = json.loads(self.settings.read_text())['hooks']
        self.assertEqual(hooks['PreCompact'], [{'matcher': 'manual', 'hooks': [mine]},
                                               installer.CANONICAL_GROUP])
        self.assertEqual(hooks['Stop'], [installer.CANONICAL_GROUP])

    def test_a_check_finds_a_registration_that_misses_an_event_or_the_limit(self):
        run(self.repo)
        settings = json.loads(self.settings.read_text())
        settings['hooks']['PreCompact'][0]['matcher'] = 'manual'
        self.settings.write_text(json.dumps(settings))
        before = self.settings.read_text()
        self.assertEqual(run('--check', self.repo)[0], 1)
        self.assertEqual(self.settings.read_text(), before, '--check writes nothing')
        run(self.repo)
        self.assertEqual(run('--check', self.repo)[0], 0)

    def test_symlinked_paths_are_refused_and_nothing_outside_is_touched(self):
        outside = tempfile.TemporaryDirectory()
        self.addCleanup(outside.cleanup)
        elsewhere = pathlib.Path(outside.name)
        sentinel = elsewhere / 'settings.json'
        sentinel.write_text('{"outside": true}')
        claude = self.repo / '.claude'
        cases = [
            (claude, elsewhere),
            (claude / 'hooks', elsewhere),
            (self.settings, sentinel),
            (self.hook, elsewhere / 'planted.py'),  # dangling
        ]
        for link, referent in cases:
            with self.subTest(link=link.name):
                link.parent.mkdir(parents=True, exist_ok=True)
                link.symlink_to(referent)
                code, _, err = run(self.repo)
                self.assertEqual(code, 2)
                self.assertIn('symlink', err)
                self.assertEqual(sentinel.read_text(), '{"outside": true}')
                self.assertEqual(sorted(p.name for p in elsewhere.iterdir()), ['settings.json'])
                link.unlink()

    def test_a_failed_second_replacement_puts_the_first_back(self):
        self.hook.parent.mkdir(parents=True)
        self.hook.write_text('# an older copy\n')
        self.settings.write_text('{"permissions": {}}')
        os.chmod(self.settings, 0o600)
        real = os.replace
        calls = []

        def second_fails(src, dst):
            calls.append(dst)
            if len(calls) == 2:
                raise OSError('disk full')
            return real(src, dst)

        with mock.patch.object(installer.os, 'replace', second_fails):
            code, _, err = run(self.repo)
        self.assertEqual(code, 2)
        self.assertIn('disk full', err)
        self.assertEqual(self.hook.read_text(), '# an older copy\n')
        self.assertEqual(self.settings.read_text(), '{"permissions": {}}')
        self.assertEqual(sorted(p.name for p in self.hook.parent.iterdir()), [installer.HOOK_NAME])
        self.assertEqual(sorted(p.name for p in self.settings.parent.iterdir()),
                         ['hooks', 'settings.json'])

        self.assertEqual(run(self.repo)[0], 0)
        self.assertEqual(stat.S_IMODE(self.settings.stat().st_mode), 0o600, 'mode kept')

    def test_a_failure_before_any_replacement_changes_nothing(self):
        with mock.patch.object(installer.os, 'replace', side_effect=OSError('read-only')):
            self.assertEqual(run(self.repo)[0], 2)
        self.assertFalse(self.hook.exists() or self.settings.exists())
        self.assertEqual(list(self.hook.parent.iterdir()), [])


if __name__ == '__main__':
    unittest.main()
