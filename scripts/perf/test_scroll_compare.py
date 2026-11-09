import unittest
from scroll_compare import stats,summarize,lead_phase_check,table

class ComparisonEvidence(unittest.TestCase):
    def phase_metrics(self, lead, aligned=True):
        period=1000/120
        return {'wait_ms':{'p50':0},'presentation_timings':{'samples':[
            {'shown_s':1+n/120,'clock_aligned':aligned,
             'handover':{'end_ns':int(((1+n/120)*1000-lead-2*period)*1e6),'micros':0}}
            for n in range(12)]}}

    def test_lead_phase_is_valid_even_when_wait_was_not_profiled(self):
        result=lead_phase_check(self.phase_metrics(1.6),self.phase_metrics(4),4000,1000/120)
        self.assertTrue(result['valid'])
        self.assertAlmostEqual(result['delta_ms'],2.4,places=5)

    def test_lead_requires_clock_evidence_and_the_intended_earlier_phase(self):
        self.assertFalse(lead_phase_check(self.phase_metrics(1.6),self.phase_metrics(4,False),4000,1000/120)['valid'])
        self.assertFalse(lead_phase_check(self.phase_metrics(1.6),self.phase_metrics(1.6),4000,1000/120)['valid'])
        mixed=self.phase_metrics(4)
        mixed['presentation_timings']['samples'][0]['clock_aligned']=False
        self.assertFalse(lead_phase_check(self.phase_metrics(1.6),mixed,4000,1000/120)['valid'])

    def test_reference_table_does_not_call_the_marker_a_wing_replay(self):
        sample=dict(arm='clock',role='variant',valid=True,status='pass',
                    condition={'scene':'zeron','gesture':'wing','command':'reference'},
                    metrics=dict(late_pct=0,interval_ms={'p99':8.33},main_work_ms={'max':1}))
        self.assertIn('zeron-idle / reference',table(summarize([sample],3)))

    def test_spread_keeps_raw_trials_and_sample_standard_deviation(self):
        s=stats([5,8,11])
        self.assertEqual(s['values'],[5,8,11])
        self.assertEqual(s['mean'],8)
        self.assertEqual(s['stdev'],3)

    def test_missing_trials_cannot_certify_a_zero_late_cell(self):
        m=dict(late_pct=0,interval_ms={'p99':8.33},main_work_ms={'max':1})
        sample=dict(arm='picture',role='variant',valid=True,status='pass',condition={},metrics=m)
        rows=summarize([sample],3)
        variant=next(r for r in rows if r['role']=='variant')
        self.assertFalse(variant['complete'])
        self.assertFalse(variant['all_gates_pass'])

    def test_rest_memory_is_separate_from_motion_end_and_missing_is_unknown(self):
        sample=dict(arm='prefetch',role='variant',valid=True,status='fail',condition={},
                    metrics=dict(late_pct=1,interval_ms={'p99':16.67},main_work_ms={'max':1},
                                 rss_kib={'after':9000,'rest_after':4000,'rest_delta_from_before':-500}))
        row=summarize([sample],3)[1]
        self.assertEqual(row['rss_rest_kib']['mean'],4000)
        self.assertEqual(row['rss_rest_delta_kib']['mean'],-500)
        del sample['metrics']['rss_kib']['rest_after']
        self.assertIsNone(summarize([sample],3)[1]['rss_rest_kib']['mean'])

    def test_smoother_variant_showing_fewer_frames_loses_count_check(self):
        samples=[]
        for trial in range(3):
            for role,late,shown in [('base',5,1000),('variant',1,970)]:
                samples.append(dict(arm='asap',role=role,trial=trial,valid=True,status='fail',condition={},
                                    metrics=dict(late_pct=late,shown_frames_tape=shown,
                                                 interval_ms={'p99':16.67},main_work_ms={'max':1})))
        result=summarize(samples,3)[1]
        self.assertTrue(result['shown_count_regression'])
        self.assertEqual(result['paired_shown_delta']['mean'],-30)
        self.assertEqual(result['paired_late_delta_pp']['mean'],-4)

if __name__=='__main__':unittest.main()
