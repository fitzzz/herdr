"""Deterministic harness contracts. Does not start Herdr or a provider."""

import argparse
import contextlib
import io
import importlib.util
import json
import os
from pathlib import Path
import shutil
import selectors
import socket
import sys
import subprocess
import tempfile
import threading
import unittest
from unittest.mock import patch

if sys.platform == "linux":
    from scripts import responsiveness_bench as bench


@unittest.skipUnless(sys.platform == "linux", "remote harness targets Linux")
class MeasurementsTest(unittest.TestCase):
    def test_empty_latency_has_no_fabricated_zero(self):
        self.assertEqual(bench.distribution([]), {
            "count": 0, "p50": None, "p95": None, "p99": None, "max": None})

    def test_tail_percentiles_retain_an_outlier(self):
        values = list(range(1, 100)) + [500]
        result = bench.distribution(values)
        self.assertEqual(result, {"count": 100, "p50": 50, "p95": 95, "p99": 99, "max": 500})

    def test_child_processed_input_is_distinct_from_visible_echo(self):
        samples = bench.Samples()
        with patch.object(bench.time, "monotonic_ns", side_effect=[100, 200]):
            samples.send(1, "direct", 90)
            samples.send(2, "direct", 190)
        samples.observe("child_write", [1, 2], 300)
        samples.observe("direct", [1], 500)
        report = samples.report()
        self.assertEqual(report["direct_to_child_write_ms"]["count"], 2)
        self.assertEqual(report["direct_to_direct_ms"]["count"], 1)
        self.assertEqual(report["direct_to_direct_ms"]["unobserved_at_end"], 1)
        self.assertEqual(report["direct_to_child_write_ms"]["unobserved_at_end"], 0)
        self.assertAlmostEqual(report["direct_to_direct_ms"]["max"], .0004)

    def test_repaints_do_not_count_as_duplicate_input_receipts(self):
        samples = bench.Samples()
        samples.send(1, "tui", 0)
        samples.observe("tui", [1], 100)
        samples.observe("tui", [1], 200)
        samples.observe("child_write", [1], 100)
        samples.observe("child_write", [1], 200)
        report = samples.report()
        self.assertEqual(report["tui_to_tui_ms"]["count"], 1)
        self.assertEqual(report["child_duplicate_receipts"], 1)

    def test_ordering_evidence_and_observers_remain_separate(self):
        samples = bench.Samples()
        for sequence in (1, 2, 3):
            samples.send(sequence, "tui", 0)
        samples.observe("child_write", [2, 1, 3], 100)
        samples.observe("tui", [1, 2, 3], 100)
        result = samples.report()
        self.assertEqual(result["child_write_order_regressions"], 1)
        self.assertEqual(result["tui_order_regressions"], 0)
        self.assertIsNone(samples.rows()[0]["direct_ns"])

    def test_consumption_ledger_distinguishes_loss_duplicates_order_and_bad_packets(self):
        sent = [{"sequence": index} for index in (1, 2, 3, 4)]
        ledger = [{"pane": "owned", "sequence": index} for index in (2, 1, 2, 5)]
        ledger.append({"pane": "owned", "malformed_input": True})
        ledger.append({"pane": "other", "sequence": 3})
        result = bench.input_processing(sent, ledger, "owned", True)
        self.assertEqual(result["not_consumed_by_cleanup"], [3, 4])
        self.assertEqual(result["duplicate_consumption"], 1)
        self.assertEqual(result["order_regressions"], 1)
        self.assertEqual(result["unexpected_sequences"], [5])
        self.assertEqual(result["malformed_input_records"], 1)
        self.assertEqual(bench.input_processing(sent, [], "owned", False), {"measured": False})

    def test_scroll_needs_history_movement_in_requested_direction(self):
        before = ("000090", "000091", "000092")
        self.assertFalse(bench.scroll_observed(before, before, -1))
        self.assertFalse(bench.scroll_observed(before, (), -1))
        self.assertFalse(bench.scroll_observed(before, ("000093",), -1))
        self.assertTrue(bench.scroll_observed(before, ("000089",), -1))
        self.assertTrue(bench.scroll_observed(before, ("000093",), 1))

    def test_cpu_counters_preserve_sampling_interval_and_observer_cost(self):
        common = {"phase": "input", "system_ticks": 0, "start_ticks": 5,
                  "rss_kib": 100, "runqueue_wait_ns": 0}
        samples = [
            {**common, "role": "server", "ns": 0, "user_ticks": 0},
            {**common, "role": "server", "ns": 2_000_000_000, "user_ticks": 50,
             "rss_kib": 150, "runqueue_wait_ns": 1_000_000},
            {**common, "role": "harness", "ns": 0, "user_ticks": 0},
        ]
        with patch.object(bench.os, "sysconf", return_value=100):
            results = bench.resource_summary(samples)
        self.assertEqual(results[0]["mean_cpu_percent"], 25)
        self.assertEqual(results[0]["max_rss_kib"], 150)
        self.assertEqual(results[0]["runqueue_wait_ms"], 1)
        self.assertIsNone(results[1]["mean_cpu_percent"])


