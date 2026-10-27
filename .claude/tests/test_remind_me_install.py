"""install_cloud_capture.py: puts the capture hook into another repository (ADR-0029).

Synthetic only: scratch repositories in a temporary directory.
Run: python3 -m unittest discover -s .claude/tests -v
"""
import contextlib
import importlib.util
import io
import json
import pathlib
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location(
    'install_cloud_capture',
    ROOT / 'crates' / 'apps' / 'rusty_remind_me' / 'scripts' / 'install_cloud_capture.py')
installer = importlib.util.module_from_spec(spec)
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
        for broken in ('{not json', '[]', '{"hooks": []}', '{"hooks": {"Stop": {}}}'):
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


if __name__ == '__main__':
    unittest.main()
