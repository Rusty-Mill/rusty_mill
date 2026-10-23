"""Offline cache/trust/retention fixtures: no APT, sudo, host installation or network."""

from contextlib import redirect_stdout
import hashlib
import importlib.util
import io
from pathlib import Path
import re
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch


ACTION = Path(__file__).parents[1] / "actions/setup-build-env"
SPEC = importlib.util.spec_from_file_location("apt_download_cache", ACTION / "apt_download_cache.py")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
DEB = "example_1%3a2.0-1_amd64.deb"
DATA = b"authenticated archive fixture"
INDEX = {"example_1:2.0-1_amd64.deb": {(len(DATA), hashlib.sha256(DATA).hexdigest())}}


class CacheKeyTests(unittest.TestCase):
    def test_release_architecture_package_list_and_day_are_partitioned(self) -> None:
        key, prefix = MODULE.cache_key("24.04", "amd64", ["a", "b"], "20261022")
        for release, arch, names, day in (
            ("22.04", "amd64", ["a", "b"], "20261022"),
            ("24.04", "arm64", ["a", "b"], "20261022"),
            ("24.04", "amd64", ["a", "c"], "20261022"),
            ("24.04", "amd64", ["a", "b"], "20261023"),
        ):
            with self.subTest(release=release, arch=arch, names=names, day=day):
                self.assertNotEqual(key, MODULE.cache_key(release, arch, names, day)[0])
        self.assertEqual(prefix, MODULE.cache_key("24.04", "amd64", ["b", "a"], "20261023")[1])
        self.assertTrue(key.startswith(prefix))
        with self.assertRaises(ValueError):
            MODULE.cache_key("24.04\npoison", "amd64", ["a"], "20261022")

    def test_only_hosted_actions_consume_and_never_pull_request_target(self) -> None:
        env = {"GITHUB_ACTIONS": "true", "RUNNER_ENVIRONMENT": "github-hosted"}
        for event in ("schedule", "workflow_dispatch", "pull_request", "merge_group"):
            self.assertTrue(MODULE.can_cache({**env, "GITHUB_EVENT_NAME": event}))
        for override in ({"GITHUB_ACTIONS": "false"}, {"RUNNER_ENVIRONMENT": "self-hosted"},
                         {"GITHUB_EVENT_NAME": "pull_request_target"}, {"GITHUB_EVENT_NAME": "push"}):
            self.assertFalse(MODULE.can_cache({**env, "GITHUB_EVENT_NAME": "schedule", **override}))

    def test_package_list_is_the_unchanged_native_install_set(self) -> None:
        self.assertEqual(MODULE.packages(ACTION / "native-packages.txt"), [
            "libxkbcommon-dev", "libwayland-dev", "libx11-dev", "libxrandr-dev", "libxi-dev",
            "libxcursor-dev", "libxinerama-dev", "libgl1-mesa-dev", "protobuf-compiler", "dbus",
            "dbus-x11", "gnome-keyring", "libasound2-dev", "libwebkit2gtk-4.1-dev", "libgtk-3-dev",
            "libayatana-appindicator3-dev", "librsvg2-dev", "libdbus-1-dev",
        ])


