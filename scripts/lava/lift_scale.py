import sys, cv2, numpy as np
src = sys.argv[1]
cap = cv2.VideoCapture(src)
def frame(t):
    cap.set(cv2.CAP_PROP_POS_MSEC, t * 1000); ok, f = cap.read(); return cv2.cvtColor(f, cv2.COLOR_BGR2GRAY).astype(np.float32)
g0 = frame(0.3); tpl = g0[436:524, 128:1672]   # the card, rows of its box (pt 218..262 x2)
for t in [float(a) for a in sys.argv[2:]]:
    g = frame(t); best = (0, 0, 0)
    for s in np.arange(0.98, 1.08, 0.0025):
        tp = cv2.resize(tpl, None, fx=s, fy=s, interpolation=cv2.INTER_LINEAR)
        r = cv2.matchTemplate(g, tp, cv2.TM_CCOEFF_NORMED); _, sc, _, loc = cv2.minMaxLoc(r)
        if sc > best[0]: best = (sc, s, loc)
    print(f"t {t:.2f}: scale {best[1]:.4f} (score {best[0]:.2f})")
