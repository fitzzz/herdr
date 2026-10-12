"""Deterministic migration coordinator checks; no running Herdr is addressed."""
import copy
import hashlib
import json
import os
from pathlib import Path
import socket
import stat
import tempfile
import threading
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
            item = next(value for value in self.panes if value["pane_id"] == address)
            if item.get("restore_error"):
                raise migrate.SourceResponseError({"error": {"code": "pane_not_found"}})
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

    def test_explicit_unavailable_placeholder_keeps_identity_and_proves_absence_twice(self):
        placeholder = {**pane(2), "cwd": "/owned-fixture-missing-directory", "label": "saved label",
                       "agent_status": "unknown", "restore_error": migrate.SAVED_DIRECTORY_UNAVAILABLE}
        api = FakeAPI([pane(), placeholder])
        with patch.object(migrate, "process_stamp", self.stamp):
            _, result, _ = migrate.capture(api)
        self.assertEqual(result[1]["terminal_id"], "opaque-2")
        self.assertIsNone(result[1]["child"])
        self.assertEqual(result[1]["unavailable"], {key: placeholder[key] for key in ("cwd", "label", "restore_error")})
        self.assertEqual(sum(method == "pane.process_info" and params["pane_id"] == "wA:p2" for method, params in api.calls), 2)
        api = FakeAPI([pane(), placeholder])
        api.second_snapshot["panes"][1]["label"] = "changed"
        with patch.object(migrate, "process_stamp", self.stamp), self.assertRaisesRegex(ValueError, "identity changed"):
            migrate.capture(api)

    def test_placeholder_does_not_mask_unexpected_runtime_or_other_errors(self):
        placeholder = {**pane(), "cwd": "/owned-fixture-missing-directory", "agent_status": "unknown",
                       "restore_error": migrate.SAVED_DIRECTORY_UNAVAILABLE}
        api = FakeAPI([placeholder])
        original = api.request
        def request(method, params):
            if method == "pane.process_info":
                return {"process_info": {"pane_id": params["pane_id"], "shell_pid": 100}}
            return original(method, params)
        api.request = request
        with self.assertRaisesRegex(ValueError, "unexpectedly has a runtime"):
            migrate.capture(api)
        for code in ("invalid_params", "connection_lost"):
            with self.subTest(code=code):
                with patch.object(api, "request", side_effect=migrate.SourceResponseError({"error": {"code": code}})), self.assertRaises(migrate.SourceResponseError):
                    migrate.verify_runtime_absent(api, "wA:p1")
        for change in ({"restore_error": "another restore failure"}, {"agent_session": {"kind": "id", "value": "native"}}):
            with self.subTest(change=change), self.assertRaises(ValueError):
                migrate.identity_table([{**placeholder, **change}], [])


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


class SourceAPITests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="herdr-migration-api-test-")
        self.path = Path(self.temporary.name) / "fixture.sock"
        self.listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.listener.bind(str(self.path))
        self.listener.listen(1)
        self.api = migrate.SourceAPI(self.path, migrate.sha("/proc/self/exe"), {})

    def tearDown(self):
        self.listener.close()
        self.temporary.cleanup()

    def request_fixture(self, response):
        observed = []
        def serve():
            with self.listener.accept()[0] as connection:
                with connection.makefile("rb") as reader:
                    line = reader.readline()
                if line:
                    request = migrate.decode(line)
                    observed.append(request)
                    connection.sendall(response(request))
        worker = threading.Thread(target=serve)
        worker.start()
        try:
            with patch.object(migrate, "source_context", return_value={}):
                result = self.api.request("pane.list", {})
            return result, observed
        finally:
            worker.join()

    def test_raw_request_authenticates_peer_and_keeps_exact_request_id(self):
        result, requests = self.request_fixture(lambda request: migrate.encoded({"id": request["id"], "result": {"panes": []}}))
        self.assertEqual(result, {"panes": []})
        self.assertEqual(requests[0]["method"], "pane.list")
        self.assertEqual(self.api.source["pid"], os.getpid())

    def test_wrong_response_id_is_not_accepted(self):
        with self.assertRaisesRegex(ValueError, "source API rejected"):
            self.request_fixture(lambda _request: migrate.encoded({"id": "another-request", "result": {"panes": []}}))

    def test_replaced_socket_or_wrong_executable_is_not_accepted(self):
        left, right = socket.socketpair()
        try:
            with patch.object(migrate, "source_context", return_value={}):
                self.api.expected_hash = "0" * 64
                with self.assertRaisesRegex(ValueError, "artifact/session context"):
                    self.api.authenticate(left)
                self.api.expected_hash = migrate.sha("/proc/self/exe")
                replacement = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                replacement_path = self.path.with_name("replacement.sock")
                try:
                    replacement.bind(str(replacement_path))
                    self.path.unlink()
                    replacement_path.rename(self.path)
                    with self.assertRaisesRegex(ValueError, "socket/peer ownership"):
                        self.api.authenticate(left)
                finally:
                    replacement.close()
        finally:
            left.close()
            right.close()


if __name__ == "__main__":
    unittest.main()
