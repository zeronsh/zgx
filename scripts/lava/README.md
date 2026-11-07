# Measuring the lava example against a reference video

How `crates/zgx-script/examples/lava` was matched to a reference recording:
each script prints numbers measured from a video (OpenCV, numpy, scipy;
1800×1600 at 60 fps, the line at 564.5 pt), and the same script run on
`lava_capture`'s output says how close the example is.

| Script | Measures |
| --- | --- |
| `track_carry.py VIDEO 241 0.6 4.4` | the carried card's displacement per frame (template on its title) |
| `lift_scale.py VIDEO t…` | the lifted card's scale |
| `track_card.py`, `track_edge.py`, `fit.py` | the release, fall, float and sink, and the fit of each |
| `haze_flow.py VIDEO t0 t1 out_dx.npy`, `haze_profile.py`, `haze_motion.py` | heat haze: optical flow against the median frame, by distance from the line; its rise speed and scales |
| `scene.py VIDEO step out.npy` | flame area, height and span; floor, air and card brightness over time |
| `tongues.py VIDEO t0 t1` | flame tongue spacing and width |
