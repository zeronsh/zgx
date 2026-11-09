#!/usr/bin/env python3
"""Export only reviewed conditions and numeric metrics from private scroll captures."""
import argparse
import json
import math
from pathlib import Path

FIELDS = {
    'scene': {'transcript', 'tables', 'plain', 'grid', 'nested', 'idle', 'zeron', 'zeron-idle'},
    'gesture': {'slow', 'flick', 'reversals', 'notches', 'fling', 'wing', 'stream', 'reference'},
    'size': {'S', 'M', 'L', 'W'},
    'background': {'opaque', 'translucent'},
    'present': {'0', '1', 'auto'},
    'status': {'pass', 'fail', 'invalid'},
    'model_catalog': {'settled', 'loading'},
}
SCALARS = ('hz', 'scale', 'repeat')
METRICS = (
    'frames', 'scroll_frames', 'active_intervals', 'late_frames', 'late_pct',
    'app_late_frames', 'app_late_pct', 'system_late_frames', 'unattributed_late_frames',
    'shown_frames_tape', 'shown_frames_motion', 'same_refresh_extra_frames',
    'double_then_skip_intervals', 'skipped_drawables', 'main_over_4ms', 'gpu_p99_ms',
    'copied_frames', 'repainted_frames', 'picture_frames', 'copy_pct', 'repaint_pct',
    'rows_built_in_scroll_frames', 'rows_prefetched', 'spare_frames',
)
SPREADS = ('interval_ms', 'main_work_ms', 'main_with_present_ms', 'present_ms',
           'wait_ms', 'instructions', 'start_to_present_ms',
           'render_start_to_shown_ms_estimate', 'frame_end_to_shown_ms_estimate')
QUANTILES = ('p50', 'p99', 'max', 'min')


def number(value):
    return value if type(value) in (int, float) and math.isfinite(value) else None


def numeric_fields(source, keys):
    return {key: number(source.get(key)) for key in keys if key in source}


def timing_summary(timing):
    return {key: numeric_fields(timing[key], keys) for key, keys in (
        ('acquisition_ms', QUANTILES), ('gpu_wait_ms', QUANTILES),
        ('present_to_display_ms', QUANTILES), ('handover_end_to_display_ms', QUANTILES),
        ('coverage', ('shown_records', 'handover_records', 'missing_handover', 'clock_mismatch')),
    ) if isinstance(timing.get(key), dict)}


def sanitize(row):
    result = {key: row[key] for key, choices in FIELDS.items()
              if isinstance(row.get(key), str) and row[key] in choices}
    result.update(numeric_fields(row, SCALARS))
    for key in ('catalog_complete', 'catalog_complete_after'):
        if type(row.get(key)) is bool:
            result[key] = row[key]
    for key in ('engine_pin', 'engine_commit', 'tape_sha256'):
        value = row.get(key)
        if isinstance(value, str) and len(value) in (40, 64) and all(c in '0123456789abcdef' for c in value):
            result[key] = value
    source = row.get('metrics', {})
    metrics = numeric_fields(source, METRICS)
    for key in SPREADS:
        if isinstance(source.get(key), dict):
            metrics[key] = numeric_fields(source[key], QUANTILES)
    if isinstance(source.get('rss_kib'), dict):
        metrics['rss_kib'] = numeric_fields(source['rss_kib'], ('before', 'after', 'delta', 'peak', 'rest_after', 'rest_seconds', 'rest_delta_from_before'))
    timing = source.get('presentation_timings', {})
    if isinstance(timing, dict):
        metrics['presentation_timings'] = timing_summary(timing)
    if isinstance(source.get('frame_source_counts'), dict):
        metrics['frame_source_counts'] = numeric_fields(source['frame_source_counts'],
            ('view link', 'display link', 'timer on the grid', 'input or other'))
    flags = row.get('host_env')
    if isinstance(flags, dict):
        allowed = {
            'ZGX_VIEW_CLOCK': {'0', '1'}, 'ZGX_SCROLL_LINK': {'0', '1'},
            'ZGX_SCROLL_DRAWABLES': {'0', '1'}, 'ZGX_HANDOVER_ASAP': {'0', '1'},
            'ZGX_LEAD_US': {'4000', '5500'}, 'ZGX_GPU_STATS': {'1'},
            'ZGX_SHADOW_WHOLE': {'1'}, 'ZGX_DRAWABLE_PREFETCH': {'0', '1'},
        }
        result['host_env'] = {key: value for key, value in flags.items()
                              if key in allowed and isinstance(value, str) and value in allowed[key]}
    result['metrics'] = metrics
    for key in ('native_floor_before', 'native_floor_after'):
        floor = row.get(key)
        if not isinstance(floor, dict):
            continue
        clean = numeric_fields(floor, ('late_pct', 'late_frames', 'skipped_drawables', 'samples', 'gpu_p99_ms'))
        if floor.get('status') in ('valid', 'invalid'):
            clean['status'] = floor['status']
        for spread in ('interval_ms', 'main_work_ms', 'display_period_ms'):
            if isinstance(floor.get(spread), dict):
                clean[spread] = numeric_fields(floor[spread], QUANTILES)
        if isinstance(floor.get('presentation_timings'), dict):
            clean['presentation_timings'] = timing_summary(floor['presentation_timings'])
        result[key] = clean
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('inputs', nargs='+', type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    rows = []
    for path in args.inputs:
        if path.is_file():
            rows.extend(sanitize(row) for row in json.loads(path.read_text()))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(rows, indent=2, allow_nan=False) + '\n')
    print(f'Exported {len(rows)} numeric scroll summaries; raw captures remain private.')


if __name__ == '__main__':
    main()
