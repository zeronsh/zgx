import unittest
from scroll_motion import accepted_events, Motion, select_view


class MotionEvidence(unittest.TestCase):
    def test_system_momentum_is_not_finger_demand_and_boundary_is_strict(self):
        events=[(0,'pixel',0,0,'started'),(10,'pixel',0,0,'ended'),
                (59.999,'pixel',0,10,'started'),(70,'pixel',0,10,'moved'),
                (80,'pixel',0,0,'ended'),(90,'pixel',0,10,'started')]
        kept, ignored=accepted_events(events)
        self.assertEqual(len(ignored),3)
        self.assertEqual(kept[-1],events[-1])
        kept, ignored=accepted_events([events[0],events[1],(60,'pixel',0,10,'started')])
        self.assertFalse(ignored)

    def frame(self, n, y, coast=True):
        return dict(frame=n, notes=[f'scroll @7 position 0 {y} px',
                    f'frame source: view link (wanted true, rate None, scroll live true, coasting {str(coast).lower()}, input false)'])

    def test_target_mapping_is_unique_and_uses_device_scale(self):
        frames=[self.frame(1,-200)]
        self.assertEqual(select_view({'scroll':'offset 100 of 0 to 1000'},frames,2),7)
        frames[0]['notes'].append('scroll @8 position 0 -200 px')
        self.assertIsNone(select_view({'scroll':'offset 100 of 0 to 1000'},frames,2))

    def test_moving_coast_requires_each_refresh_even_for_one_pixel(self):
        a=(.001,.001,1,0);b=(.018,.018,2,0)
        m=Motion([self.frame(1,0),self.frame(2,-10)],7)
        self.assertEqual(m.interval(a,b,False,1000/120),(True,True))
        m=Motion([self.frame(1,0),self.frame(2,-1)],7)
        self.assertEqual(m.interval(a,b,False,1000/120),(True,True))
        self.assertEqual(m.summary()['coast_slow_step_intervals'],1)

    def test_pixel_limited_fingers_do_not_require_each_refresh(self):
        m=Motion([self.frame(1,0,False),self.frame(2,-1,False)],7)
        self.assertEqual(m.interval((.001,.001,1,0),(.018,.018,2,0),True,1000/120),(True,False))
        self.assertEqual(m.summary()['finger_late_intervals'],0)

    def test_finger_and_coast_late_counts_are_separate(self):
        m=Motion([self.frame(1,0),self.frame(2,-10),self.frame(3,-20)],7)
        m.interval((.001,.001,1,0),(.018,.018,2,0),True,1000/120)
        m.interval((.018,.018,2,0),(.035,.035,3,0),False,1000/120)
        self.assertEqual(m.summary()['finger_late_intervals'],1)
        self.assertEqual(m.summary()['coast_late_intervals'],1)
        self.assertEqual(m.summary()['coast_active_intervals'],1)
        self.assertEqual(m.summary()['finger_active_intervals'],1)

    def test_canceled_intermediate_position_still_proves_missing_motion(self):
        frames=[self.frame(1,0),self.frame(2,-10),self.frame(3,0)]
        m=Motion(frames,7)
        self.assertEqual(m.interval((.001,.001,1,0),(.018,.018,3,0),False,1000/120),(True,True))

    def test_unknown_positions_never_certify_and_nonrender_frames_carry_position(self):
        frames=[self.frame(1,0),dict(frame=2,notes=[])]
        m=Motion(frames,7)
        self.assertEqual(m.states[2][0],(0,0))
        m=Motion(frames,None)
        m.interval((.001,.001,1,0),(.018,.018,2,0),True,1000/120)
        self.assertFalse(m.summary()['cadence_scoring_complete'])

    def test_stationary_dropped_momentum_is_not_a_missed_refresh(self):
        m=Motion([self.frame(1,0,False),self.frame(2,0,False)],7)
        self.assertEqual(m.interval((.001,.001,1,0),(.05,.05,2,0),False,1000/120),(False,False))


if __name__ == '__main__':unittest.main()
