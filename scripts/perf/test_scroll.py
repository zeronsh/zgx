#!/usr/bin/env python3
"""Measurement integrity: half-rate, idle gaps, quantization and missing evidence."""
import argparse
from pathlib import Path
import tempfile
from unittest.mock import patch
import unittest
import scroll


class ScrollMetrics(unittest.TestCase):
    def metrics(self, step, gesture='fling', skipped=0):
        events = scroll.parse_tape(scroll.tape(gesture, 120))
        presents = [(t / 1000, t / 1000) for t in range(step, 3990, step)]
        presents.extend([(1., 0.)] * skipped)
        frames = [dict(frame=i, notes=['scroll @1 copied its pixels'], phases=[
            dict(phase='update', micros=1000, instructions=100000),
            dict(phase='wait', micros=10000, instructions=100)], gpu_micros=500)
                  for i in range(40)]
        return scroll.summarize(frames, presents, events, 120, 2, 0, {})

    def test_half_rate_cannot_redefine_refresh_to_pass(self):
        m = self.metrics(17)
        self.assertGreater(m['late_pct'], 95)
        self.assertGreater(m['interval_ms']['p99'], 16)

    def test_full_rate_passes(self):
        m = self.metrics(8)
        self.assertEqual(m['late_pct'], 0)
        self.assertEqual(m['main_over_4ms'], 0)  # wait is not CPU work

    def test_canceled_submissions_do_not_double_count_visible_missed_refreshes(self):
        m=self.metrics(8,skipped=20)
        self.assertEqual(m['late_pct'],0)
        self.assertEqual(m['skipped_drawables'],20)
        self.assertGreater(m['presentation_incident_pct'],3)

    def test_idle_gap_is_not_a_missed_refresh(self):
        p = 1000/120
        events = [(0, 'pixel', 0, -10, 'moved'), (p, 'pixel', 0, 0, 'ended'),
                  (1000, 'pixel', 0, -10, 'moved')]
        m = scroll.summarize([], [(0.001, .001), (1.001, 1.001)], events, 120, 2, 0, {})
        self.assertEqual(m['idle_intervals'], 1)
        self.assertIsNone(m['late_pct'])

    def test_slow_quantized_pixels_are_not_late(self):
        m = self.metrics(25, 'slow')
        self.assertEqual(m['late_frames'], 0)
        self.assertGreater(m['idle_intervals'], 10)

    def test_missing_feedback_never_passes(self):
        m = scroll.summarize([], [], [], 120, 2, 0, {})
        args = argparse.Namespace(min_intervals=30, max_late=0, max_rows=0, max_main_ms=4,
                                  max_instructions=50_000_000, max_gpu_ms=8)
        self.assertTrue(scroll.failures(m, args))

    def test_late_attribution_uses_exact_frame_records(self):
        events = scroll.parse_tape(scroll.tape('fling',120))
        frame = dict(frame=1, notes=['scroll @1 copied its pixels'],
                     phases=[dict(phase='paint',micros=5000,instructions=1000)], gpu_micros=1000)
        m = scroll.summarize([frame],[(.01,.01,1,0),(.03,.03,1,0)],events,120,2,0,{})
        self.assertEqual(m['app_late_frames'],1)
        frame['phases'][0]['micros']=1000
        m = scroll.summarize([frame],[(.01,.01,1,0),(.03,.03,1,0)],events,120,2,0,{})
        self.assertEqual(m['system_late_frames'],0)
        self.assertEqual(m['unattributed_late_frames'],1)
        frame['gpu_micros']=6000
        m = scroll.summarize([frame],[(.01,.01,1,0),(.03,.03,1,0)],events,120,2,0,{})
        self.assertEqual(m['late_attributions'][0]['cause'],'gpu')

    def test_late_details_keep_present_wait_and_nearby_phase_changes(self):
        events = [(0,'pixel',0,-20,'started'),(8,'pixel',0,-20,'moved'),
                  (16,'pixel',0,-20,'moved'),(24,'pixel',0,-20,'moved')]
        frame = dict(frame=1, notes=['scroll @1 composited its picture'],
                     phases=[dict(phase='paint',micros=1000,instructions=1000),
                             dict(phase='present',micros=7000,instructions=100),
                             dict(phase='wait',micros=2000,instructions=10)],gpu_micros=500)
        m=scroll.summarize([frame],[(.001,.001,1,0),(.024,.024,1,0)],events,120,2,0,{})
        a=m['late_attributions'][0]
        self.assertEqual(a['cause'],'unattributed')
        self.assertEqual(a['phase_ms']['present'],7)
        self.assertEqual(a['nearby_phase_changes'][0]['phase'],'started')
        self.assertEqual(m['main_work_ms']['max'],1)
        self.assertEqual(m['main_with_present_ms']['max'],8)
        self.assertEqual(m['picture_frames'],1)

    def test_missing_frame_identity_is_not_called_system_noise(self):
        m = self.metrics(17)
        self.assertGreater(m['unattributed_late_frames'],0)
        self.assertEqual(m['system_late_frames'],0)

    def test_main_max_is_a_gate_even_without_lateness(self):
        m = self.metrics(8)
        m['main_work_ms']['max']=4.01
        args = argparse.Namespace(min_intervals=30,max_late=0,max_rows=0,max_main_ms=4,
                                  max_instructions=50_000_000,max_gpu_ms=4)
        self.assertTrue(any('main-thread work max' in e for e in scroll.failures(m,args)))

    def test_stream_motion_stays_near_its_visible_tail(self):
        offset=low=high=0
        for _,_,_,dy,_ in scroll.parse_tape(scroll.tape('stream',120)):
            offset-=dy
            low=min(low,offset);high=max(high,offset)
        self.assertLessEqual(high-low,122)
        self.assertEqual(offset,0)

    def test_drawable_reference_uses_view_link_without_an_explicit_view_flag(self):
        m={'wait_ms':{'p99':0},'scroll_frames':40,'render_frames':20,'frame_source_counts':{'view link':20}}
        checks=scroll.behavior_checks(m,[],[(1,1,1,.99)],'1',True,True)
        self.assertTrue(checks['frame_source_is_view link'])
        checks=scroll.behavior_checks(m,[],[(1,1,1,.99)],'auto',True,True)
        self.assertTrue(checks['frame_source_is_view link'])

    def test_lead_marker_probe_cannot_silently_bypass_the_hold(self):
        import contextlib,io
        from unittest.mock import patch
        with patch('sys.argv',['scroll.py','reference','--host-env','ZGX_LEAD_US=4000']),contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as result:scroll.main()
        self.assertEqual(result.exception.code,2)

    def test_material_condition_must_match_effective_ui_state(self):
        self.assertTrue(scroll.material_matches({'bench_effective_frost':False},'opaque'))
        self.assertFalse(scroll.material_matches({'bench_effective_frost':False},'translucent'))
        self.assertFalse(scroll.material_matches({},'opaque'))
        self.assertTrue(scroll.material_matches({'bench_effective_frost':True},'translucent'))

    def test_pacing_source_mismatch_invalidates_a_received_flag(self):
        m={'wait_ms':{'p99':0},'scroll_frames':20,'frame_source_counts':{'timer on the grid':20}}
        flags=['ZGX_VIEW_CLOCK=1','ZGX_SCROLL_LINK=1']
        self.assertFalse(scroll.behavior_checks(m,flags,[],'0',False)['frame_source_is_view link'])
        m['frame_source_counts']={'view link':20}
        self.assertTrue(scroll.behavior_checks(m,flags,[],'0',False)['frame_source_is_view link'])

    def test_flag_verification_checks_exact_actual_child_values(self):
        checks=scroll.verify_environment_text('pid executable ZGX_LEAD_US=40000 ZGX_VIEW_CLOCK=1',
                                               ['ZGX_LEAD_US=4000','ZGX_VIEW_CLOCK=1'])
        self.assertFalse(checks['ZGX_LEAD_US=4000'])
        self.assertTrue(checks['ZGX_VIEW_CLOCK=1'])

    def test_flag_behavior_rejects_a_baseline_disguised_as_asap_or_hybrid(self):
        m={'wait_ms':{'p99':4}}
        checks=scroll.behavior_checks(m,['ZGX_HANDOVER_ASAP=1','ZGX_SCROLL_DRAWABLES=1'],[(1,1,1,0)],'0',False)
        self.assertFalse(checks['asap_no_hold'])
        self.assertFalse(checks['hybrid_drawables_during_scroll'])
        self.assertTrue(scroll.behavior_checks(m,['ZGX_SCROLL_DRAWABLES=1'],[(1,1,1,0)],'0',True)['hybrid_tiles_at_rest'])

    def test_unnoted_row_build_during_input_is_charged_to_scroll_work(self):
        events=scroll.parse_tape(scroll.tape('fling',120))
        frame=dict(frame=1,ended_ms=10,frame_micros=6000,notes=['list built row'],
                   phases=[dict(phase='layout',micros=6000,instructions=1000)],gpu_micros=500)
        m=scroll.summarize([frame],[],events,120,2,0,{},trace_to_host_ms=0)
        self.assertEqual(m['rows_built_in_scroll_frames'],1)
        self.assertEqual(m['main_over_4ms'],1)
        self.assertEqual(m['main_work_ms']['max'],6)
        self.assertEqual(m['motion_window_work']['rows_built'],1)
        self.assertEqual(m['motion_window_work']['phases_ms']['rest']['max'],6)
        self.assertEqual(m['noted_scroll_frames'],0)

    def test_explicit_gpu_stats_survives_inherited_environment_sanitizing(self):
        env=scroll.host_environment(['ZGX_GPU_STATS=1'],'0',
             {'ZGX_GPU_STATS':'old','ZERON_DATA_DIR':'production','ZGUI_OLD':'1','PATH':'/bin'})
        self.assertEqual(env['ZGX_GPU_STATS'],'1')
        self.assertNotIn('ZERON_DATA_DIR',env)
        self.assertNotIn('ZGUI_OLD',env)
        self.assertEqual(env['PATH'],'/bin')

    def test_picture_cost_groups_overlap_and_retain_the_worst_frame(self):
        def frame(n, notes, work):
            return dict(frame=n,notes=notes,phases=[dict(phase='paint',micros=work,instructions=100)],gpu_micros=500)
        result=scroll.picture_costs([
            frame(1,['scroll @1 made a picture','scroll @1 rastered 900 px rows'],30000),
            frame(2,['scroll @1 composited its picture, moved 8 px'],1000),
            frame(3,['scroll @1 rastered 10 px rows','scroll @1 composited its picture, moved 8 px'],2000)])
        self.assertEqual(result['promotion']['count'],1)
        self.assertEqual(result['raster']['count'],2)
        self.assertEqual(result['composite_only']['count'],1)
        self.assertEqual(result['raster']['main_ms']['max'],30)
        self.assertEqual(result['raster']['worst_main_frame']['frame'],1)

    def test_same_refresh_frames_do_not_hide_a_following_skip(self):
        events=scroll.parse_tape(scroll.tape('fling',120))
        frames=[dict(frame=i,notes=['scroll @1 copied its pixels'],
                     phases=[dict(phase='paint',micros=1000,instructions=100)],gpu_micros=500)
                for i in (1,2,3)]
        m=scroll.summarize(frames,[(.01,.01,1,0),(.01,.01,2,0),(.03,.03,3,0)],events,120,2,0,{})
        self.assertEqual(m['same_refresh_extra_frames'],1)
        self.assertEqual(m['double_then_skip_intervals'],1)
        self.assertEqual(m['active_intervals'],1)
        self.assertEqual(m['late_pct'],100)

    def test_duplicate_feedback_is_distinct_from_two_frames(self):
        _,groups=scroll.coalesce_presentations([(.01,.01,1,0),(.011,.01,1,0),(.018,.018,2,0)],1000/120)
        self.assertEqual(len(groups),2)
        self.assertEqual(len({r[2] for r in groups[0]}),1)

    def test_trace_clock_latency_estimates_are_labeled_and_calibrated(self):
        frame=dict(frame=1,ended_ms=20,frame_micros=2000,notes=['scroll @1 copied its pixels'],
                   phases=[dict(phase='paint',micros=1000,instructions=100)],gpu_micros=500)
        m=scroll.summarize([frame],[(1.027,1.026,1,1.017)],scroll.parse_tape(scroll.tape('fling',120)),120,2,1000,{},trace_to_host_ms=1000)
        self.assertAlmostEqual(m['render_start_to_shown_ms_estimate']['p99'],8)
        self.assertAlmostEqual(m['frame_end_to_shown_ms_estimate']['p99'],6)
        self.assertAlmostEqual(m['render_start_estimate_error_ms']['p99'],-1)

    def test_real_tape_is_preserved_and_longer_than_a_minute(self):
        events = scroll.parse_tape(scroll.tape('wing', 120))
        self.assertGreater(events[-1][0], 94000)
        self.assertEqual(events[-1][-1], 'ended')


