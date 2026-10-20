import importlib.util
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("stop_wsl_codex", Path(__file__).parents[1] / "scripts/stop_wsl_codex.py")
stopper = importlib.util.module_from_spec(spec)
spec.loader.exec_module(stopper)


class StopWslCodexTests(unittest.TestCase):
    def test_discovery_returns_only_home_paths_and_deduplicates_processes(self):
        target = {"pid": 100, "name": "codex", "parent": 1, "uid": stopper.os.getuid(), "exe": "/opt/custom/codex"}
        child = {**target, "pid": 101}
        with patch.object(stopper, "all_processes", return_value={100: target, 101: child}), patch.object(Path, "read_bytes", return_value=b"CODEX_HOME=/tmp/codex-home\0PRIVATE_TOKEN=never-output\0"), patch.object(stopper, "inspect_process", side_effect=[target, child]):
            self.assertEqual(stopper.discover_homes(), ["/tmp/codex-home"])

    def test_discovery_does_not_open_pidfds_or_send_signals(self):
        with patch.object(sys, "argv", ["helper", "--discover"]), patch.object(stopper, "discover_homes", return_value=["/tmp"]), patch.object(stopper.os, "pidfd_open") as opened, patch.object(stopper.signal, "pidfd_send_signal") as sent:
            self.assertEqual(stopper.main(), 0)
            opened.assert_not_called()
            sent.assert_not_called()

    def test_discovery_detects_conflicting_homes_without_selecting_one(self):
        target = {"pid": 100, "name": "codex", "parent": 1, "uid": stopper.os.getuid(), "exe": "/opt/custom/codex"}
        child = {**target, "pid": 101}
        with patch.object(stopper, "all_processes", return_value={100: target, 101: child}), patch.object(stopper, "process_home", side_effect=["/a", "/b"]), patch.object(stopper, "inspect_process", side_effect=[target, child]):
            self.assertEqual(stopper.discover_homes(), ["/a", "/b"])

    def test_detection_rejects_relative_home_and_changed_process_identity(self):
        with patch.object(Path, "read_bytes", return_value=b"CODEX_HOME=relative\0"):
            with self.assertRaisesRegex(RuntimeError, "non-absolute"):
                stopper.process_home(100)
        target = {"pid": 100, "name": "codex", "parent": 1, "uid": stopper.os.getuid(), "exe": "/opt/custom/codex"}
        with patch.object(stopper, "all_processes", return_value={100: target}), patch.object(stopper, "process_home", return_value="/tmp"), patch.object(stopper, "inspect_process", return_value={**target, "parent": 2}):
            with self.assertRaisesRegex(RuntimeError, "identity changed"):
                stopper.discover_homes()

    def test_trusted_executables(self):
        home = "/home/test/.codex"
        self.assertTrue(stopper.trusted_executable(home + "/packages/standalone/releases/version/bin/codex", home))
        self.assertTrue(stopper.trusted_executable("/mnt/c/Users/Test/.codex/bin/wsl/version/codex", home))
        self.assertTrue(stopper.trusted_executable("/opt/custom/bin/codex", home))
        self.assertFalse(stopper.trusted_executable("/tmp/codex-switcher", home))
        self.assertFalse(stopper.trusted_executable("/mnt/c/Users/Test/.codex/bin/wsl/version/unrelated", home))

    def test_preview_never_opens_process_handles(self):
        with patch.object(sys, "argv", ["stop", "--home", "/tmp"]), patch.object(stopper, "snapshot_targets", return_value=({}, set())), patch.object(stopper, "all_processes", return_value={}), patch.object(stopper.os, "pidfd_open") as opened:
            self.assertEqual(stopper.main(), 0)
            opened.assert_not_called()

    def test_identity_change_prevents_all_signals(self):
        target = {"pid": 100, "name": "codex", "parent": 1, "start": "1", "uid": 1000, "exe": "/test/codex"}
        with patch.object(sys, "argv", ["stop", "--home", "/tmp", "--stop"]), patch.object(stopper, "snapshot_targets", return_value=({100: target}, set())), patch.object(stopper.os, "pidfd_open", return_value=7), patch.object(stopper, "inspect_process", return_value={**target, "start": "2"}), patch.object(stopper.os, "close"), patch.object(stopper.signal, "pidfd_send_signal") as send:
            with self.assertRaisesRegex(RuntimeError, "identity changed"):
                stopper.main()
            send.assert_not_called()

    def test_remaining_process_blocks_success(self):
        blocker = {"pid": 200, "name": "codex", "parent": 1}
        with patch.object(sys, "argv", ["stop", "--home", "/tmp", "--stop"]), patch.object(stopper, "snapshot_targets", return_value=({}, set())), patch.object(stopper, "all_processes", return_value={200: blocker}), patch.object(stopper.time, "sleep"):
            self.assertEqual(stopper.main(), 3)

    def test_only_sigterm_is_sent_to_verified_handle(self):
        target = {"pid": 100, "name": "codex", "parent": 1}
        with patch.object(sys, "argv", ["stop", "--home", "/tmp", "--stop"]), patch.object(stopper, "snapshot_targets", return_value=({100: target}, set())), patch.object(stopper.os, "pidfd_open", return_value=7), patch.object(stopper, "inspect_process", return_value=target), patch.object(stopper.os, "close"), patch.object(stopper.signal, "pidfd_send_signal") as send, patch.object(stopper, "exited", side_effect=[False, True, True]), patch.object(stopper.time, "sleep"), patch.object(stopper, "all_processes", return_value={}):
            self.assertEqual(stopper.main(), 0)
            send.assert_called_once_with(7, stopper.signal.SIGTERM)

    def test_other_home_is_not_selected(self):
        target = {"pid": 100, "name": "codex", "parent": 1, "uid": stopper.os.getuid(), "exe": "/tmp/codex"}
        with patch.object(stopper, "all_processes", return_value={100: target}), patch.object(Path, "read_bytes", return_value=b"CODEX_HOME=/other/home\0"):
            self.assertEqual(stopper.snapshot_targets("/tmp"), ({}, set()))

    def test_process_home_default_and_custom_installation(self):
        target = {"pid": 100, "name": "codex", "parent": 1, "uid": stopper.os.getuid(), "exe": "/opt/custom/codex"}
        with patch.object(stopper, "all_processes", return_value={100: target}), patch.object(Path, "read_bytes", return_value=b"HOME=/tmp/test-user\0"):
            self.assertEqual(stopper.snapshot_targets("/tmp/test-user/.codex"), ({100: target}, set()))

    def test_unknown_executable_prevents_termination(self):
        target = {"pid": 100, "name": "codex", "parent": 1, "uid": stopper.os.getuid(), "exe": "/tmp/other-app"}
        with patch.object(stopper, "all_processes", return_value={100: target}), patch.object(Path, "read_bytes", return_value=b"CODEX_HOME=/tmp\0"):
            with self.assertRaisesRegex(RuntimeError, "Unrecognized executable"):
                stopper.snapshot_targets("/tmp")

    def test_desktop_ancestry_does_not_include_unrelated_apps(self):
        processes = {10: {"parent": 1}, 11: {"parent": 10}, 20: {"parent": 1}}
        self.assertTrue(stopper.belongs_to(11, {10}, processes))
        self.assertFalse(stopper.belongs_to(20, {10}, processes))


if __name__ == "__main__":
    unittest.main()
