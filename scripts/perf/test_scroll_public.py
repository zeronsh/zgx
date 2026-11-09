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

    def test_flags_and_modes_do_not_copy_arbitrary_environment_or_notes(self):
        result = sanitize({'host_env': {'ZGX_VIEW_CLOCK': '0', 'SECRET_TOKEN': 'private'},
                           'metrics': {'frame_source_counts': {'view link': 100, '/private/source': 8}}})
        self.assertEqual(result['host_env'], {'ZGX_VIEW_CLOCK': '0'})
        self.assertEqual(result['metrics']['frame_source_counts'], {'view link': 100})

    def test_prefetch_and_rest_numbers_are_exported_without_private_details(self):
        result = sanitize({'host_env': {'ZGX_DRAWABLE_PREFETCH': '0', 'SECRET': 'private'},
                           'metrics': {'rss_kib': {'rest_after': 1234, 'rest_seconds': 3,
                                                  'rest_delta_from_before': -10, 'path': 'private'}}})
        self.assertEqual(result['host_env'], {'ZGX_DRAWABLE_PREFETCH': '0'})
        self.assertEqual(result['metrics']['rss_kib'],
                         {'rest_after': 1234, 'rest_seconds': 3, 'rest_delta_from_before': -10})

    def test_native_brackets_keep_numbers_and_discard_raw_samples(self):
        result = sanitize({'native_floor_before': {'status': 'valid', 'late_pct': 0,
                           'source': '/private/path', 'presentation_timings': {
                               'samples': [{'private': 'text'}],
                               'present_to_display_ms': {'p50': 12.5}}}})
        self.assertEqual(result['native_floor_before'], {'status': 'valid', 'late_pct': 0,
                         'presentation_timings': {'present_to_display_ms': {'p50': 12.5}}})

    def test_unknown_enums_and_non_numeric_values_are_not_exported(self):
        result = sanitize(dict(scene='private conversation', hz='secret',
                               engine_pin='private path', metrics={'late_pct': float('nan'),
                                                                   'frames': 'secret'}))
        self.assertEqual(result, {'hz': None, 'metrics': {'frames': None, 'late_pct': None,
                                                       'presentation_timings': {}}})


if __name__ == '__main__':
    unittest.main()
