"""Native AT-SPI checks invoked by atspi_smoke on its private desktop."""
import json
import select
import subprocess
import sys
import time


def run(Atspi, find, wait_for, events, output):
    target = wait_for(lambda: find("Projection target"), "nested label")
    outer = wait_for(lambda: find("Move outer"), "outer scroll action")
    inner = wait_for(lambda: find("Move inner"), "inner scroll action")
    entry = wait_for(lambda: find("Projection focus"), "focus field")

    def bounds(node):
        node.clear_cache()
        r = node.get_component_iface().get_extents(Atspi.CoordType.SCREEN)
        return (r.x, r.y, r.width, r.height)

    def activate(node):
        action = node.get_action_iface()
        assert action is not None and Atspi.Action.do_action(action, 0)

    # Keep input arriving while reading geometry: coalescing all updates
    # until idle must fail this part even if the final stopped box is right.
    origin = bounds(target)
    driver = subprocess.Popen([sys.executable, __file__, "--drive"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    moving = []
    try:
        assert select.select([driver.stdout], [], [], 10)[0], "scroll driver startup"
        assert driver.stdout.readline().strip() == "READY", "scroll driver ready"
        previous = bounds(target)
        for _ in range(6):
            def changed_while_scrolling():
                current = bounds(target)
                return current if driver.poll() is None and current != previous else None
            current = wait_for(changed_while_scrolling, "bounds refreshed during continuous nested scrolling", timeout=3)
            text = target.get_text_iface()
            r = text.get_range_extents(0, 10, Atspi.CoordType.SCREEN)
            # Native queries are separate calls while input continues; the
            # next geometry update may land between them. Both containers
            # move within 0..20 points, so the range must remain in this live envelope.
            assert origin[1] - 41 <= r.y <= origin[1] + origin[3], (origin, r.y)
            moving.append(current)
            previous = current
        assert driver.wait(timeout=8) == 0, driver.stderr.read()
    finally:
        if driver.poll() is None:
            driver.terminate()
            driver.wait(timeout=3)
        driver.stdout.close()
        driver.stderr.close()

    initial = wait_for(lambda: origin if bounds(target) == origin else None, "final nested scroll reset")
    offsets = [0, 0]
    samples = []
    for step in range(24):
        axis = step % 2
        offsets[axis] = (offsets[axis] + 4) % 24
        activate((outer, inner)[axis])
        expected = initial[1] - sum(offsets)
        # The existing geometry pacer is retained. A stale snapshot forever,
        # or a missed nested translation, must fail rather than look idle.
        current = wait_for(lambda: (r if abs(r[1] - expected) <= 1 else None)
                           if (r := bounds(target)) else None, "live nested bounds", timeout=2)
        component = target.get_component_iface()
        hit = component.get_accessible_at_point(current[0] + current[2] // 2,
                                               current[1] + current[3] // 2,
                                               Atspi.CoordType.SCREEN)
        assert hit is not None, "live native hit test"
        # A text run can be the hit child; it must belong to this label.
        names = []
        for _ in range(4):
            if hit is None:
                break
            names.append(hit.get_name())
            hit = hit.get_parent()
        assert "Projection target" in names, names
        text = target.get_text_iface()
        assert text is not None and text.get_text(0, -1).startswith("Projection target")
        r = text.get_range_extents(0, 10, Atspi.CoordType.SCREEN)
        assert current[1] - 1 <= r.y <= current[1] + current[3], (current, r.y)
        assert entry.get_component_iface().grab_focus()
        def focused():
            entry.clear_cache()
            return entry.get_state_set().contains(Atspi.StateType.FOCUSED)
        wait_for(focused, "native focus")
        editor = entry.get_text_iface()
        if editor.get_n_selections():
            assert editor.remove_selection(0)
        assert editor.add_selection(0, 9)
        selection = wait_for(lambda: editor.get_selection(0) if editor.get_n_selections() else None, "native selection")
        assert (selection.start_offset, selection.end_offset) == (0, 9)
        assert editor.get_text(0, -1) == "selection stays live"
        samples.append(dict(step=step, offsets=list(offsets), bounds=current, hit_names=names))
    wait_for(lambda: any(e["type"].startswith("object:bounds-changed") for e in events), "native bounds events")
    (output / "events.json").write_text(json.dumps(events, indent=2) + "\n")
    (output / "results.json").write_text(json.dumps(dict(ok=True, moving_snapshots=moving, samples=samples), indent=2) + "\n")


def drive():
    import gi
    gi.require_version("Atspi", "2.0")
    from gi.repository import Atspi
    Atspi.init()

    def find(node, name, depth=0):
        if node is None or depth > 30:
            return None
        if node.get_name() == name:
            return node
        for i in range(node.get_child_count()):
            found = find(node.get_child_at_index(i), name, depth + 1)
            if found is not None:
                return found
        return None

    desktop = Atspi.get_desktop(0)
    buttons = [find(desktop, name) for name in ("Move outer", "Move inner")]
    assert all(button is not None for button in buttons)
    print("READY", flush=True)
    # 180 actions finish with both offsets back at zero (90 moves each).
    for step in range(180):
        assert buttons[step % 2].get_action_iface().do_action(0)
        time.sleep(.012)


if __name__ == "__main__":
    assert sys.argv[1:] == ["--drive"]
    drive()
