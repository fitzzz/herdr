"""Deterministic migration coordinator checks; no running Herdr is addressed."""
import copy
import hashlib
import json
import os
from pathlib import Path
import socket
import stat
import tempfile
import unittest
from unittest.mock import patch

from scripts.fast_api import migrate


def pane(index=1):
    return {"pane_id": f"wA:p{index}", "terminal_id": f"opaque-{index}",
            "workspace_id": "wA", "tab_id": "wA:tD", "agent_session": None}


def snapshot(panes):
    return {"panes": copy.deepcopy(panes), "agents": [],
            "workspaces": [{"workspace_id": "wA"}],
            "tabs": [{"tab_id": "wA:tD", "workspace_id": "wA"}],
            "layouts": [{"tab_id": "wA:tD", "workspace_id": "wA",
                         "panes": [{"pane_id": item["pane_id"]} for item in panes]}]}


class FakeAPI:
    def __init__(self, panes):
        self.source = {"uid": 1000}
        self.panes = panes
        self.first_snapshot = snapshot(panes)
        self.second_snapshot = snapshot(panes)
        self.pane_reads = 0
        self.snapshot_reads = 0
        self.calls = []

    def request(self, method, params):
        self.calls.append((method, params))
        if method == "ping":
            return {"version": "0.9.3", "protocol": 16}
        if method == "pane.list":
            self.pane_reads += 1
            return {"panes": self.panes if self.pane_reads == 1 else self.second_snapshot["panes"]}
        if method == "session.snapshot":
            self.snapshot_reads += 1
            return {"snapshot": self.first_snapshot if self.snapshot_reads == 1 else self.second_snapshot}
        if method == "pane.process_info":
            address = params["pane_id"]
            return {"process_info": {"pane_id": address, "shell_pid": 100 + next(index for index, value in enumerate(self.panes) if value["pane_id"] == address)}}
        raise AssertionError("unexpected API method: " + method)


class CaptureTests(unittest.TestCase):
    def stamp(self, pid):
        return {"pid": pid, "uid": 1000, "start_ticks": pid * 10,
                "boot_id": "boot", "pid_namespace": "pid:[1]", "mount_namespace": "mnt:[1]"}

    def test_complete_174_map_retains_ids_and_exact_shell_associations(self):
        api = FakeAPI([pane(index) for index in range(1, 175)])
        with patch.object(migrate, "process_stamp", self.stamp):
            ping, result, agents = migrate.capture(api)
        self.assertEqual(ping["version"], "0.9.3")
        self.assertEqual(len(result), 174)
        self.assertEqual([item["terminal_id"] for item in result], [f"opaque-{index}" for index in range(1, 175)])
        self.assertEqual([item["child"]["pid"] for item in result], list(range(100, 274)))
        self.assertFalse(agents)
        self.assertNotIn("server.live_handoff", [method for method, _ in api.calls])

    def test_same_count_replacement_is_detected_before_handoff(self):
        api = FakeAPI([pane()])
        api.second_snapshot["panes"][0]["terminal_id"] = "replacement"
        with patch.object(migrate, "process_stamp", self.stamp), self.assertRaisesRegex(ValueError, "identity changed"):
            migrate.capture(api)

    def test_move_missing_layout_launch_pending_and_duplicate_ids(self):
        for mutation in ("move", "missing_layout", "launch_pending", "duplicate"):
            with self.subTest(mutation=mutation):
                api = FakeAPI([pane()])
                if mutation == "move":
                    api.second_snapshot["panes"][0]["tab_id"] = "wA:tE"
                elif mutation == "missing_layout":
                    api.second_snapshot["layouts"] = []
                elif mutation == "launch_pending":
                    api.first_snapshot["agents"] = [{**pane(), "launch_pending": True}]
                else:
                    api.panes.append(pane())
                with patch.object(migrate, "process_stamp", self.stamp), self.assertRaises(ValueError):
                    migrate.capture(api)

    def test_processless_and_reused_child_cannot_be_captured(self):
        api = FakeAPI([pane()])
        counter = 0
        def stamp(pid):
            nonlocal counter
            counter += 1
            value = self.stamp(pid)
            value["start_ticks"] += counter
            return value
        with patch.object(migrate, "process_stamp", stamp), self.assertRaisesRegex(ValueError, "changed after capture"):
            migrate.capture(api)
        with self.assertRaises(ValueError):
            migrate.process_stamp(0)

    def test_native_session_and_name_join_are_exact(self):
        original = pane()
        original["agent_session"] = {"source": "herdr:pi", "agent": "pi", "kind": "path", "value": "/native/saved.jsonl"}
        agent = {**original, "name": "worker", "state_change_seq": 2 ** 64 - 1}
        table = migrate.identity_table([original], [agent])
        self.assertEqual(table[original["pane_id"]]["agent_name"], "worker")
        self.assertEqual(table[original["pane_id"]]["agent_session"], original["agent_session"])
        agent["agent_session"] = None
        with self.assertRaises(ValueError):
            migrate.identity_table([original], [agent])


class ReceiptTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="herdr-migration-test-")
        self.base = Path(self.temporary.name)
        self.candidate = self.base / "candidate"
        self.candidate.write_bytes(b"candidate fixture")
        source = migrate.process_stamp(os.getppid())
        self.receipt = {"version": 1, "operation_id": "fresh-operation", "observation_epoch": "fresh-operation",
                        "source": source, "source_sha256": migrate.sha(f"/proc/{source['pid']}/exe"),
                        "candidate_sha256": migrate.sha(self.candidate), "data_dir": str(self.base)}
        self.private_socket = self.base / f"herdr-handoff-{source['pid']}.sock"
        self.listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.listener.bind(str(self.private_socket))
        self.arguments = ["server", "--handoff-import", str(self.private_socket), "opaque-token"]

    def tearDown(self):
        self.listener.close()
        self.temporary.cleanup()

    def invoke(self, attempt, arguments=None):
        digest = hashlib.sha256((attempt / "receipt.json").read_bytes()).hexdigest()
        return migrate.shim_main(str(attempt), digest, str(self.candidate), self.receipt["candidate_sha256"], self.receipt["operation_id"], arguments or self.arguments)

    def test_frozen_shim_execs_same_process_with_only_explicit_read_fds(self):
        attempt = migrate.make_attempt(self.base, self.receipt, self.candidate)
        before = set(os.listdir("/proc/self/fd"))
        captured = []
        def execute(path, argv, environment):
            captured.append((path, argv, environment))
            self.assertEqual(argv[:5], [str(self.candidate), *self.arguments])
            self.assertEqual(argv[5], "--handoff-identity-fds")
            fds = list(map(int, argv[6:]))
            self.assertEqual(len(set(fds)), 3)
            self.assertTrue(all(os.get_inheritable(fd) for fd in fds))
            claim = migrate.decode(migrate.read_fd(fds[1]))
            self.assertEqual(claim["importer"]["pid"], os.getpid())
            self.assertEqual(claim["token"], "opaque-token")
            self.assertEqual(claim["receipt_sha256"], hashlib.sha256(migrate.read_fd(fds[0])).hexdigest())
            raise SystemExit(0)
        try:
            with patch.object(migrate.os, "execve", execute), self.assertRaises(SystemExit):
                self.invoke(attempt)
        finally:
            # exec is mocked, so close only descriptors opened by this call.
            for fd in set(os.listdir("/proc/self/fd")) - before:
                try:
                    os.close(int(fd))
                except OSError:
                    pass
        self.assertEqual(captured[0][2], dict(os.environ))
        self.assertEqual(stat.S_IMODE(os.stat(attempt).st_mode), 0o700)
        self.assertEqual(stat.S_IMODE(os.stat(attempt / "receipt.json").st_mode), 0o400)
        self.assertEqual(stat.S_IMODE(os.stat(attempt / "import-shim").st_mode), 0o500)
        self.assertIn("shim_main(", (attempt / "import-shim").read_text())
        with self.assertRaises(FileExistsError):
            self.invoke(attempt)

    def test_bad_arguments_and_substituted_candidate_create_no_claim(self):
        attempt = migrate.make_attempt(self.base, self.receipt, self.candidate)
        with self.assertRaisesRegex(ValueError, "unexpected handoff shim arguments"):
            self.invoke(attempt, self.arguments + ["extra"])
        self.assertFalse((attempt / "claimed.json").exists())
        second = migrate.make_attempt(self.base, self.receipt, self.candidate)
        self.candidate.write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "candidate differs"):
            self.invoke(second)
        self.assertFalse((second / "claimed.json").exists())

    def test_symlink_receipt_and_replay_cannot_replace_fixed_map(self):
        attempt = migrate.make_attempt(self.base, self.receipt, self.candidate)
        directory_fd = os.open(attempt, os.O_RDONLY | os.O_DIRECTORY)
        try:
            with self.assertRaises(FileExistsError):
                migrate.exclusive_write(directory_fd, "receipt.json", b"replacement")
            outside = self.base / "outside"
            outside.write_bytes(b"fixture")
            os.symlink(outside, attempt / "symlink.json")
            with self.assertRaises(OSError):
                migrate.checked_open(directory_fd, "symlink.json")
        finally:
            os.close(directory_fd)

    def test_duplicate_keys_and_integer_precision(self):
        with self.assertRaises(ValueError):
            migrate.decode(b'{"map":{"id":"old","id":"new"}}')
        self.assertEqual(migrate.decode(b'{"sequence":18446744073709551615}')["sequence"], 2 ** 64 - 1)


if __name__ == "__main__":
    unittest.main()
