"""Visible scroll demand on macOS: accepted fingers and observed app coast."""
import re


def accepted_events(events):
    state, lifted = 'idle', None
    accepted, ignored = [], []
    for event in events:
        at, unit, _, _, phase = event
        if unit != 'pixel':
            accepted.append(event)
            continue
        momentum_end = False
        if phase == 'started':
            if state == 'lifted' and at-lifted < 50:
                state = 'momentum'
            elif state != 'momentum':
                state = 'fingers'
        elif phase in ('ended', 'cancelled'):
            if state == 'fingers':
                state, lifted = 'lifted', at
            elif state == 'momentum':
                state, momentum_end = 'idle', True
        (ignored if state == 'momentum' or momentum_end else accepted).append(event)
    return accepted, ignored


def positions(frame):
    result = {}
    for note in frame.get('notes', []):
        match = re.fullmatch(r'scroll @(\d+) position (-?\d+) (-?\d+) px', note)
        if match:
            result[int(match[1])] = (int(match[2]), int(match[3]))
    return result


def coast_state(frame):
    values = [re.search(r'\bcoasting (true|false)\b', note)
              for note in frame.get('notes', []) if note.startswith('frame source: ')]
    values = [v[1] == 'true' for v in values if v]
    return any(values) if values else None


def select_view(node, initial_frames, scale, horizontal=False):
    match = re.match(r'offset (-?[0-9.]+) of ', node.get('scroll', ''))
    if not match:
        return None
    offset = -float(match[1])*scale
    expected = (offset, 0) if horizontal else (0, offset)
    latest = {}
    for frame in sorted(initial_frames, key=lambda f: f['frame']):
        latest.update(positions(frame))
    candidates = [key for key, value in latest.items()
                  if max(abs(value[i]-expected[i]) for i in (0, 1)) <= 1]
    return candidates[0] if len(candidates) == 1 else None


class Motion:
    def __init__(self, frames, view, initial_frames=()):
        self.view = view
        self.states = {}
        self.unknown = 0
        self.slow_steps = 0
        self.stationary = 0
        self.coast_intervals = 0
        self.finger_intervals = self.finger_late = self.coast_late = 0
        self.coast_long_gaps = 0
        point, coasting, last_context = None, None, None
        self.stale_render_context = 0
        measured_ids = {f['frame'] for f in frames}
        for frame in sorted([*initial_frames, *frames], key=lambda f: f['frame']):
            point = positions(frame).get(view, point)
            state = coast_state(frame)
            if state is not None:
                coasting = state
                last_context = frame['frame']
            if positions(frame) and frame['frame'] in measured_ids and (last_context is None or frame['frame']-last_context > 4):
                self.stale_render_context += 1
            self.states[frame['frame']] = (point, coasting)
        self.metadata = any(coast_state(f) is not None for f in frames)
        self.have_positions = view is not None and any(positions(f).get(view) is not None
                                                     for f in [*initial_frames, *frames])

    def interval(self, previous, current, finger, period):
        """Return (visible demand, late); unknown coverage never becomes a pass."""
        ids = range(previous[2], current[2]+1) if previous[2] and current[2] >= previous[2] else []
        values = [self.states[i] for i in ids if i in self.states]
        coast = any(state is True for _, state in values)
        gap = (current[1]-previous[1])*1000
        if not finger and not coast:
            return False, False
        points = [point for point, _ in values if point is not None]
        if not points or not self.have_positions:
            if gap > 1.5*period:
                self.unknown += 1
            return finger, finger and gap > 1.5*period
        distance = max(max(p[axis] for p in points)-min(p[axis] for p in points)
                       for axis in (0, 1))
        if distance == 0:
            self.stationary += 1
            if finger and gap > 1.5*period:
                self.unknown += 1  # lost input, clamping or cancellation needs evidence
            return False, False
        if coast and not finger:
            self.coast_intervals += 1
            self.coast_long_gaps += gap > 1.5*period
        if finger:
            self.finger_intervals += 1
        if gap > 1.5*period and distance <= 1:
            self.slow_steps += 1
            if not coast:
                return True, False  # finger input can be pixel-limited
            # An observed moving coast is refresh-paced, even at a one-pixel step.
        late = gap > 1.5*period
        if late:
            if finger:self.finger_late += 1
            elif coast:self.coast_late += 1
        return True, late

    def summary(self):
        return dict(cadence_model='accepted-input-and-observed-coast',
                    cadence_scoring_complete=self.metadata and self.have_positions and self.unknown == 0 and self.stale_render_context == 0,
                    coast_context_missing_frames=self.stale_render_context,
                    finger_active_intervals=self.finger_intervals,
                    finger_late_intervals=self.finger_late,coast_late_intervals=self.coast_late,
                    coast_long_gap_intervals=self.coast_long_gaps,coast_active_intervals=self.coast_intervals,
                    coast_slow_step_intervals=self.slow_steps,
                    stationary_scroll_intervals=self.stationary,
                    unclassified_motion_intervals=self.unknown)