class NativeReference(unittest.TestCase):
    def test_skipped_native_drawable_and_its_gap_remain_visible(self):
        with tempfile.TemporaryDirectory() as directory:
            args=argparse.Namespace(output=Path(directory),native_scale=2)
            def run(command,stdout,**kwargs):
                stdout.write('NATIVE_BEGIN 1\n')
                for n in range(1200):
                    t=1+n/120
                    stdout.write(f'NATIVE_WAKE {n} {t} {t} {t+1/120}\n')
                    shown=0 if n==20 else t+.005
                    stdout.write(f'NATIVE_PRESENT {n} {t+.006} {shown} {t}\n')
                    stdout.write(f'NATIVE_GPU {n} 0.1\nNATIVE_WORK {n} 0.2\n')
                stdout.write('NATIVE_END 11\n')
            with patch.object(scroll.subprocess,'run',side_effect=run),patch.object(scroll,'sha',return_value='hash'):
                m=scroll.native_floor(args,'W',120,1,'0','native')
            self.assertEqual(m['native_points'],[557,342.5])
            self.assertEqual(m['skipped_drawables'],1)
            self.assertEqual(m['late_frames'],1)
            self.assertGreater(m['late_pct'],0)
            self.assertAlmostEqual(m['display_period_ms']['p50'],1000/120)


    def test_native_timeout_is_preserved_as_invalid_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            args=argparse.Namespace(output=Path(directory),native_scale=2)
            with patch.object(scroll.subprocess,'run',side_effect=scroll.subprocess.TimeoutExpired('native',30)),patch.object(scroll,'sha',return_value='hash'):
                m=scroll.native_floor(args,'W',120,2,'0','timeout')
            self.assertEqual(m['status'],'invalid')
            self.assertTrue((Path(directory)/'timeout/run.json').is_file())


