import sys, cv2, numpy as np
src, t0, t1 = sys.argv[1], float(sys.argv[2]), float(sys.argv[3])
cap = cv2.VideoCapture(src); cap.set(cv2.CAP_PROP_POS_MSEC, t0 * 1000)
acs = []; widths = []
while cap.get(cv2.CAP_PROP_POS_MSEC) < t1 * 1000:
    ok, f = cap.read()
    if not ok: break
    hsv = cv2.cvtColor(f[1129 - 60:1129 - 20, 260:1560], cv2.COLOR_BGR2HSV)   # 10-30 pt above the line, inside the card's span
    m = ((hsv[..., 2] > 170) & (hsv[..., 1] > 90) & (hsv[..., 0] < 35)).mean(0)
    v = m - m.mean()
    if v.std() < 1e-3: continue
    ac = np.correlate(v, v, 'full')[len(v) - 1:]; ac /= ac[0]; acs.append(ac[:300])
    # Tongue widths: runs of columns more than half lit.
    on = m > 0.5; runs = np.diff(np.flatnonzero(np.diff(np.r_[0, on.astype(int), 0])))[::2]
    widths += list(runs)
ac = np.mean(acs, 0)
first_min = int(np.argmin(ac[:200])); peak = first_min + int(np.argmax(ac[first_min:300]))
print(f"{len(acs)} frames: tongue spacing (autocorrelation peak) {peak/2:.1f} pt, half-width at {np.argmax(ac < 0.5)/2:.1f} pt; median tongue width {np.median(widths)/2 if widths else 0:.1f} pt; tongues per 100 pt {len(widths)/len(acs)/ (1300/2) * 100:.2f}")
