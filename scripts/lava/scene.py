import sys, cv2, numpy as np
src = sys.argv[1]; step = int(sys.argv[2]) if len(sys.argv) > 2 else 6
cap = cv2.VideoCapture(src)
rows = []
i = 0
while True:
    ok, f = cap.read()
    if not ok: break
    i += 1
    if i % step: continue
    t = cap.get(cv2.CAP_PROP_POS_MSEC) / 1000
    g = cv2.cvtColor(f, cv2.COLOR_BGR2GRAY)
    line = 1129
    hsv = cv2.cvtColor(f[line - 400:line - 6], cv2.COLOR_BGR2HSV)
    m = (hsv[..., 2] > 170) & (hsv[..., 1] > 90) & (hsv[..., 0] < 35)
    ys, xs = np.nonzero(m)
    n = int(m.sum())
    p95 = (400 - np.percentile(ys, 5)) / 2 if n > 50 else 0
    x0, x1 = ((np.percentile(xs, 5) / 2, np.percentile(xs, 95) / 2) if n > 50 else (0, 0))
    floor = g[line + 20:line + 200].mean()
    # The air between the lowest card and the line (smoke, haze, light): its mean.
    air = g[line - 60:line - 12, 100:1700].mean()
    # The lowest card's fill, beside its text (the card's right part).
    card = g[960:1020, 1100:1400].mean()
    rows.append((t, n, p95, x0, x1, floor, air, card))
    print(f"{t:5.2f} flame {n:6d} top95 {p95:5.1f}pt x {x0:4.0f}-{x1:4.0f}  floor {floor:5.1f}  air {air:5.1f}  card {card:5.1f}")
np.save(sys.argv[3], np.array(rows))
