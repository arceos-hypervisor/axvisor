#!/usr/bin/env python3
# Copyright 2025 The Axvisor Team
#
# Wrapper for CI: runs `cargo xtask qemu` for NimbOS and automatically sends
# "usertests\n" to the guest when "Rust user shell" appears, so the test can
# complete without interactive input. Ostool only matches success_regex on
# stdout and does not send stdin; this script bridges that gap.

import sys
import subprocess
import threading
import os

SEND_AFTER = b"Rust user shell"
SEND_LINE = b"usertests\n"


def main():
    try:
        sep = sys.argv.index("--")
    except ValueError:
        print("Usage: ci_run_qemu_nimbos.py -- <command> [args...]", file=sys.stderr)
        sys.exit(2)
    cmd = sys.argv[sep + 1 :]
    if not cmd:
        print("No command after --", file=sys.stderr)
        sys.exit(2)

    proc = subprocess.Popen(
        cmd,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    sent = threading.Lock()
    sent_done = [False]  # list to allow closure to mutate

    def read_stdout():
        buffer = b""
        while True:
            chunk = proc.stdout.read(256)
            if not chunk:
                break
            sys.stdout.buffer.write(chunk)
            sys.stdout.buffer.flush()
            buffer = (buffer + chunk)[-max(512, len(SEND_AFTER) * 2) :]
            if not sent_done[0] and SEND_AFTER in buffer:
                with sent:
                    if not sent_done[0]:
                        try:
                            proc.stdin.write(SEND_LINE)
                            proc.stdin.flush()
                        except (BrokenPipeError, OSError):
                            pass
                        sent_done[0] = True

    t = threading.Thread(target=read_stdout)
    t.daemon = True
    t.start()
    proc.wait()
    t.join(timeout=1)
    sys.exit(proc.returncode if proc.returncode is not None else 1)


if __name__ == "__main__":
    main()
