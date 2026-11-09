import unittest

from scroll_public import sanitize


class PublicSummaryTests(unittest.TestCase):
    def test_private_evidence_and_nested_notes_never_pass_through(self):
        row = dict(scene='zeron', status='fail', failures=['private source'],
                   evidence='/private/scene.png', frames=[{'notes': ['private text']}],
                   metrics={'late_pct': 3.5, 'blockers': {'private text': 42},
                            'worst_main_frame': {'notes': ['private text']},
                            'main_work_ms': {'p99': 2.1, 'notes': 'private text'},
                            'presentation_timings': {'samples': [{'private': 'text'}],
                                                     'coverage': {'clock_mismatch': 0}}})
        result = sanitize(row)
        self.assertEqual(result, dict(scene='zeron', status='fail', metrics={
            'late_pct': 3.5, 'main_work_ms': {'p99': 2.1},
            'presentation_timings': {'coverage': {'clock_mismatch': 0}}}))

    def test_unknown_enums_and_non_numeric_values_are_not_exported(self):
        result = sanitize(dict(scene='private conversation', hz='secret',
                               engine_pin='private path', metrics={'late_pct': float('nan'),
                                                                   'frames': 'secret'}))
        self.assertEqual(result, {'hz': None, 'metrics': {'frames': None, 'late_pct': None,
                                                       'presentation_timings': {}}})


if __name__ == '__main__':
    unittest.main()
