"""Fits the dropped card's motion to the tracks track_card.py and
track_edge.py print: free fall from release, then an exponential approach
to a float level, then a slow sink.

    python3 scripts/lava/track_card.py VIDEO 4.2 5.12 4.2 | tail -n +2 > title.txt
    python3 scripts/lava/track_edge.py VIDEO 5.0 5.7 4.2 | tail -n +2 > edge.txt
    python3 scripts/lava/fit.py title.txt edge.txt
"""
import sys
import numpy as np
from scipy.optimize import curve_fit

T = np.loadtxt(sys.argv[1], usecols=(0, 1)); t, d = T[:, 0], T[:, 1]
top = 444.5 + d   # the card's top, pt, from its title's track
LINE = 564.5      # the line, pt
m = (t >= 4.28) & (top + 66 <= LINE + 2)
f = lambda t, tr, g: 444.5 + 0.5 * g * np.clip(t - tr, 0, None) ** 2
(tr, g), _ = curve_fit(f, t[m], top[m], p0=(4.3, 2000))
print(f"fall: released {tr:.3f} s, g {g:.0f} pt/s^2, rms {np.sqrt(np.mean((f(t[m], tr, g) - top[m]) ** 2)):.2f} pt over {m.sum()} frames")
ti = tr + np.sqrt(2 * (LINE - 66 - 444.5) / g)
print(f"meets the surface at {ti:.3f} s, {g * (ti - tr):.0f} pt/s")
m2 = (t > ti + 0.02) & (t <= 5.1)
h = lambda t, yeq, A, tau: yeq - A * np.exp(-(t - ti) / tau)
(yeq, A, tau), _ = curve_fit(h, t[m2], top[m2], p0=(538, 30, 0.1))
print(f"float: top {LINE - yeq:.1f} pt above the line, tau {tau * 1000:.0f} ms")
E = np.loadtxt(sys.argv[2], usecols=(0, 2))
ok = (E[:, 0] > 5.12) & (E[:, 0] < 5.5)
print(f"then sinks at {np.polyfit(E[ok, 0], E[ok, 1], 1)[0]:.0f} pt/s (edge track)")
