"""Remote Linux contracts; no Herdr server or provider process is required."""
import os
from pathlib import Path
import select
import subprocess
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch

from scripts.fast_api import rehearse


class ReceiptTests(unittest.TestCase):
    def test_flush_to_already_open_receipt_wakes_waiter_before_writer_closes(self):
        with tempfile.TemporaryDirectory(prefix="hfr-") as folder:
            path = Path(folder) / "received"
            read_started = threading.Event()
            flushed = threading.Event()
            writer_errors = []
            real_read = os.read
            with path.open("a", buffering=1) as receipt:
                def append_after_wait_starts():
                    read_started.wait()
                    try:
                        receipt.write("after-0-0\n")
                        receipt.flush()
                    except BaseException as error:
                        writer_errors.append(error)
                    finally:
                        flushed.set()

                def synchronized_read(fd, size):
                    read_started.set()
                    flushed.wait()
                    self.assertEqual(writer_errors, [])
                    self.assertFalse(receipt.closed)
                    # Flush has completed, so its kernel event must be queued.
                    # A zero-time readiness check fails the old mask without
                    # hanging the contract or depending on a timing threshold.
                    self.assertEqual(select.select([fd], [], [], 0)[0], [fd],
                                     "flush to an open receipt produced no watched event")
                    return real_read(fd, size)

                writer = threading.Thread(target=append_after_wait_starts)
                writer.start()
                try:
                    with patch.object(rehearse.os, "read", synchronized_read):
                        result = rehearse.wait_marker(path, lambda text: "after-0-0\n" in text)
                    self.assertEqual(result, "after-0-0\n")
                    self.assertFalse(receipt.closed)
                finally:
                    read_started.set()
                    writer.join()


class OutputTests(unittest.TestCase):
    def test_output_verification_reads_the_requested_pane_after_output_barrier(self):
        tokens = [f"after-12-{n}" for n in range(3)]
        read = {"pane_id": "w1:p12", "text": "\n".join("got:" + t for t in tokens)}
        with patch.object(rehearse, "request", side_effect=[{"result": {}},
                                                          {"result": {"read": read}}]) as api:
            self.assertEqual(rehearse.verify_pane_output(Path("unused"), "w1:p12", tokens), read)
        self.assertEqual([call.args[1] for call in api.call_args_list],
                         ["pane.wait_for_output", "pane.read"])
        self.assertEqual(api.call_args_list[0].args[2]["match"]["value"], "got:after-12-2")
        self.assertNotIn("timeout_ms", api.call_args_list[0].args[2])
        self.assertEqual(api.call_args_list[1].args[2]["pane_id"], "w1:p12")

    def test_output_read_from_a_different_pane_is_rejected_even_with_matching_markers(self):
        tokens = [f"after-12-{n}" for n in range(3)]
        read = {"pane_id": "w1:p13", "text": "\n".join("got:" + t for t in tokens)}
        with patch.object(rehearse, "request", side_effect=[{"result": {}},
                                                          {"result": {"read": read}}]):
            with self.assertRaisesRegex(AssertionError, "different pane"):
                rehearse.verify_pane_output(Path("unused"), "w1:p12", tokens)

    def test_unique_exact_output_markers_reject_echo_wrong_pane_duplicate_and_reordering(self):
        tokens = [f"after-12-{n}" for n in range(3)]
        markers = ["got:" + token for token in tokens]
        for text in ("\n".join(tokens), "\n".join(markers).replace("after-12", "after-13"),
                     "\n".join(markers + markers[:1]), "\n".join(reversed(markers)),
                     "\n".join(markers).replace("got:after-12-0", "got:after-12-01")):
            with self.subTest(text=text), self.assertRaises(AssertionError):
                rehearse.assert_output_markers(text, tokens, "w1:p12")

    def test_missing_output_negative_control_keeps_complete_ordered_receipt(self):
        with tempfile.TemporaryDirectory(prefix="hfo-") as folder:
            tokens = [f"after-0-{n}" for n in range(3)]
            standin = Path(rehearse.__file__).with_name("standin.py")
            child = subprocess.run([sys.executable, str(standin), folder,
                                    "--suppress-output", tokens[0]],
                                   input="\n".join(tokens) + "\n", text=True,
                                   capture_output=True, check=True)
            self.assertEqual((Path(folder) / "received").read_text().splitlines(), tokens)
            self.assertIn("got:" + tokens[-1], child.stdout.splitlines())
            self.assertNotIn("got:" + tokens[0], child.stdout.splitlines())
            # Exercise the actual verifier, with the available final marker but
            # suppressed earlier output. The original receipt-only check passes.
            read = {"pane_id": "w1:p0", "text": child.stdout}
            with patch.object(rehearse, "request", side_effect=[{"result": {}},
                                                              {"result": {"read": read}}]):
                with self.assertRaisesRegex(AssertionError, "missing.*got:after-0-0"):
                    rehearse.verify_pane_output(Path("unused"), "w1:p0", tokens)


if __name__ == "__main__":
    unittest.main()
