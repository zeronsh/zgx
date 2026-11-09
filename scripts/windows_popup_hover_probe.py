"""Exercise popup_hover on an unlocked Windows console desktop.

Usage: python windows_popup_hover_probe.py PATH/TO/popup_hover.exe

No clicks or system settings changes are made. A system dialog, unavailable
interactive desktop, or covered target aborts the run. Build the example first.
All child processes inherit a kill-on-close Job Object. The pointer is restored
for every case, including failures. JSON lines on stdout are the complete log.
"""

import argparse
import ctypes
from ctypes import wintypes
import json
import os
from pathlib import Path
import subprocess
import sys
import threading
import time


class SafetyFailure(RuntimeError):
    """Continuing would violate the probe's desktop safety requirements."""


class ExpectationFailure(RuntimeError):
    """The observed GUI behavior differs from the expected behavior."""


def require(condition, message):
    if not condition:
        raise ExpectationFailure(message)


def log(event, **fields):
    print(json.dumps({"time": time.time(), "event": event, **fields}), flush=True)


if sys.platform != "win32":
    raise SystemExit("This probe requires an unlocked Windows console desktop.")


user32 = ctypes.WinDLL("user32", use_last_error=True)
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
POINTER = ctypes.c_void_p
LPARAM = ctypes.c_ssize_t
WPARAM = ctypes.c_size_t
WINDOW_PROC = ctypes.WINFUNCTYPE(
    LPARAM, wintypes.HWND, wintypes.UINT, WPARAM, LPARAM
)
ENUM_PROC = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, LPARAM)


def bind(dll, name, result, arguments):
    function = getattr(dll, name)
    function.restype = result
    function.argtypes = arguments
    return function


# HWNDs and pointer-sized results must not use ctypes' default 32-bit int.
bind(user32, "EnumWindows", wintypes.BOOL, [ENUM_PROC, LPARAM])
bind(user32, "GetWindowThreadProcessId", wintypes.DWORD,
     [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)])
bind(user32, "GetWindow", wintypes.HWND, [wintypes.HWND, wintypes.UINT])
bind(user32, "GetAncestor", wintypes.HWND, [wintypes.HWND, wintypes.UINT])
bind(user32, "GetWindowLongPtrW", LPARAM, [wintypes.HWND, ctypes.c_int])
bind(user32, "GetClassNameW", ctypes.c_int,
     [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int])
bind(user32, "GetWindowTextW", ctypes.c_int,
     [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int])
bind(user32, "GetWindowRect", wintypes.BOOL,
     [wintypes.HWND, ctypes.POINTER(wintypes.RECT)])
bind(user32, "IsWindowVisible", wintypes.BOOL, [wintypes.HWND])
bind(user32, "GetForegroundWindow", wintypes.HWND, [])
bind(user32, "GetFocus", wintypes.HWND, [])
bind(user32, "GetGUIThreadInfo", wintypes.BOOL, [wintypes.DWORD, POINTER])
bind(user32, "SetForegroundWindow", wintypes.BOOL, [wintypes.HWND])
bind(user32, "AttachThreadInput", wintypes.BOOL,
     [wintypes.DWORD, wintypes.DWORD, wintypes.BOOL])
bind(user32, "SetWindowPos", wintypes.BOOL,
     [wintypes.HWND, wintypes.HWND, ctypes.c_int, ctypes.c_int,
      ctypes.c_int, ctypes.c_int, wintypes.UINT])
bind(user32, "GetCursorPos", wintypes.BOOL, [ctypes.POINTER(wintypes.POINT)])
bind(user32, "SetCursorPos", wintypes.BOOL, [ctypes.c_int, ctypes.c_int])
bind(user32, "WindowFromPoint", wintypes.HWND, [wintypes.POINT])
bind(user32, "ClientToScreen", wintypes.BOOL,
     [wintypes.HWND, ctypes.POINTER(wintypes.POINT)])
