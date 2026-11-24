#!/usr/bin/env python3
"""A virtual keyboard on a wlroots compositor (sway): `zwp_virtual_keyboard_v1`,
spoken over the Wayland socket in plain Python (no pywayland), as
`wayland_pointer.VirtualPointer` speaks its pointer. Its keymap holds the
keys of a US keyboard from the start, each on its own code (a client
reading the physical key, as WebKit's editing shortcuts do, sees Ctrl+C on
KeyC), and Sway gives that keymap to every client then, even one never
focused (GTK's display beside a web view, which translates its keys). Any
other keysym or character goes on a spare code, as `wtype` puts every key,
the keymap sent again (to the focused client) when one is added.

    keyboard = VirtualKeyboard(os.environ["XDG_RUNTIME_DIR"], "wayland-1")
    keyboard.key("ctrl+a")            # xdotool's names: X keysym names
    keyboard.type("nihao", delay=.09)
"""
import os
import socket
import struct
import time

# Requests and events by opcode, from the protocols' XML.
DISPLAY_SYNC, DISPLAY_GET_REGISTRY = 0, 1
REGISTRY_BIND = 0
MANAGER_CREATE = 0
KEYMAP, KEY, MODIFIERS = 0, 1, 2
XKB_V1 = 1
PRESSED, RELEASED = 1, 0
# Modifier masks in a keymap that includes "complete" (Shift, Lock,
# Control, Mod1, Mod2, Mod3, Mod4, Mod5).
MASKS = {"shift": 1, "ctrl": 4, "control": 4, "alt": 8, "super": 64, "meta": 8}
# A US keyboard's evdev codes, by keysym name.
EVDEV = {**{c: 16 + i for i, c in enumerate("qwertyuiop")}, **{c: 30 + i for i, c in enumerate("asdfghjkl")},
         **{c: 44 + i for i, c in enumerate("zxcvbnm")}, **{str(d): 1 + d for d in range(1, 10)}, "0": 11,
         **{f"F{n}": 58 + n for n in range(1, 11)}, "F11": 87, "F12": 88,
         "Escape": 1, "minus": 12, "equal": 13, "BackSpace": 14, "Tab": 15, "bracketleft": 26, "bracketright": 27,
         "Return": 28, "semicolon": 39, "apostrophe": 40, "grave": 41, "backslash": 43, "comma": 51, "period": 52,
         "slash": 53, "space": 57, "Home": 102, "Up": 103, "Prior": 104, "Left": 105, "Right": 106, "End": 107,
         "Down": 108, "Next": 109, "Insert": 110, "Delete": 111}
# Codes for the rest, from the top of an X keymap's range down.
SPARE = 247


def _string(text):
    data = text.encode() + b"\0"
    return struct.pack("<I", len(data)) + data + b"\0" * (-len(data) % 4)


def keysym(character):
    """The keysym name of `character` in a keymap: letters and digits by
    their own name, anything else by its code point (`U4F60`)."""
    if character.isascii() and character.isalnum():
        return character
    return "U%04X" % ord(character)


