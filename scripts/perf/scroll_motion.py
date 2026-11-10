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


def applied(frame):
    """Deltas dispatched after native momentum filtering and resampling."""
    result = []
    number = r'(-?\d+(?:\.\d+)?)'
    pattern = (r'scroll-applied source=(direct|end|refresh|input|coast) dx='+number+
               r' dy='+number+r'(?: delay_ms='+number+r' refresh_ms='+number+r')? at_ns=(\d+)')
    for note in frame.get('notes', []):
        match = re.fullmatch(pattern, note)
        if match:
            result.append((match[1], float(match[2]), float(match[3]), int(match[6])))
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


def live_state(frame):
    values = [re.search(r'\bscroll live (true|false)\b', note)
              for note in frame.get('notes', []) if note.startswith('frame source: ')]
    values = [v[1] == 'true' for v in values if v]
    return any(values) if values else None


class Motion:
    def __init__(self, frames, view, initial_frames=(), *, axis=1, scale=1):
        self.view = view
        self.axis, self.scale = axis, scale
        self.applied = {f['frame']: applied(f) for f in frames}
        self.have_applied = any(self.applied.values())
        self.states = {}
        self.live = {}
        self.moved_frames = set()
        self.onsets = self.app_animated = 0
        self.unknown = 0
        self.slow_steps = 0
        self.finger_slow_steps = self.coast_slow_steps = 0
        self.stationary = 0
        self.coast_intervals = 0
        self.finger_intervals = self.finger_late = self.coast_late = 0
        self.coast_long_gaps = 0
        point, coasting, live, last_context = None, None, None, None
        self.stale_render_context = 0
        measured_ids = {f['frame'] for f in frames}
        for frame in sorted([*initial_frames, *frames], key=lambda f: f['frame']):
            old_point = point
            point = positions(frame).get(view, point)
            if old_point is not None and point != old_point and frame['frame'] in measured_ids:
                self.moved_frames.add(frame['frame'])
            state = coast_state(frame)
            if state is not None:
                coasting = state
                last_context = frame['frame']
            if positions(frame) and frame['frame'] in measured_ids and (last_context is None or frame['frame']-last_context > 4):
                self.stale_render_context += 1
            current_live = live_state(frame)
            if current_live is not None:live = current_live
            self.live[frame['frame']] = live
            self.states[frame['frame']] = (point, coasting)
        self.metadata = any(coast_state(f) is not None for f in frames)
        self.have_live = any(live_state(f) is not None for f in frames)
        self.have_positions = view is not None and any(positions(f).get(view) is not None
                                                     for f in [*initial_frames, *frames])

    def interval(self, previous, current, finger, period, *, onset=False):
        """Return (visible demand, late); unknown coverage never becomes a pass."""
        ids = range(previous[2], current[2]+1) if previous[2] and current[2] >= previous[2] else []
        values = [self.states[i] for i in ids if i in self.states]
        coast = any(state is True for _, state in values)
        dispatched = [v for i in range(previous[2]+1, current[2]+1)
                      for v in self.applied.get(i, ())]
        path = [0.]
        for source, dx, dy, _ in dispatched:
            if source != 'coast':
                path.append(path[-1]+(dx,dy)[self.axis]*self.scale)
        finger_distance = max(path)-min(path)
        if self.have_applied:
            finger = any(source != 'coast' and (dx,dy)[self.axis] != 0
                         for source,dx,dy,_ in dispatched)
        gap = (current[1]-previous[1])*1000
        if not finger and not coast:
            points = [point for point, _ in values if point is not None]
            if len(set(points)) > 1:self.app_animated += 1
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
            if finger and gap > 1.5*period and (not self.have_applied or finger_distance >= 1):
                self.unknown += 1  # dispatched motion without a position change needs evidence
            return False, False
        if onset:
            self.onsets += 1
            return False, False
        if coast and not finger:
            self.coast_intervals += 1
            self.coast_long_gaps += gap > 1.5*period
        if finger:
            self.finger_intervals += 1
        if gap > 1.5*period and distance <= 1:
            self.slow_steps += 1
            if finger:self.finger_slow_steps += 1
            else:self.coast_slow_steps += 1
            if not coast:
                return True, False  # finger input can be pixel-limited
            # An observed moving coast is refresh-paced, even at a one-pixel step.
        late = gap > 1.5*period
        if late:
            if finger:self.finger_late += 1
            elif coast:self.coast_late += 1
        return True, late

    def presented_intervals(self, shown, period):
        """Background presents must not hide a held scroll position."""
        previous_index = 0
        for index, current in enumerate(shown[1:], 1):
            previous = shown[previous_index]
            states = [self.live[i] for i in range(previous[2], current[2]) if i in self.live]
            continuous = bool(states) and all(state is True for state in states)
            demand, late = self.interval(previous, current, False, period, onset=not continuous)
            if demand:
                yield previous_index, previous, current, True, late
                previous_index = index
            else:
                yield index-1, shown[index-1], current, False, False
                if not continuous or self.live.get(current[2]) is not True:
                    previous_index = index

    def summary(self):
        return dict(cadence_model='applied-input-and-observed-coast',
                    motion_onset_intervals=self.onsets, app_animated_intervals=self.app_animated,
                    applied_finger_entries=sum(v[0] != 'coast' for values in self.applied.values() for v in values),
                    applied_coast_entries=sum(v[0] == 'coast' for values in self.applied.values() for v in values),
                    cadence_scoring_complete=self.metadata and self.have_live and self.have_applied and self.have_positions and self.unknown == 0 and self.stale_render_context == 0,
                    coast_context_missing_frames=self.stale_render_context,
                    finger_active_intervals=self.finger_intervals,
                    finger_late_intervals=self.finger_late,coast_late_intervals=self.coast_late,
                    coast_long_gap_intervals=self.coast_long_gaps,coast_active_intervals=self.coast_intervals,
                    slow_step_intervals=self.slow_steps,
                    finger_slow_step_intervals=self.finger_slow_steps,
                    coast_slow_step_intervals=self.coast_slow_steps,
                    stationary_scroll_intervals=self.stationary,
                    unclassified_motion_intervals=self.unknown)