class IntegrityTests(unittest.TestCase):
    def test_only_matching_current_version_architecture_size_and_sha256_are_copied(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            source, target = Path(tmp) / "restored", Path(tmp) / "root-archives"
            source.mkdir()
            target.mkdir()
            for name, data in ((DEB, DATA), ("example_1.0_amd64.deb", DATA),
                               ("example_1%3a2.0-1_arm64.deb", DATA), ("bad.deb", DATA), ("lock", b"state")):
                (source / name).write_bytes(data)
            (source / "partial").mkdir()
            self.assertEqual(MODULE.copy_verified(source, target, INDEX), 1)
            self.assertEqual([p.name for p in target.iterdir()], [DEB])
            (source / DEB).write_bytes(b"x" * len(DATA))  # Same size is insufficient.
            (target / DEB).unlink()
            self.assertEqual(MODULE.copy_verified(source, target, INDEX), 0)
            self.assertEqual(list(target.iterdir()), [])
            (source / DEB).write_bytes(DATA + b"extra")
            self.assertEqual(MODULE.copy_verified(source, target, INDEX), 0)

    def test_symlinks_are_not_copied(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            source, target = Path(tmp) / "source", Path(tmp) / "target"
            source.mkdir()
            target.mkdir()
            (source / DEB).write_bytes(DATA)
            with patch.object(Path, "is_symlink", lambda path: path.name == DEB):
                self.assertEqual(MODULE.copy_verified(source, target, INDEX), 0)
            self.assertEqual(list(target.iterdir()), [])

    def test_metadata_uses_each_records_trusted_source_not_any_trusted_origin(self) -> None:
        good = {"Package": "example", "Version": "1:2.0-1", "Architecture": "amd64",
                "Size": str(len(DATA)), "SHA256": hashlib.sha256(DATA).hexdigest()}
        bad = {**good, "SHA256": "0" * 64}

        class Records:
            def lookup(self, location):
                self.record = good if location[0] == "signed" else bad
                return True
            def __contains__(self, key):
                return key in self.record
            def __getitem__(self, key):
                return self.record[key]

        sources = SimpleNamespace(read_main_list=lambda: None,
                                  find_index=lambda file: None if file == "status" else SimpleNamespace(is_trusted=file == "signed"))
        cache = SimpleNamespace(packages=[SimpleNamespace(name="example", version_list=[
            SimpleNamespace(file_list=[("unsigned", 0), ("status", 0), ("signed", 0)]),
        ])])
        apt = SimpleNamespace(init=lambda: None, Cache=lambda: cache, PackageRecords=lambda _: Records(),
                              SourceList=lambda: sources)
        with patch.dict(sys.modules, {"apt_pkg": apt}):
            self.assertEqual(MODULE.trusted_index({DEB}), INDEX)


class InstallationTests(unittest.TestCase):
    def simulate(self, warm: bool = False, newer: bool = False, retain: bool = True,
                 fail: bool = False, unavailable: bool = False) -> tuple[list[list[str]], str, str]:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            staging, archives, output = root / "cache", root / "archives", root / "output"
            staging.mkdir()
            if warm:
                (staging / DEB).write_bytes(DATA)
            current = "example_3.0-1_amd64.deb" if newer else DEB
            index = {current.replace("%3a", ":"): {(len(DATA), hashlib.sha256(DATA).hexdigest())}}
            if newer:
                index.update(INDEX)  # Old versions can still exist in signed indexes.
            calls = []

            def mkdir(**kwargs):
                archives.mkdir()
                return str(archives)

            def run(command, **kwargs):
                calls.append(command)
                if "--print-uris" in command:
                    text = "" if (archives / current).exists() else f"'https://archive.example/{current}' {current} {len(DATA)} MD5Sum:unused\n"
                    return subprocess.CompletedProcess(command, 0, text)
                if "install" in command:
                    if fail:
                        raise subprocess.CalledProcessError(100, command)
                    if retain:
                        (archives / current).write_bytes(DATA)
                return subprocess.CompletedProcess(command, 0)

            stream = io.StringIO()
            effect = ImportError("no apt_pkg") if unavailable else None
            with patch.object(MODULE.tempfile, "mkdtemp", side_effect=mkdir), \
                 patch.object(MODULE.shutil, "chown"), patch.object(MODULE.subprocess, "run", side_effect=run), \
                 patch.object(MODULE, "trusted_index", return_value=index, side_effect=effect), redirect_stdout(stream):
                if fail:
                    with self.assertRaises(subprocess.CalledProcessError):
                        MODULE.install(["example"], staging, output)
                    self.assertFalse(output.exists(), "failed installs cannot produce save outputs")
                else:
                    MODULE.install(["example"], staging, output)
            return calls, stream.getvalue(), output.read_text() if output.exists() else ""

    def test_cold_and_warm_still_update_and_install_but_reuse_only_verified_downloads(self) -> None:
        for warm in (False, True):
            calls, log, output = self.simulate(warm=warm)
            self.assertEqual(calls[0], ["apt-get", "update"])
            self.assertIn("install", calls[-1])
            self.assertNotIn("--print-uris", calls[-1])
            self.assertIn("APT::Keep-Downloaded-Packages=true", calls[-1])
            self.assertIn("Binary::apt-get::APT::Keep-Downloaded-Packages=true", calls[-1])
            for command in calls:
                self.assertNotIn("--allow-unauthenticated", command)
                self.assertFalse(any("AllowInsecure" in option or "AllowUnauthenticated" in option
                                     or "trusted=yes" in option for option in command))
            self.assertIn(f"verified-reused={int(warm)}", log)
            self.assertIn(f"planned-downloads={int(not warm)}", log)
            self.assertIn("retained=true", output)

    def test_new_security_version_is_downloaded_despite_an_old_cache(self) -> None:
        _, log, _ = self.simulate(warm=True, newer=True)
        self.assertIn("verified-reused=1", log)
        self.assertIn("planned-downloads=1", log)

    def test_retention_failure_disables_save_without_hiding_successful_install(self) -> None:
        _, log, output = self.simulate(retain=False)
        self.assertIn("retained=false", output)
        self.assertIn("::warning::", log)

    def test_dependency_install_failure_is_not_masked(self) -> None:
        self.simulate(fail=True)

    def test_failed_authenticated_update_stops_before_cache_verification_or_install(self) -> None:
        with patch.object(MODULE.subprocess, "run", side_effect=subprocess.CalledProcessError(100, ["apt-get", "update"])) as run, \
             patch.object(MODULE, "trusted_index") as verify, patch.object(MODULE, "copy_verified") as copy:
            with self.assertRaises(subprocess.CalledProcessError):
                MODULE.install(["example"], Path("unused-restored-cache"), Path("unused-output"))
            run.assert_called_once_with(["apt-get", "update"], check=True)
            verify.assert_not_called()
            copy.assert_not_called()

    def test_missing_verifier_falls_back_to_normal_downloads(self) -> None:
        calls, log, _ = self.simulate(warm=True, unavailable=True)
        self.assertIn("::warning::", log)
        self.assertIn("verified-reused=0", log)
        self.assertIn("install", calls[-1])

    def test_self_hosted_path_uses_normal_update_and_install_without_caching(self) -> None:
        with patch.object(MODULE.subprocess, "run") as run:
            MODULE.install(["example"], None, Path("unused"))
            self.assertEqual([call.args[0] for call in run.call_args_list],
                             [["apt-get", "update"], ["apt-get", "install", "-y", "example"]])

    def test_export_contains_only_verified_archives_not_lock_partial_or_system_state(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            archives, staging, output = Path(tmp) / "archives", Path(tmp) / "cache", Path(tmp) / "output"
            archives.mkdir()
            staging.mkdir()
            (archives / DEB).write_bytes(DATA)
            (archives / "lock").write_text("lock")
            (archives / "partial").mkdir()
            (staging / "stale.deb").write_bytes(b"obsolete")
            with patch.object(MODULE, "trusted_index", return_value=INDEX):
                MODULE.export(archives, staging, output)
            self.assertEqual([p.name for p in staging.iterdir()], [DEB])
            self.assertIn("cacheable=true", output.read_text())


class ActionPolicyTests(unittest.TestCase):
    def test_failed_setup_empty_exports_and_exact_hits_cannot_save(self) -> None:
        action = (ACTION / "action.yml").read_text()
        block = action.split("    - name: Save downloaded Ubuntu packages", 1)[1].split("continue-on-error:", 1)[0]
        condition = " ".join(block.split("if: >-", 1)[1].split())
        for success, cacheable, hit in ((False, "true", "false"), (True, "false", "false"), (True, "true", "true")):
            expression = condition
            values = {"success()": repr(success), "steps.apt-export.outputs.cacheable": repr(cacheable),
                      "steps.apt-restore.outputs.cache-hit": repr(hit), "github.event_name": "'schedule'",
                      "github.ref": "'refs/heads/main'", "github.repository": "'Rusty-Mill/rusty_mill'"}
            for name, value in values.items():
                expression = expression.replace(name, value)
            self.assertFalse(eval(expression.replace("&&", " and ").replace("||", " or "), {"__builtins__": {}}))

    def test_save_condition_allows_only_successful_trusted_main_sweeps(self) -> None:
        action = (ACTION / "action.yml").read_text()
        condition = action.split("    - name: Save downloaded Ubuntu packages", 1)[1].split("continue-on-error:", 1)[0]
        condition = " ".join(condition.split("if: >-", 1)[1].split())
        for event in ("schedule", "workflow_dispatch", "pull_request", "merge_group", "pull_request_target", "push"):
            for ref in ("refs/heads/main", "refs/heads/feature", "refs/pull/1/merge"):
                for repo in ("Rusty-Mill/rusty_mill", "fork/rusty_mill"):
                    values = {"success()": "True", "steps.apt-export.outputs.cacheable": "'true'",
                              "steps.apt-restore.outputs.cache-hit": "'false'", "github.event_name": repr(event),
                              "github.ref": repr(ref), "github.repository": repr(repo)}
                    expression = condition
                    for name, value in values.items():
                        expression = expression.replace(name, value)
                    actual = eval(expression.replace("&&", " and ").replace("||", " or "), {"__builtins__": {}})
                    self.assertEqual(actual, event in {"schedule", "workflow_dispatch"} and
                                     ref == "refs/heads/main" and repo == "Rusty-Mill/rusty_mill")

    def test_official_restore_and_save_are_optional_but_installation_is_blocking(self) -> None:
        action = (ACTION / "action.yml").read_text()
        for operation in ("restore", "save"):
            self.assertIn(f"actions/cache/{operation}@caa296126883cff596d87d8935842f9db880ef25", action)
        self.assertEqual(action.count("path: ${{ steps.apt-key.outputs.path }}/*.deb"), 2)
        native = action.split("    - name: Install native build dependencies", 1)[1].split("    - name:", 1)[0]
        self.assertNotIn("cache-hit", native)
        self.assertNotIn("continue-on-error", native)
        self.assertIn("if: runner.os == 'Linux'", native)
        self.assertLess(action.index("Save downloaded Ubuntu packages"), action.index("Start a D-Bus session"))
        self.assertIn("SEGMENT_DOWNLOAD_TIMEOUT_MINS: '1'", action)


if __name__ == "__main__":
    unittest.main()