class CollectionIntegrity(unittest.TestCase):
    def test_disappearing_transcript_invalidates_the_sample(self):
        with tempfile.TemporaryDirectory() as directory:
            def answer(pid,cmd,**kwargs):
                if cmd=='frames': return {'data':[]}
                if cmd=='mark': return {'data':{'frame':1}}
                raise RuntimeError('nothing matches #transcript')
            with patch.object(scroll,'request',side_effect=answer):
                with self.assertRaisesRegex(RuntimeError,'nothing matches'):
                    scroll.collect(1,None,{'frame':0},Path(directory),'#transcript')


class DisplayModes(unittest.TestCase):
    def modes(self, directory):
        modes = object.__new__(scroll.RefreshModes)
        original = dict(id=1,width=1728,height=1117,pixel_width=3456,pixel_height=2234,hz=0)
        modes.original = dict(current=original,modes=[original,dict(original,id=2,hz=60)])
        modes.current, modes.actions = 1, []
        modes.binary, modes.output = Path('/fake/scroll-display'), Path(directory)
        return modes

    def test_a_real_mode_is_selected_and_the_original_is_restored(self):
        with tempfile.TemporaryDirectory() as directory:
            modes = self.modes(directory)
            with patch.object(scroll.subprocess,'check_output',return_value='{}'), patch.object(scroll.time,'sleep'):
                modes.select(60)
            self.assertEqual(modes.current,2)
            with patch.object(scroll.subprocess,'run') as run:
                modes.restore()
            self.assertEqual(run.call_args.args[0][-1],'1')
            self.assertTrue((Path(directory)/'display-modes.json').is_file())

    def test_current_mode_is_usable_even_if_not_in_enumeration(self):
        with tempfile.TemporaryDirectory() as directory:
            modes=self.modes(directory)
            modes.original['current']['hz']=120
            modes.original['modes']=[]
            with patch.object(scroll.subprocess,'check_output') as change:
                modes.select(120)
            change.assert_not_called()

    def test_a_mode_with_different_dimensions_is_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            modes = self.modes(directory)
            modes.original['modes'][1]['pixel_width']=1920
            with self.assertRaisesRegex(RuntimeError,'no actual 60 Hz mode'):
                modes.select(60)
            self.assertEqual(modes.current,1)


if __name__ == '__main__':
    unittest.main()