@unittest.skipUnless(sys.platform == "linux", "remote harness targets Linux")
class IsolationTest(unittest.TestCase):
    def test_routing_and_provider_credentials_are_not_inherited(self):
        inherited = {"HERDR_SOCKET_PATH": "/live/herdr.sock", "HERDR_ENV": "1",
                     "HERDR_CLIENT_SOCKET_PATH": "/live/client.sock", "HERDR_SESSION": "default",
                     "CODEX_HOME": "/live/codex", "ANTHROPIC_API_KEY": "test-sentinel",
                     "SSH_AUTH_SOCK": "/live/ssh.sock"}
        with patch.dict(os.environ, inherited):
            env = bench.isolated_environment(Path("/var/tmp/hdl-owned"), "latency-0123456789ab")
        for key in inherited.keys() - {"HERDR_SESSION"}:
            self.assertNotIn(key, env)
        self.assertEqual(env["HERDR_SESSION"], "latency-0123456789ab")
        self.assertEqual(env["HOME"], "/var/tmp/hdl-owned/home")

    def test_default_and_injected_session_names_cannot_route_socket(self):
        for session in ("default", "existing", "latency-../../live", "../default"):
            with self.assertRaises(ValueError):
                bench.owned_socket_path(Path("/var/tmp/hdl-owned"), session)
        path = bench.owned_socket_path(Path("/var/tmp/hdl-owned"), "latency-0123456789ab")
        self.assertEqual(path.name, "herdr.sock")
        self.assertIn("latency-0123456789ab", str(path))

    def test_socket_path_length_is_checked_before_launch(self):
        with self.assertRaises(ValueError):
            bench.owned_socket_path(Path("/var/tmp/" + "x" * 100), "latency-0123456789ab")

    def test_changed_cleanup_owner_retains_state(self):
        with tempfile.TemporaryDirectory() as directory:
            args = argparse.Namespace(output=Path(directory) / "evidence", binary=Path(__file__).resolve())
            case = bench.Case(args)
            try:
                (case.root / "owner").write_text("changed-owner")
                with self.assertRaisesRegex(RuntimeError, "ownership changed"):
                    case.cleanup()
                self.assertTrue(case.root.exists())
            finally:
                shutil.rmtree(case.root)

    def test_load_requires_explicit_remote_switch_before_any_case_is_created(self):
        args = argparse.Namespace(remote_isolated=False)
        with patch.object(bench, "Case") as case:
            with self.assertRaisesRegex(RuntimeError, "remote-isolated"):
                bench.run_case(args)
        case.assert_not_called()


@unittest.skipUnless(sys.platform == "linux", "remote harness targets Linux")
class WorkloadTest(unittest.TestCase):
    def test_subscribers_vary_independently_of_pane_cardinality(self):
        groups = bench.subscription_groups(["w1:p1"], 121, 3)
        self.assertEqual(sum(map(len, groups)), 121)
        self.assertEqual(len(groups), 3)
        self.assertTrue(all(entry["pane_id"] == "w1:p1" for group in groups for entry in group))
        groups = bench.subscription_groups([f"w1:p{i}" for i in range(174)], 15, 1)
        self.assertEqual(len(groups[0]), 15)
        self.assertEqual(bench.subscription_groups(["w1:p1"], 0, 1), [])

    def test_matrix_contains_all_required_independent_axes(self):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            bench.matrix(None)
        cases = [json.loads(line) for line in output.getvalue().splitlines()]
        self.assertEqual(len(cases), 162)
        self.assertEqual({item["panes"] for item in cases}, {1, 15, 174})
        self.assertEqual({item["mode"] for item in cases}, {"direct", "tui", "mixed"})
        self.assertTrue(any(item["panes"] == 1 and item["subscriptions"] == 121 for item in cases))
        self.assertTrue(any(item["panes"] == 174 and item["subscriptions"] == 0 for item in cases))
        self.assertTrue(any((item["cols"], item["rows"]) == (483, 128) for item in cases))

    def test_nonfinite_rates_are_rejected(self):
        for rate in ("nan", "inf", "0", "-1"):
            with self.assertRaises(argparse.ArgumentTypeError):
                bench.positive(rate)