bind(user32, "GetDpiForWindow", wintypes.UINT, [wintypes.HWND])
bind(user32, "SetProcessDpiAwarenessContext", wintypes.BOOL, [POINTER])
bind(user32, "OpenInputDesktop", wintypes.HANDLE,
     [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD])
bind(user32, "GetUserObjectInformationW", wintypes.BOOL,
     [wintypes.HANDLE, ctypes.c_int, POINTER, wintypes.DWORD,
      ctypes.POINTER(wintypes.DWORD)])
bind(user32, "CloseDesktop", wintypes.BOOL, [wintypes.HANDLE])
bind(kernel32, "GetCurrentProcess", wintypes.HANDLE, [])
bind(kernel32, "GetCurrentThreadId", wintypes.DWORD, [])
bind(kernel32, "WTSGetActiveConsoleSessionId", wintypes.DWORD, [])
bind(kernel32, "ProcessIdToSessionId", wintypes.BOOL,
     [wintypes.DWORD, ctypes.POINTER(wintypes.DWORD)])
bind(kernel32, "CreateJobObjectW", wintypes.HANDLE, [POINTER, wintypes.LPCWSTR])
bind(kernel32, "SetInformationJobObject", wintypes.BOOL,
     [wintypes.HANDLE, ctypes.c_int, POINTER, wintypes.DWORD])
bind(kernel32, "AssignProcessToJobObject", wintypes.BOOL,
     [wintypes.HANDLE, wintypes.HANDLE])


class BasicJobLimits(ctypes.Structure):
    _fields_ = [
        ("process_time", ctypes.c_int64),
        ("job_time", ctypes.c_int64),
        ("flags", wintypes.DWORD),
        ("minimum_working_set", ctypes.c_size_t),
        ("maximum_working_set", ctypes.c_size_t),
        ("active_process_limit", wintypes.DWORD),
        ("affinity", ctypes.c_size_t),
        ("priority_class", wintypes.DWORD),
        ("scheduling_class", wintypes.DWORD),
    ]


class IoCounters(ctypes.Structure):
    _fields_ = [
        ("read_operations", ctypes.c_uint64),
        ("write_operations", ctypes.c_uint64),
        ("other_operations", ctypes.c_uint64),
        ("read_bytes", ctypes.c_uint64),
        ("write_bytes", ctypes.c_uint64),
        ("other_bytes", ctypes.c_uint64),
    ]


class ExtendedJobLimits(ctypes.Structure):
    _fields_ = [
        ("basic", BasicJobLimits),
        ("io", IoCounters),
        ("process_memory", ctypes.c_size_t),
        ("job_memory", ctypes.c_size_t),
        ("peak_process_memory", ctypes.c_size_t),
        ("peak_job_memory", ctypes.c_size_t),
    ]


class GuiThreadInfo(ctypes.Structure):
    _fields_ = [
        ("size", wintypes.DWORD),
        ("flags", wintypes.DWORD),
        ("active", wintypes.HWND),
        ("focus", wintypes.HWND),
        ("capture", wintypes.HWND),
        ("menu_owner", wintypes.HWND),
        ("move_size", wintypes.HWND),
        ("caret", wintypes.HWND),
        ("caret_rect", wintypes.RECT),
    ]


def create_job():
    job = kernel32.CreateJobObjectW(None, None)
    if not job:
        raise ctypes.WinError(ctypes.get_last_error())
    limits = ExtendedJobLimits()
    limits.basic.flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
    if not kernel32.SetInformationJobObject(
        job, 9, ctypes.byref(limits), ctypes.sizeof(limits)
    ):
        raise ctypes.WinError(ctypes.get_last_error())
    if not kernel32.AssignProcessToJobObject(job, kernel32.GetCurrentProcess()):
        raise SafetyFailure("Cannot assign probe to its kill-on-close Job Object")
    # Retain this handle until process exit. Closing it earlier kills this
    # process too. Child processes inherit job membership, not this handle.
    return job


