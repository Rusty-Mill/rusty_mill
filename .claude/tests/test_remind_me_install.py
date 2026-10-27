"""install_cloud_capture.py: puts the capture hook in user-level Claude settings (ADR-0029).

Synthetic only: a scratch home directory.
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
        self.home = pathlib.Path(scratch.name)
        self.settings = self.home / '.claude' / 'settings.json'
        self.hook = self.home / '.claude' / 'hooks' / installer.HOOK_NAME

    def commands(self, event):
        hooks = json.loads(self.settings.read_text())['hooks'][event]
        return [h['command'] for group in hooks for h in group['hooks']]

    def test_the_source_is_this_repositorys_hook(self):
        self.assertEqual(installer.SOURCE, ROOT / '.claude' / 'hooks' / installer.HOOK_NAME)
        self.assertTrue(installer.SOURCE.is_file())

    def test_a_bare_repository_gets_the_hook_on_both_events(self):
        self.assertEqual(run('--home', self.home)[0], 0)
        self.assertEqual(self.hook.read_text(), installer.SOURCE.read_text())
        for event in installer.EVENTS:
            self.assertEqual(self.commands(event), [installer.COMMAND])

    def test_existing_settings_and_hooks_are_kept_and_nothing_is_added_twice(self):
        self.settings.parent.mkdir()
        mine = {'permissions': {'allow': ['Bash(ls)']},
                'hooks': {'Stop': [{'hooks': [{'type': 'command', 'command': 'echo mine'}]}]}}
        self.settings.write_text(json.dumps(mine))
        run('--home', self.home)
        run('--home', self.home)
        settings = json.loads(self.settings.read_text())
        self.assertEqual(settings['permissions'], mine['permissions'])
        self.assertEqual(self.commands('Stop'), ['echo mine', installer.COMMAND])
        self.assertEqual(self.commands('PreCompact'), [installer.COMMAND])

    def test_check_reports_drift_without_writing_then_passes_once_current(self):
        code, out, _ = run('--check', '--home', self.home)
        self.assertEqual(code, 1)
        self.assertIn('stale', out)
        self.assertFalse(self.hook.exists() or self.settings.exists())
        run('--home', self.home)
        self.assertEqual(run('--check', '--home', self.home)[0], 0)
        self.hook.write_text('# an older copy\n')
        self.assertEqual(run('--check', '--home', self.home)[0], 1)

    def test_a_target_it_cannot_safely_edit_is_refused_untouched(self):
        self.settings.parent.mkdir()
        for broken in ('{not json', '[]', '{"hooks": []}', '{"hooks": null}',
                       '{"hooks": {"Stop": {}}}', '{"hooks": {"Stop": [42]}}',
                       '{"hooks": {"Stop": [{"hooks": "x"}]}}',
                       '{"hooks": {"PreCompact": [{"hooks": [1]}]}}',
                       '{"hooks": {"Stop": [{"hooks": [{}]}]}}',
                       '{"hooks": {"Stop": [{"hooks": [{"command": "x"}]}]}}',
                       '{"hooks": {"Stop": [{"hooks": [{"type": "command"}]}]}}',
                       '{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": 7}]}]}}',
                       '{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "x", "timeout": "30"}]}]}}',
                       '{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "x", "timeout": true}]}]}}',
                       '{"hooks": {"Stop": [{"hooks": [{"type": "bogus"}]}]}}',
                       '{"hooks": {"Stop": [{"hooks": [{"type": "prompt"}]}]}}',
                       '{"hooks": {"Stop": [{"hooks": [{"type": "agent", "prompt": ""}]}]}}',
                       '{"hooks": {"Stop": [{"hooks": [{"type": "http"}]}]}}',
                       '{"hooks": {"Stop": [{"hooks": [{"type": "http", "url": "u", "headers": {"a": 1}}]}]}}',
                       '{"hooks": {"Stop": [{"hooks": [{"type": "mcp_tool", "server": "s"}]}]}}',
                       '{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "x", "args": [1]}]}]}}'):
            self.settings.write_text(broken)
            code, _, err = run('--home', self.home)
            self.assertEqual(code, 2, broken)
            self.assertIn('error:', err)
            self.assertEqual(self.settings.read_text(), broken)
            self.assertFalse(self.hook.exists())

    def test_a_home_that_is_not_a_directory_is_refused(self):
        code, _, err = run('--home', self.home / 'missing')
        self.assertEqual(code, 2)
        self.assertIn('not a directory', err)

    def test_the_source_can_be_a_downloaded_copy(self):
        download = self.home / 'fetched.py'
        download.write_text('# the hook as fetched\n')
        self.assertEqual(run('--home', self.home, '--source', download)[0], 0)
        self.assertEqual(self.hook.read_text(), '# the hook as fetched\n')
        self.assertEqual(run('--home', self.home, '--source', self.home / 'gone.py')[0], 2)

    def test_the_command_finds_the_hook_under_home(self):
        self.assertEqual(installer.COMMAND, 'python3 "$HOME"/.claude/hooks/remind-me-capture.py')

    def test_a_registration_that_differs_is_repaired_once_and_other_hooks_stay(self):
        self.settings.parent.mkdir()
        mine = {'type': 'command', 'command': 'echo mine'}
        self.settings.write_text(json.dumps({'hooks': {
            'PreCompact': [{'matcher': 'manual', 'hooks': [mine, dict(installer.HOOK_ENTRY)]}],
            'Stop': [{'hooks': [dict(installer.HOOK_ENTRY, timeout=5)]},
                     {'hooks': [dict(installer.HOOK_ENTRY)]}],
        }}))
        self.assertEqual(run('--home', self.home)[0], 0)
        written = self.settings.read_text()
        self.assertEqual(run('--check', '--home', self.home)[0], 0)
        run('--home', self.home)
        self.assertEqual(self.settings.read_text(), written, 'a second run changes nothing')
        hooks = json.loads(self.settings.read_text())['hooks']
        self.assertEqual(hooks['PreCompact'], [{'matcher': 'manual', 'hooks': [mine]},
                                               installer.CANONICAL_GROUP])
        self.assertEqual(hooks['Stop'], [installer.CANONICAL_GROUP])

    def test_a_check_finds_a_registration_that_misses_an_event_or_the_limit(self):
        run('--home', self.home)
        settings = json.loads(self.settings.read_text())
        settings['hooks']['PreCompact'][0]['matcher'] = 'manual'
        self.settings.write_text(json.dumps(settings))
        before = self.settings.read_text()
        self.assertEqual(run('--check', '--home', self.home)[0], 1)
        self.assertEqual(self.settings.read_text(), before, '--check writes nothing')
        run('--home', self.home)
        self.assertEqual(run('--check', '--home', self.home)[0], 0)

    def test_symlinked_paths_are_refused_and_nothing_outside_is_touched(self):
        outside = tempfile.TemporaryDirectory()
        self.addCleanup(outside.cleanup)
        elsewhere = pathlib.Path(outside.name)
        sentinel = elsewhere / 'settings.json'
        sentinel.write_text('{"outside": true}')
        claude = self.home / '.claude'
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
                code, _, err = run('--home', self.home)
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
            code, _, err = run('--home', self.home)
        self.assertEqual(code, 2)
        self.assertIn('disk full', err)
        self.assertEqual(self.hook.read_text(), '# an older copy\n')
        self.assertEqual(self.settings.read_text(), '{"permissions": {}}')
        self.assertEqual(sorted(p.name for p in self.hook.parent.iterdir()), [installer.HOOK_NAME])
        self.assertEqual(sorted(p.name for p in self.settings.parent.iterdir()),
                         ['hooks', 'settings.json'])

        self.assertEqual(run('--home', self.home)[0], 0)
        self.assertEqual(stat.S_IMODE(self.settings.stat().st_mode), 0o600, 'mode kept')

    def test_a_failure_before_any_replacement_changes_nothing(self):
        with mock.patch.object(installer.os, 'replace', side_effect=OSError('read-only')):
            self.assertEqual(run('--home', self.home)[0], 2)
        self.assertFalse(self.hook.exists() or self.settings.exists())
        self.assertEqual(list(self.hook.parent.iterdir()), [])


    def test_other_valid_handler_types_are_kept(self):
        self.settings.parent.mkdir()
        others = [
            {'type': 'prompt', 'prompt': 'summarise', 'model': 'm'},
            {'type': 'agent', 'prompt': 'check', 'timeout': 60},
            {'type': 'http', 'url': 'https://example.test/h', 'headers': {'X': '$V'},
             'allowedEnvVars': ['V'], 'onFailure': 'continue'},
            {'type': 'mcp_tool', 'server': 's', 'tool': 't', 'input': {'a': 1}},
            {'type': 'command', 'command': 'node', 'args': ['x.js'], 'async': True, 'shell': 'bash'},
        ]
        self.settings.write_text(json.dumps({'hooks': {'Stop': [{'hooks': others}]}}))
        self.assertEqual(run('--home', self.home)[0], 0)
        stop = json.loads(self.settings.read_text())['hooks']['Stop']
        self.assertEqual(stop, [{'hooks': others}, installer.CANONICAL_GROUP])

    def test_a_refusal_never_echoes_a_handler_value(self):
        self.settings.parent.mkdir()
        secret = 'sk-SENTINEL-' + 'x' * 24
        for broken in (
            {'type': 'command', 'command': f'curl -H "Authorization: {secret}"', 'timeout': 'soon'},
            {'type': 'http', 'url': f'https://u/?k={secret}', 'headers': {'Authorization': 7}},
            {'type': f'{secret}'},
        ):
            text = json.dumps({'hooks': {'Stop': [{'hooks': [broken]}]}})
            self.settings.write_text(text)
            for args in (('--home', self.home), ('--check', '--home', self.home)):
                code, out, err = run(*args)
                self.assertEqual(code, 2)
                self.assertNotIn('SENTINEL', out + err)
                self.assertIn('hooks.Stop[0].hooks[0]', err)
                self.assertEqual(self.settings.read_text(), text)

    def old_copy_and_settings(self):
        self.hook.parent.mkdir(parents=True)
        self.hook.write_text('# an older copy\n')
        self.settings.write_text('{"permissions": {}}')

    def replace_failing_on(self, *failing):
        real, calls = os.replace, []

        def replace(src, dst):
            calls.append(dst)
            if len(calls) in failing:
                raise OSError(f'replace {len(calls)} failed')
            return real(src, dst)
        return mock.patch.object(installer.os, 'replace', replace)

    def test_rollback_still_happens_when_cleanup_fails(self):
        self.old_copy_and_settings()
        with self.replace_failing_on(2), \
                mock.patch.object(pathlib.Path, 'unlink', side_effect=OSError('busy')):
            code, _, err = run('--home', self.home)
        self.assertEqual(code, 2)
        self.assertEqual(self.hook.read_text(), '# an older copy\n')
        self.assertEqual(self.settings.read_text(), '{"permissions": {}}')
        self.assertIn('warning: could not remove', err)

    def test_a_failed_rollback_names_the_recovery_copy_it_kept(self):
        self.old_copy_and_settings()
        with self.replace_failing_on(2, 3):
            code, _, err = run('--home', self.home)
        self.assertEqual(code, 2)
        self.assertIn('rollback incomplete', err)
        kept = [p for p in self.hook.parent.iterdir() if p.name != installer.HOOK_NAME]
        self.assertEqual(len(kept), 1)
        self.assertIn(str(kept[0]), err)
        self.assertEqual(kept[0].read_text(), '# an older copy\n')

    def test_a_staging_failure_changes_nothing_and_leaves_nothing(self):
        self.old_copy_and_settings()
        real, calls = installer.stage, []

        def third_fails(change):
            calls.append(change)
            if len(calls) == 3:
                raise OSError('no space')
            return real(change)

        with mock.patch.object(installer, 'stage', third_fails):
            self.assertEqual(run('--home', self.home)[0], 2)
        self.assertEqual(self.hook.read_text(), '# an older copy\n')
        self.assertEqual(self.settings.read_text(), '{"permissions": {}}')
        self.assertEqual([p.name for p in self.hook.parent.iterdir()], [installer.HOOK_NAME])
        self.assertEqual(sorted(p.name for p in self.settings.parent.iterdir()),
                         ['hooks', 'settings.json'])


if __name__ == '__main__':
    unittest.main()
