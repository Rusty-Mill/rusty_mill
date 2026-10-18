"""Execute every verifier module and reach its subprocess boundary without a game."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

BOT = Path(__file__).resolve().parent
MODULES = (
    'ball_events', 'duel_report', 'fuzz_report', 'kstep_regimes',
    'onestep_episodes', 'regime_bias', 'score_dir', 'spread_report',
    'touchdown_errors', 'two_wheel_errors', 'wheel_facets', 'wheel_kstep',
)


class CalledVerifier(Exception):
    """Stop at the external verifier boundary, after executing module code."""


class VerifyPath(unittest.TestCase):
    def test_import_and_subprocess_argument_for_every_module(self) -> None:
        with tempfile.TemporaryDirectory(prefix='verifier path ') as directory:
            batch = Path(directory)
            capture = batch / 'duel_801_run1.jsonl'
            car = {'position': dict(x=0, y=0, z=0),
                   'velocity': dict(x=0, y=0, z=0),
                   'rotation': dict(x=0, y=0, z=0, w=1)}
            capture.write_text('\n'.join(json.dumps({
                'timestamp_secs': tick / 120, 'cars': [car, car],
            }) for tick in range(16)), encoding='utf-8')
            scenario = BOT / 'experiments' / 'duel_801.json'
            custom = str(batch / 'custom target' / 'rb-verify.exe')
            for value in (None, '', custom):
                for name in MODULES:
                    with self.subTest(module=name, RB_VERIFY=value):
                        environment = dict(os.environ)
                        environment.pop('RB_VERIFY', None)
                        if value is not None:
                            environment['RB_VERIFY'] = value
                        with patch.dict(os.environ, environment, clear=True):
                            spec = importlib.util.spec_from_file_location(name, BOT / f'{name}.py')
                            module = importlib.util.module_from_spec(spec)
                            spec.loader.exec_module(module)
                        expected = value or str(BOT.parents[1] / 'target' / 'release' / 'rb-verify.exe')
                        self.assertEqual(Path(module.VERIFY), Path(expected))
                        def intercept(argv: list, **kwargs: object) -> None:
                            self.assertIsInstance(argv, list)
                            self.assertEqual(Path(argv[0]), Path(expected))
                            self.assertGreater(len(argv), 1)
                            self.assertFalse(kwargs.get('shell', False))
                            raise CalledVerifier
                        with patch('subprocess.run', side_effect=intercept), patch.object(
                            sys, 'argv', [name, str(batch)]
                        ), contextlib.redirect_stdout(io.StringIO()):
                            with self.assertRaises(CalledVerifier):
                                if name == 'ball_events':
                                    module.ticks_of(capture)
                                elif name == 'fuzz_report':
                                    module.rows_of(scenario, capture)
                                elif name == 'onestep_episodes':
                                    list(module.episodes(capture, 1, 0))
                                elif name == 'score_dir':
                                    module.score(scenario, capture)
                                elif name == 'spread_report':
                                    module.port_mean(scenario, capture)
                                elif name == 'wheel_facets':
                                    module.trace(str(capture))
                                elif name == 'wheel_kstep':
                                    module.rows_of(str(capture), 4)
                                elif name == 'kstep_regimes':
                                    with patch.object(sys, 'argv', [name, '4', str(capture)]):
                                        module.main()
                                else:
                                    module.main()


if __name__ == '__main__':
    unittest.main()