def window_identity(hwnd):
    process_id = wintypes.DWORD()
    thread_id = user32.GetWindowThreadProcessId(hwnd, ctypes.byref(process_id))
    return {"hwnd": hwnd or 0, "pid": process_id.value, "tid": thread_id}


def enumerate_windows():
    result = []

    @ENUM_PROC
    def visit(hwnd, unused):
        rect = wintypes.RECT()
        class_name = ctypes.create_unicode_buffer(256)
        title = ctypes.create_unicode_buffer(512)
        user32.GetWindowRect(hwnd, ctypes.byref(rect))
        user32.GetClassNameW(hwnd, class_name, len(class_name))
        user32.GetWindowTextW(hwnd, title, len(title))
        extended_style = user32.GetWindowLongPtrW(hwnd, -20)
        result.append({
            "z": len(result),
            **window_identity(hwnd),
            "class": class_name.value,
            "title": title.value,
            "owner": user32.GetWindow(hwnd, 4) or 0,  # GW_OWNER
            "exstyle": hex(extended_style),
            "topmost": bool(extended_style & 8),
            "visible": bool(user32.IsWindowVisible(hwnd)),
            "rect": [rect.left, rect.top, rect.right, rect.bottom],
        })
        return True

    if not user32.EnumWindows(visit, 0):
        raise ctypes.WinError(ctypes.get_last_error())
    return result


def check_desktop():
    session_id = wintypes.DWORD()
    if not kernel32.ProcessIdToSessionId(os.getpid(), ctypes.byref(session_id)):
        raise SafetyFailure("Cannot determine the probe's Windows session")
    console_id = kernel32.WTSGetActiveConsoleSessionId()
    if session_id.value != console_id:
        raise SafetyFailure("No interactive console session for this process")
    desktop = user32.OpenInputDesktop(0, False, 1)  # DESKTOP_READOBJECTS
    if not desktop:
        raise SafetyFailure("Cannot open input desktop: it may be locked")
    try:
        name = ctypes.create_unicode_buffer(256)
        needed = wintypes.DWORD()
        success = user32.GetUserObjectInformationW(
            desktop, 2, name, ctypes.sizeof(name), ctypes.byref(needed)
        )
        if not success or name.value != "Default":
            raise SafetyFailure("Input desktop is not the unlocked Default desktop")
    finally:
        user32.CloseDesktop(desktop)
    if not user32.GetForegroundWindow():
        raise SafetyFailure("No foreground window on the interactive desktop")


def check_safety():
    check_desktop()
    dialogs = [
        window for window in enumerate_windows()
        if window["visible"] and window["class"] == "#32770"
    ]
    if dialogs:
        log("system_dialog", windows=dialogs)
        raise SafetyFailure("System dialog detected. It was not touched or dismissed.")


def pause(seconds):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        check_safety()
        time.sleep(min(0.05, max(0, deadline - time.monotonic())))


def cursor_position():
    point = wintypes.POINT()
    if not user32.GetCursorPos(ctypes.byref(point)):
        raise SafetyFailure("Cannot read cursor position")
    return point.x, point.y


def move_cursor(point):
    check_safety()
    if not user32.SetCursorPos(*point):
        raise SafetyFailure("SetCursorPos failed")


def place_window(hwnd, x, y, width, height):
    check_safety()
    # SWP_NOZORDER | SWP_NOACTIVATE: geometry alone changes neither order nor focus.
    if not user32.SetWindowPos(hwnd, None, x, y, width, height, 0x14):
        raise SafetyFailure("Cannot position probe window")