class VirtualKeyboard:
    def __init__(self, runtime_dir, display):
        self.socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.socket.connect(os.path.join(runtime_dir, display))
        self.buffer = b""
        self.next_id = 2
        registry = self._new_id()
        self._send(1, DISPLAY_GET_REGISTRY, struct.pack("<I", registry))
        done = self._new_id()
        self._send(1, DISPLAY_SYNC, struct.pack("<I", done))
        manager = seat = None
        while True:
            sender, opcode, body = self._event()
            if sender == registry and opcode == 0:
                name, = struct.unpack_from("<I", body)
                length, = struct.unpack_from("<I", body, 4)
                interface = body[8:8 + length - 1].decode()
                version, = struct.unpack_from("<I", body, 8 + length + (-length % 4))
                if interface == "zwp_virtual_keyboard_manager_v1" and manager is None:
                    manager = self._new_id()
                    self._send(registry, REGISTRY_BIND, struct.pack("<I", name) + _string(interface)
                               + struct.pack("<II", 1, manager))
                elif interface == "wl_seat" and seat is None:
                    seat = self._new_id()
                    self._send(registry, REGISTRY_BIND, struct.pack("<I", name) + _string(interface)
                               + struct.pack("<II", min(version, 7), seat))
            elif sender == done:
                break
            elif sender == 1 and opcode == 0:
                raise RuntimeError("Wayland protocol error: " + body.hex())
        if manager is None or seat is None:
            raise RuntimeError("the compositor has no zwp_virtual_keyboard_manager_v1 or seat")
        self.keyboard = self._new_id()
        self._send(manager, MANAGER_CREATE, struct.pack("<II", seat, self.keyboard))
        # Its keymap's keys: evdev code to keysym name.
        self.keys = {code: name for name, code in EVDEV.items()}
        self._upload()

    def _new_id(self):
        self.next_id += 1
        return self.next_id - 1

    def _send(self, sender, opcode, body=b"", fds=()):
        data = struct.pack("<II", sender, (8 + len(body)) << 16 | opcode) + body
        if fds:
            self.socket.sendmsg([data], [(socket.SOL_SOCKET, socket.SCM_RIGHTS, struct.pack(f"{len(fds)}i", *fds))])
        else:
            self.socket.sendall(data)

    def _event(self):
        while len(self.buffer) < 8 or len(self.buffer) < (struct.unpack_from("<I", self.buffer, 4)[0] >> 16):
            data = self.socket.recv(4096)
            if not data:
                raise RuntimeError("the compositor closed the connection")
            self.buffer += data
        sender, word = struct.unpack_from("<II", self.buffer)
        size = word >> 16
        body, self.buffer = self.buffer[8:size], self.buffer[size:]
        return sender, word & 0xFFFF, body

    def _upload(self):
        """Send the keymap of its keys."""
        keys = self.keys
        keycodes = "".join(f"<K{code}> = {8 + code};\n" for code in keys)
        symbols = "".join(f"key <K{code}> {{[ {name} ]}};\n" for code, name in keys.items())
        keymap = ("xkb_keymap {\n"
                  f'xkb_keycodes "zgx" {{ minimum = 8; maximum = 255;\n{keycodes}}};\n'
                  'xkb_types "zgx" { include "complete" };\n'
                  'xkb_compatibility "zgx" { include "complete" };\n'
                  f'xkb_symbols "zgx" {{\n{symbols}}};\n'
                  "};\n").encode() + b"\0"
        fd = os.memfd_create("zgx-keymap")
        try:
            os.write(fd, keymap)
            # Its arguments: the format and the size; the descriptor rides
            # beside the message.
            self._send(self.keyboard, KEYMAP, struct.pack("<II", XKB_V1, len(keymap)), fds=[fd])
        finally:
            os.close(fd)
        # Round trip: the compositor has it before a key uses it.
        self._roundtrip()

    def _roundtrip(self):
        done = self._new_id()
        self._send(1, DISPLAY_SYNC, struct.pack("<I", done))
        while True:
            sender, opcode, body = self._event()
            if sender == done:
                return
            if sender == 1 and opcode == 0:
                raise RuntimeError("Wayland protocol error: " + body.hex())

    def _keycode(self, name):
        """The evdev keycode (an xkb keycode less 8) for keysym `name`, added
        to its keymap if new."""
        for code, known in self.keys.items():
            if known == name:
                return code
        code = EVDEV.get(name)
        if code is None:
            code = SPARE - sum(1 for c in self.keys if c not in EVDEV.values())
        self.keys[code] = name
        self._upload()
        return code

    @staticmethod
    def _now():
        return int(time.monotonic() * 1000) & 0xFFFFFFFF

    def _modifiers(self, mask):
        self._send(self.keyboard, MODIFIERS, struct.pack("<IIII", mask, 0, 0, 0))

    def press(self, name, pressed):
        """Press or release keysym `name` (an X keysym name)."""
        self._send(self.keyboard, KEY, struct.pack("<III", self._now(), self._keycode(name),
                                                    PRESSED if pressed else RELEASED))

    def key(self, *combos):
        """Each of `combos` pressed and released in turn, as xdotool's `key`
        takes them: a keysym name, after modifiers joined by `+`
        (`ctrl+shift+Tab`)."""
        for combo in combos:
            *mods, name = combo.split("+")
            mask = 0
            for mod in mods:
                mask |= MASKS[mod.lower()]
            if mask:
                self._modifiers(mask)
            self.press(name, True)
            time.sleep(.02)
            self.press(name, False)
            if mask:
                self._modifiers(0)
            self._roundtrip()

    def type(self, text, delay=.012):
        """`text` typed a character at a time, `delay` seconds apart."""
        for character in text:
            name = "Return" if character == "\n" else "Tab" if character == "\t" else keysym(character)
            self.press(name, True)
            self.press(name, False)
            self._roundtrip()
            time.sleep(delay)

    def close(self):
        self.socket.close()
