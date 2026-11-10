"""Native AT-SPI checks invoked by atspi_smoke on its private desktop."""
import json


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

    initial = bounds(target)
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
    (output / "results.json").write_text(json.dumps(dict(ok=True, samples=samples), indent=2) + "\n")
