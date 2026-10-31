"""Readiness for the owned nested Sway X11 output used by Linux probes."""
import re
import subprocess
import time


def wait_for_sway_host(env, output, process, timeout=10):
    """Wait for the actual host window; sockets alone do not prove it exists."""
    deadline = time.monotonic() + timeout
    while True:
        try:
            result = subprocess.run(['xwininfo', '-root', '-tree'], env=env,
                                    capture_output=True, text=True, timeout=3)
        except (OSError, subprocess.TimeoutExpired) as error:
            (output / 'sway-host-query.log').write_text(f'{type(error).__name__}: {error}\n')
            raise
        (output / 'sway-host-tree.log').write_text(result.stdout)
        (output / 'sway-host-query.log').write_text(
            f'exit_code={result.returncode}\n' + result.stderr)
        status = process.poll()
        if status is not None:
            raise RuntimeError(f'Sway exited with {status}; inspect sway.log and sway-host-tree.log')
        if result.returncode:
            raise RuntimeError('xwininfo failed; inspect sway-host-query.log')
        match = re.search(r'(0x[0-9a-fA-F]+) "wlroots - X11-1"', result.stdout)
        if match:
            return match.group(1)
        if time.monotonic() >= deadline:
            raise RuntimeError('Sway host window did not appear; inspect sway.log and sway-host-tree.log')
        time.sleep(.05)


def wait_for_openbox(env, output, process, timeout=10):
    """Require EWMH/UTF8 title atoms before wlroots interns existing atoms."""
    deadline = time.monotonic() + timeout
    while True:
        try:
            root = subprocess.run(['xprop', '-root', '_NET_SUPPORTING_WM_CHECK'],
                                  env=env, capture_output=True, text=True, timeout=3)
            owner = re.search(r'window id # (0x[0-9a-fA-F]+)', root.stdout)
            detail = None
            if root.returncode == 0 and owner:
                detail = subprocess.run(['xprop', '-id', owner.group(1), '_NET_WM_NAME'],
                                        env=env, capture_output=True, text=True, timeout=3)
        except (OSError, subprocess.TimeoutExpired) as error:
            (output / 'openbox-readiness.log').write_text(f'{type(error).__name__}: {error}\n')
            raise
        (output / 'openbox-readiness.log').write_text(
            f'root_exit={root.returncode}\n{root.stdout}{root.stderr}'
            + (f'owner_exit={detail.returncode}\n{detail.stdout}{detail.stderr}' if detail else ''))
        status = process.poll()
        if status is not None:
            raise RuntimeError(f'Openbox exited with {status}; inspect openbox.log')
        if (detail and detail.returncode == 0
                and re.search(r'_NET_WM_NAME\(UTF8_STRING\) = "Openbox"', detail.stdout)):
            return
        if time.monotonic() >= deadline:
            raise RuntimeError('Openbox EWMH title properties unavailable; inspect openbox-readiness.log')
        time.sleep(.05)


# An owned Sway repeats no key. A key's release can reach it late on a loaded
# runner, and then the client, or an input method's keyboard grab (Fcitx
# repeats keys itself), repeated the key: "nihao" composed 您好.
KEYBOARD = "input type:keyboard repeat_rate 0\n"


def sway_config(path, text):
    """Write an owned Sway's config: `text`, with keys that don't repeat."""
    path.write_text(text + KEYBOARD)