def activate(hwnd):
    check_safety()
    current_thread = kernel32.GetCurrentThreadId()
    foreground_thread = window_identity(user32.GetForegroundWindow())["tid"]
    attached = False
    if foreground_thread and foreground_thread != current_thread:
        attached = bool(user32.AttachThreadInput(
            current_thread, foreground_thread, True
        ))
    try:
        success = bool(user32.SetForegroundWindow(hwnd))
    finally:
        if attached:
            user32.AttachThreadInput(current_thread, foreground_thread, False)
    log("activation_requested", hwnd=hwnd, success=success)
    pause(0.3)
    if user32.GetForegroundWindow() != hwnd:
        raise SafetyFailure("Cannot establish requested foreground without clicks")


def thread_focus(thread_id):
    info = GuiThreadInfo()
    info.size = ctypes.sizeof(info)
    if not user32.GetGUIThreadInfo(thread_id, ctypes.byref(info)):
        raise SafetyFailure(f"GetGUIThreadInfo failed for thread {thread_id}")
    return {
        "active": window_identity(info.active),
        "focus": window_identity(info.focus),
    }


class Application:
    def __init__(self, arguments, second=False):
        environment = os.environ.copy()
        for key in ("POPUP_HOVER_SECOND", "POPUP_HOVER_ACT", "POPUP_HOVER_AT"):
            environment.pop(key, None)
        if second:
            environment["POPUP_HOVER_SECOND"] = "1"
        self.lines = []
        self.process = subprocess.Popen(
            arguments,
            env=environment,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
        self.reader = threading.Thread(target=self.read_output, daemon=True)
        self.reader.start()

    def read_output(self):
        for line in self.process.stdout:
            line = line.rstrip("\r\n")
            self.lines.append(line)
            log("app_output", pid=self.process.pid, line=line)

    def ready(self):
        deadline = time.monotonic() + 40
        while "READY" not in self.lines:
            self.check_running()
            if time.monotonic() > deadline:
                raise ExpectationFailure("App did not print READY within 40 seconds")
            pause(0.05)
        pause(0.2)
        return self

    def check_running(self):
        if self.process.poll() is not None:
            raise ExpectationFailure(
                f"App {self.process.pid} exited early with {self.process.returncode}"
            )

    def window(self, title):
        matches = [
            window["hwnd"] for window in enumerate_windows()
            if window["pid"] == self.process.pid and window["visible"]
            and window["title"] == title and not window["owner"]
        ]
        require(len(matches) == 1, f"Expected one window with title {title!r}")
        return matches[0]

    def target(self, hwnd, name):
        coordinates = {}
        for line in self.lines:
            fields = line.split()
            if len(fields) == 4 and fields[0] == "AT":
                coordinates[fields[1]] = (float(fields[2]), float(fields[3]))
        require(name in coordinates, f"Missing AT coordinates for {name}")
        scale = user32.GetDpiForWindow(hwnd) / 96
        x, y = coordinates[name]
        point = wintypes.POINT(round(x * scale), round(y * scale))
        if not user32.ClientToScreen(hwnd, ctypes.byref(point)):
            raise SafetyFailure("Cannot convert target coordinates to screen")
        return point.x, point.y

    def close(self):
        if self.process.poll() is None:
            self.process.terminate()
            self.process.wait(timeout=10)
        self.reader.join(timeout=2)
        self.process.stdout.close()
        log("app_closed", pid=self.process.pid, returncode=self.process.returncode)


def verify_target(hwnd, point):
    check_safety()
    hit = user32.WindowFromPoint(wintypes.POINT(*point))
    root = user32.GetAncestor(hit, 2)  # GA_ROOT
    log("target_hit", point=point, hit=window_identity(hit), root=root)
    if root != hwnd:
        raise SafetyFailure("Hover point does not belong to the intended main window")


def hover(hwnd, point):
    for offset in ((0, 0), (2, 0), (1, 1), (0, 0)):
        destination = (point[0] + offset[0], point[1] + offset[1])
        verify_target(hwnd, destination)
        move_cursor(destination)
        pause(0.08)
    pause(1.5)


def overlaps(first, second):
    return (
        max(first[0], second[0]) < min(first[2], second[2])
        and max(first[1], second[1]) < min(first[3], second[3])
    )


def snapshot(label, app, front_pid, main, front):
    check_safety()
    app.check_running()
    all_windows = enumerate_windows()
    relevant = [
        window for window in all_windows
        if window["pid"] in (app.process.pid, front_pid)
    ]
    foreground = user32.GetForegroundWindow()
    threads = {
        window_identity(hwnd)["tid"] for hwnd in (foreground, main, front)
    }
    state = {
        "foreground": window_identity(foreground),
        "calling_thread_GetFocus": window_identity(user32.GetFocus()),
        "thread_focus": {str(tid): thread_focus(tid) for tid in sorted(threads)},
        "windows": relevant,
    }
    log("snapshot", label=label, **state)
    return state


def find_window(state, hwnd):
    return next(window for window in state["windows"] if window["hwnd"] == hwnd)


def visible_popups(state, app):
    return [
        window for window in state["windows"]
        if window["pid"] == app.process.pid
        and window["class"] == "ZgxPopup" and window["visible"]
    ]


def compare_states(name, before, after, app, main, front, target):
    front_window = find_window(after, front)
    popups = visible_popups(after, app)
    details = []
    for popup in popups:
        details.append({
            **popup,
            "above_front": popup["z"] < front_window["z"],
            "overlaps_front": overlaps(popup["rect"], front_window["rect"]),
        })
    raised = (
        find_window(before, main)["z"] > find_window(before, front)["z"]
        and find_window(after, main)["z"] < front_window["z"]
    )
    result = {
        "case": name,
        "foreground_changed": before["foreground"] != after["foreground"],
        "focus_changed": before["thread_focus"] != after["thread_focus"],
        "main_raised": raised,
        "popup_shown": bool(popups),
        "popup_above_front": any(popup["above_front"] for popup in details),
        "hover_enter_heard": f"HOVER {target} enter" in app.lines,
        "hover_lines": [line for line in app.lines if line.startswith("HOVER ")],
        "popups": details,
    }
    log("result", **result)
    return result


def assert_stable(result):
    require(not result["foreground_changed"], "Hover changed foreground window")
    require(not result["focus_changed"], "Hover changed active/focus owner")
    require(not result["main_raised"], "Hover raised first main above front window")


def assert_popup_count(state, app, target, showing):
    expected = (2 if target == "row" else 1) if showing and target != "overlay" else 0
    popups = visible_popups(state, app)
    require(len(popups) == expected,
            f"Expected {expected} visible ZgxPopup windows, observed {len(popups)}")
    for popup in popups:
        require(not popup["topmost"], "Popup unexpectedly has WS_EX_TOPMOST")
        require(popup["owner"], "Visible popup has no owner")
        owner = find_window(state, popup["owner"])
        require(popup["z"] < owner["z"], "Visible popup is below its owner")
        log("popup_above_owner", popup=popup["hwnd"], popup_z=popup["z"],
            owner=owner["hwnd"], owner_z=owner["z"])
    if expected == 2:
        # This example places the submenu to the right of its root menu.
        menu, submenu = sorted(popups, key=lambda window: window["rect"][0])
        require(submenu["rect"][0] >= menu["rect"][2] - 1,
                "Cannot identify row menu and submenu from their rectangles")
        require(submenu["z"] < menu["z"], "Submenu is below its parent menu")
        log("submenu_above_menu", menu=menu["hwnd"], menu_z=menu["z"],
            submenu=submenu["hwnd"], submenu_z=submenu["z"])


def front_window():
    """Run an ordinary, independent Win32 window with no controls or dialogs."""
    bind(kernel32, "GetModuleHandleW", wintypes.HMODULE, [wintypes.LPCWSTR])
    bind(user32, "DefWindowProcW", LPARAM,
         [wintypes.HWND, wintypes.UINT, WPARAM, LPARAM])
    bind(user32, "CreateWindowExW", wintypes.HWND,
         [wintypes.DWORD, wintypes.LPCWSTR, wintypes.LPCWSTR, wintypes.DWORD,
          ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int,
          wintypes.HWND, wintypes.HMENU, wintypes.HINSTANCE, POINTER])
    bind(user32, "GetMessageW", wintypes.BOOL,
         [ctypes.POINTER(wintypes.MSG), wintypes.HWND, wintypes.UINT, wintypes.UINT])
    bind(user32, "TranslateMessage", wintypes.BOOL, [ctypes.POINTER(wintypes.MSG)])
    bind(user32, "DispatchMessageW", LPARAM, [ctypes.POINTER(wintypes.MSG)])

    @WINDOW_PROC
    def procedure(hwnd, message, wparam, lparam):
        return user32.DefWindowProcW(hwnd, message, wparam, lparam)

    class WindowClass(ctypes.Structure):
        _fields_ = [
            ("style", wintypes.UINT),
            ("procedure", WINDOW_PROC),
            ("class_extra", ctypes.c_int),
            ("window_extra", ctypes.c_int),
            ("instance", wintypes.HINSTANCE),
            ("icon", wintypes.HICON),
            ("cursor", wintypes.HANDLE),
            ("brush", wintypes.HBRUSH),
            ("menu", wintypes.LPCWSTR),
            ("name", wintypes.LPCWSTR),
        ]

    bind(user32, "RegisterClassW", wintypes.WORD, [ctypes.POINTER(WindowClass)])
    instance = kernel32.GetModuleHandleW(None)
    window_class = WindowClass()
    window_class.procedure = procedure
    window_class.instance = instance
    window_class.brush = 6  # COLOR_WINDOW + 1
    window_class.name = "ZgxPopupHoverProbeFront"
    require(user32.RegisterClassW(ctypes.byref(window_class)), "RegisterClass failed")
    hwnd = user32.CreateWindowExW(
        0, window_class.name, "zgx hover probe front", 0x10CF0000,
        100, 400, 550, 300, None, None, instance, None
    )
    require(hwnd, "Creating ordinary front window failed")
    print("READY", flush=True)
    message = wintypes.MSG()
    while True:
        result = user32.GetMessageW(ctypes.byref(message), None, 0, 0)
        if result == -1:
            raise ctypes.WinError(ctypes.get_last_error())
        if result == 0:
            break
        user32.TranslateMessage(ctypes.byref(message))
        user32.DispatchMessageW(ctypes.byref(message))


def run_case(binary, mode, target):
    name = f"{mode}/{target}"
    saved_cursor = cursor_position()
    applications = []
    log("case_start", case=name)
    try:
        move_cursor((5, 5))
        app = Application([str(binary)], second=(mode == "same_app"))
        applications.append(app)
        app.ready()
        main = app.window("Popup hover")
        place_window(main, 100, 100, 700, 500)
        point = app.target(main, target)
        if mode == "same_app":
            front = app.window("Popup hover, second")
            front_pid = app.process.pid
        else:
            helper = Application(
                [sys.executable, str(Path(__file__).resolve()), "--front-window"],
            )
            applications.append(helper)
            helper.ready()
            front = helper.window("zgx hover probe front")
            front_pid = helper.process.pid
        place_window(front, point[0] - 45, point[1] + 18, 550, 300)
        activate(front)
        if mode in ("foreground", "deactivate"):
            activate(main)
        pause(0.4)
        verify_target(main, point)
        before = snapshot("before_hover", app, front_pid, main, front)
        assert_popup_count(before, app, target, False)
        hover(main, point)
        after = snapshot("after_hover", app, front_pid, main, front)
        result = compare_states(name, before, after, app, main, front, target)
        assert_stable(result)
        require(result["hover_enter_heard"], f"No HOVER {target} enter was heard")
        showing = mode in ("foreground", "same_app", "deactivate")
        assert_popup_count(after, app, target, showing)
        if showing and target != "overlay" and mode != "same_app":
            require(all(popup["above_front"] for popup in result["popups"]),
                    "Foreground popup is below other process")
        if mode == "same_app":
            require(after["foreground"]["hwnd"] == front,
                    "First window took foreground from same-app second window")
            require(find_window(after, main)["z"] > find_window(after, front)["z"],
                    "First main window is above the same-app second window")
        if mode == "activate":
            # Keep the pointer exactly on the target. This is the only intentional
            # activation after the before/after hover measurement.
            require(cursor_position() == point, "Pointer left target before activation")
            verify_target(main, point)
            activate(main)
            activated_before = snapshot("activation_settled", app, front_pid, main, front)
            pause(1.5)
            activated_after = snapshot("after_activation", app, front_pid, main, front)
            activated = compare_states(
                name + "/activated", activated_before, activated_after,
                app, main, front, target
            )
            assert_stable(activated)
            require(activated_after["foreground"]["hwnd"] == main,
                    "Foreground changed after requested activation")
            require(cursor_position() == point, "Pointer moved during activation")
            assert_popup_count(activated_after, app, target, True)
            require(all(popup["above_front"] for popup in activated["popups"]),
                    "Held popup did not show above front")
        if mode == "deactivate":
            require(cursor_position() == point, "Pointer left target before deactivation")
            activate(front)
            deactivated_before = snapshot("deactivation_settled", app, front_pid, main, front)
            pause(1.5)
            deactivated_after = snapshot("after_deactivation", app, front_pid, main, front)
            deactivated = compare_states(
                name + "/deactivated", deactivated_before, deactivated_after,
                app, main, front, target
            )
            assert_stable(deactivated)
            assert_popup_count(deactivated_after, app, target, False)
            require(deactivated_after["foreground"]["hwnd"] == front,
                    "Foreground changed after requested deactivation")
            require(cursor_position() == point, "Pointer moved during deactivation")
        log("case_pass", case=name)
    finally:
        # Restoring the pointer sends no clicks or keyboard messages. Even if a
        # dialog appeared, it is neither activated nor dismissed here.
        restored = bool(user32.SetCursorPos(*saved_cursor))
        log("cursor_restored", point=saved_cursor, success=restored)
        for application in reversed(applications):
            application.close()
        if not restored:
            raise SafetyFailure("Could not restore the original cursor position")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path, nargs="?")
    parser.add_argument("--front-window", action="store_true", help=argparse.SUPPRESS)
    arguments = parser.parse_args()
    user32.SetProcessDpiAwarenessContext(POINTER(-4))
    if arguments.front_window:
        front_window()
        return 0
    if arguments.binary is None or not arguments.binary.is_file():
        parser.error("Pass the path to a built popup_hover.exe")
    job = create_job()
    check_safety()
    log("start", binary=str(arguments.binary.resolve()), job=job)
    cases = [
        (mode, target)
        for mode in ("back", "foreground")
        for target in ("tip", "card", "row", "overlay")
    ]
    cases.extend(("activate", target) for target in ("tip", "card", "row"))
    cases.extend(("same_app", target) for target in ("tip", "card"))
    cases.append(("deactivate", "tip"))
    failures = []
    for mode, target in cases:
        try:
            run_case(arguments.binary.resolve(), mode, target)
        except ExpectationFailure as error:
            failures.append(f"{mode}/{target}: {error}")
            log("case_fail", case=f"{mode}/{target}", error=str(error))
        except SafetyFailure as error:
            log("safety_abort", case=f"{mode}/{target}", error=str(error))
            return 2
    log("complete", cases=len(cases), failures=failures)
    if failures:
        print("FAIL: " + " | ".join(failures), file=sys.stderr)
        return 1
    print(f"PASS: all {len(cases)} popup hover cases", flush=True)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (SafetyFailure, OSError) as error:
        print(f"ABORT: {error}", file=sys.stderr)
        sys.exit(2)
