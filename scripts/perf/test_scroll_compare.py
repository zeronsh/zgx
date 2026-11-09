import unittest
from scroll_compare import stats,summarize

class ComparisonEvidence(unittest.TestCase):
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
