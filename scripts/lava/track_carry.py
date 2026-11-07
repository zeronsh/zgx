import sys, cv2, numpy as np
src, y_pt, t0, t1 = sys.argv[1], float(sys.argv[2]), float(sys.argv[3]), float(sys.argv[4])
cap = cv2.VideoCapture(src)
cap.set(cv2.CAP_PROP_POS_MSEC, 0.3 * 1000); ok, f = cap.read(); g0 = cv2.cvtColor(f, cv2.COLOR_BGR2GRAY).astype(np.float32)
y = int(y_pt * 2); tpl = g0[y - 24:y + 24, 200:640]
cap.set(cv2.CAP_PROP_POS_MSEC, t0 * 1000)
while True:
    ok, f = cap.read()
    if not ok: break
    t = cap.get(cv2.CAP_PROP_POS_MSEC) / 1000
    if t > t1: break
    g = cv2.cvtColor(f, cv2.COLOR_BGR2GRAY).astype(np.float32)
    res = cv2.matchTemplate(g[:, 100:760], tpl, cv2.TM_CCOEFF_NORMED); _, sc, _, loc = cv2.minMaxLoc(res)
    print(f"{t:6.3f} {(loc[1] + 24) / 2 - y_pt:7.1f} {sc:4.2f}")