@unittest.skipUnless(sys.platform == "linux", "remote harness targets Linux")
class JsonConnectionTest(unittest.TestCase):
    def test_each_ordinary_request_reconnects_and_revalidates_peer(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "one-request.sock"
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
                listener.bind(str(path))
                listener.listen(1)
                listener.settimeout(.1)  # Let failing implementations cancel the fixture.
                received = []
                stopped = threading.Event()
                def respond():
                    accepted = 0
                    while accepted < 3 and not stopped.is_set():
                        try:
                            connection, _ = listener.accept()
                        except socket.timeout:
                            continue
                        accepted += 1
                        with connection, connection.makefile("rb") as stream:
                            line = stream.readline()
                            if line:
                                request = json.loads(line)
                                received.append(request)
                                connection.sendall(json.dumps({"id": request["id"],
                                                               "result": {"type": "ok"}}).encode() + b"\n")
                thread = threading.Thread(target=respond)
                thread.start()
                peer = bench.JsonPeer(path, os.getpid())
                try:
                    self.assertEqual(peer.request("workspace.list"), {"type": "ok"})
                    self.assertEqual(peer.request("workspace.list"), {"type": "ok"})
                    self.assertEqual([r["id"] for r in received], ["bench-1", "bench-2"])
                    # A replacement server must be rejected on the next call.
                    peer.expected_pid = os.getpid() + 1
                    with self.assertRaisesRegex(RuntimeError, "not the owned server"):
                        peer.request("workspace.list")
                    self.assertEqual(len(received), 2)
                finally:
                    peer.close()
                    stopped.set()
                    thread.join()

    def test_subscription_keeps_one_connection_for_multiple_events(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "stream.sock"
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
                listener.bind(str(path))
                listener.listen(1)
                def respond():
                    connection, _ = listener.accept()
                    with connection, connection.makefile("rb") as stream:
                        request = json.loads(stream.readline())
                        for value in [{"id": request["id"], "result": {"type": "subscription_started"}},
                                      {"event": 1}, {"event": 2}]:
                            connection.sendall(json.dumps(value).encode() + b"\n")
                thread = threading.Thread(target=respond)
                thread.start()
                peer = bench.JsonPeer(path, os.getpid())
                try:
                    request_id = peer.send("events.subscribe", {"subscriptions": []})
                    self.assertEqual(peer.receive()["id"], request_id)
                    self.assertEqual(peer.receive(), {"event": 1})
                    self.assertEqual(peer.receive(), {"event": 2})
                finally:
                    peer.close()
                    thread.join()

    def exercise(self, response_id, expected_pid=None):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "fake-api.sock"
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
                listener.bind(str(path))
                listener.listen(1)
                received = []
                def respond():
                    connection, _ = listener.accept()
                    with connection, connection.makefile("rb") as stream:
                        line = stream.readline()
                        if line:
                            request = json.loads(line)
                            received.append(request)
                            connection.sendall(json.dumps({"id": response_id,
                                                           "result": {"type": "ok"}}).encode() + b"\n")
                thread = threading.Thread(target=respond)
                thread.start()
                peer = None
                try:
                    peer = bench.JsonPeer(path, os.getpid() if expected_pid is None else expected_pid)
                    return peer.request("pane.send_input", pane_id="owned:p1", text="I00000001",
                                        keys=["Enter"]), received
                finally:
                    if peer:
                        peer.close()
                    thread.join()

    def test_a_write_ack_is_only_a_response_not_an_echo_measurement(self):
        result, requests = self.exercise("bench-1")
        self.assertEqual(result["type"], "ok")
        self.assertEqual(requests[0]["params"]["text"], "I00000001")
        self.assertNotIn("ECHO", requests[0]["params"]["text"])

    def test_response_identity_mismatch_is_reported(self):
        with self.assertRaisesRegex(RuntimeError, "identity mismatch"):
            self.exercise("wrong-request")

    def test_an_unowned_socket_is_rejected_before_sending_input(self):
        with self.assertRaisesRegex(RuntimeError, "not the owned server"):
            self.exercise("bench-1", os.getpid() + 1)


@unittest.skipUnless(sys.platform == "linux", "remote harness targets Linux")
class ProducerProtocolTest(unittest.TestCase):
    def test_real_pty_echo_disables_kernel_echo_and_preserves_packet_order(self):
        # An inert Python child, never Herdr, a provider, or a user's pane.
        import pty
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "producer.json").write_text(json.dumps({
                "rows": 4, "output_hz": 30, "output": "idle", "audit": True}))
            env = bench.isolated_environment(root, "latency-0123456789ab")
            env["HERDR_PANE_ID"] = "owned:p1"
            master, slave = pty.openpty()
            process = subprocess.Popen([sys.executable, str(Path(bench.__file__).resolve()), "producer"],
                                       env=env, stdin=slave, stdout=slave,
                                       stderr=subprocess.PIPE, start_new_session=True)
            os.close(slave)
            captured = b""
            def receive_until(marker):
                nonlocal captured
                with selectors.DefaultSelector() as selector:
                    selector.register(master, selectors.EVENT_READ)
                    while marker not in captured:
                        selector.select()
                        data = os.read(master, 4096)
                        self.assertTrue(data, "stand-in exited before emitting its marker")
                        captured += data
            try:
                receive_until(b"BENCHREADY")
                os.write(master, b"I00000001\nI00000002\nI00000003\nH00000002\n")
                receive_until(b"SCROLLBOTTOM00000002")
                self.assertNotIn(b"I00000001", captured)
                self.assertLess(captured.index(b"ECHO00000001"), captured.index(b"ECHO00000002"))
                self.assertLess(captured.index(b"ECHO00000002"), captured.index(b"ECHO00000003"))
                ledger_path = next(root.glob("audit-*.jsonl"))
                ledger = [json.loads(line) for line in ledger_path.read_text().splitlines()]
                self.assertEqual([item["sequence"] for item in ledger], [1, 2, 3])
            finally:
                process.terminate()
                process.wait()
                process.stderr.close()
                os.close(master)


@unittest.skipUnless(sys.platform == "linux" and importlib.util.find_spec("pyte"),
                     "cell-observer contracts require remote pyte dependency")
class CellObserverTest(unittest.TestCase):
    def test_private_device_status_query_does_not_change_cells_or_break_parser(self):
        cells = bench.TerminalCells(20, 3)
        cells.feed(b"\x1b[1;1HREADY")
        before = tuple(cells.screen.display)
        cells.feed(b"\x1b[?99")
        cells.feed(b"6n")
        self.assertEqual(tuple(cells.screen.display), before)
        commits = cells.feed(b"\x1b[2;1HECHO00000001")
        self.assertTrue(any("ECHO00000001" in text for text, _ in commits))

    def test_synchronized_update_never_publishes_partial_frame_cells(self):
        cells = bench.TerminalCells(20, 3)
        self.assertEqual(cells.feed(b"\x1b[?2026h\x1b[1;1HECHO00000001"), [])
        self.assertEqual(cells.feed(b"\x1b[?2026"), [])
        commits = cells.feed(b"l")
        self.assertTrue(any("ECHO00000001" in text for text, _ in commits))
        self.assertEqual(cells.sync_commits, 1)
        # A subsequent unfinished frame must not hide the completed frame in
        # the same transport read or claim the next echo is visible yet.
        commits = cells.feed(b"\x1b[?2026h\x1b[1;1HECHO00000002\x1b[?2026l"
                             b"\x1b[?2026h\x1b[1;1HECHO00000003")
        self.assertTrue(any("ECHO00000002" in text for text, _ in commits))
        self.assertFalse(any("ECHO00000003" in text for text, _ in commits))

    def test_fragmented_updates_are_measured_from_cells_and_repaints_do_not_duplicate_echo(self):
        samples = bench.Samples()
        samples.send(1, "tui", 0)
        with patch.object(bench.pty, "fork", return_value=(1234, 999)), patch.object(
                bench.os, "set_blocking"):
            tui = bench.Tui(Path("/unused"), {}, "latency-0123456789ab", 20, 3,
                            samples, threading.Event(), threading.Event())
        with patch.object(bench.os, "read", side_effect=[
                b"\x1b[1;1HECHO0000", b"0001", b"\x1b[1;1HECHO00000001", b"\x1b[2J"]):
            tui.read()
            self.assertEqual(samples.report()["tui_to_tui_ms"]["count"], 0)
            tui.read()
            self.assertEqual(samples.report()["tui_to_tui_ms"]["count"], 1)
            first = samples.observed["tui"][1]
            tui.read()
            self.assertEqual(samples.observed["tui"][1], first)
            tui.read()
            self.assertNotIn("ECHO", tui.visible())


if __name__ == "__main__":
    unittest.main()
