#!/usr/bin/env python3
"""`xvfb_retry/Xvfb` goes with whatever started it, however that ended: a smoke
that crashed or was killed left its display taken for good, until no lane on the
machine could get one. Against a stand-in for the real Xvfb, so no X is needed."""
import os
import pathlib
import signal
import stat
import subprocess
import sys
import tempfile
import textwrap
import time
import unittest

WRAPPER = pathlib.Path(__file__).resolve().parent / "xvfb_retry" / "Xvfb"

# Reports display 7 on the fd it is given, as Xvfb does, and runs until stopped.
# With a file named FAKE_FAIL_ONCE, the first start exits without reporting.
FAKE = """#!/bin/sh
fd=
while [ $# -gt 0 ]; do
    [ "$1" = -displayfd ] && fd=$2
    shift
done
echo $$ >> "$FAKE_PIDS"
if [ -e "$FAKE_FAIL_ONCE" ]; then
    rm "$FAKE_FAIL_ONCE"
    exit 1
fi
[ -n "$fd" ] && eval "echo 7 >&$fd"
exec sleep 300
"""

# What a smoke does: start it, read the display, say so, then end as it's told.
PARENT = textwrap.dedent("""
    import os, signal, subprocess, sys
    how, args = sys.argv[1], sys.argv[2:]
    wrapper = subprocess.Popen(args, stdout=subprocess.PIPE, text=True)
    if "-displayfd" in args:
        wrapper.stdout.readline()
    print(wrapper.pid, flush=True)
    sys.stdin.readline()
    if how == "killed":
        os.kill(os.getpid(), signal.SIGKILL)
    elif how == "raises":
        raise SystemExit(1)
    elif how == "terminates":
        wrapper.terminate()
        wrapper.wait()
""")


def running(pid):
    try:
        state = pathlib.Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[0]
    except OSError:
        return False
    return state != "Z"


@unittest.skipUnless(sys.platform == "linux", "PR_SET_PDEATHSIG is Linux's")
class Wrapper(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory(prefix="zgx-xvfb-retry-")
        self.addCleanup(self.dir.cleanup)
        root = pathlib.Path(self.dir.name)
        fake = root / "Xvfb"
        fake.write_text(FAKE)
        fake.chmod(fake.stat().st_mode | stat.S_IEXEC)
        self.pids_file = root / "pids"
        self.fail_once = root / "fail-once"
        self.env = dict(os.environ, PATH=f"{root}{os.pathsep}{os.environ['PATH']}",
                        FAKE_PIDS=str(self.pids_file), FAKE_FAIL_ONCE=str(self.fail_once))
        self.started = []
        self.addCleanup(self.stop_all)

    def stop_all(self):
        for pid in self.started + self.servers():
            if running(pid):
                try:
                    os.kill(pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass  # It went between the two lines: what this is here to make sure of.

    def servers(self):
        return [int(line) for line in self.pids_file.read_text().split()] if self.pids_file.exists() else []

    def parent(self, how, *args):
        """A parent that has started the wrapper: (process, the wrapper's pid)."""
        process = subprocess.Popen([sys.executable, "-c", PARENT, how, str(WRAPPER), *args], env=self.env,
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        self.addCleanup(process.stdout.close)
        self.addCleanup(process.stdin.close)
        self.addCleanup(process.wait)
        wrapper = int(process.stdout.readline())
        self.started.append(wrapper)
        return process, wrapper

    def assert_gone(self, *pids):
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline and any(running(pid) for pid in pids):
            time.sleep(.02)
        left = [pid for pid in pids if running(pid)]
        self.assertEqual(left, [], "still running")

    def ends(self, how):
        process, wrapper = self.parent(how, "-displayfd", "1")
        servers = self.servers()
        self.assertEqual(len(servers), 1)
        self.assertTrue(running(wrapper) and running(servers[0]), "up before its parent ends")
        process.stdin.write("\n")
        process.stdin.flush()
        process.wait()
        self.assert_gone(wrapper, servers[0])

    def test_goes_when_its_parent_is_killed(self):
        self.ends("killed")

    def test_goes_when_its_parent_exits_without_stopping_it(self):
        self.ends("exits")

    def test_goes_when_its_parent_fails(self):
        self.ends("raises")

    def test_stopped_by_its_parent_as_ever(self):
        self.ends("terminates")

    def test_with_no_displayfd_it_goes_with_its_parent_too(self):
        # With nothing to retry the wrapper becomes the real server.
        process, wrapper = self.parent("killed")
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline and not self.servers():
            time.sleep(.02)
        self.assertEqual(self.servers(), [wrapper], "the wrapper execs the server")
        process.stdin.write("\n")
        process.stdin.flush()
        process.wait()
        self.assert_gone(wrapper)

    def test_a_start_with_no_display_is_tried_again(self):
        self.fail_once.touch()
        process, wrapper = self.parent("exits", "-displayfd", "1")
        servers = self.servers()
        self.assertEqual(len(servers), 2, "the failed start and the one that reported")
        self.assertFalse(self.fail_once.exists())
        self.assertTrue(running(servers[1]))
        process.stdin.write("\n")
        process.stdin.flush()
        process.wait()
        self.assert_gone(wrapper, servers[1])

    def test_a_server_that_stops_ends_the_wrapper(self):
        process, wrapper = self.parent("waits", "-displayfd", "1")
        server = self.servers()[0]
        os.kill(server, signal.SIGTERM)
        self.assert_gone(wrapper, server)
        process.stdin.write("\n")
        process.stdin.flush()


if __name__ == "__main__":
    unittest.main()
