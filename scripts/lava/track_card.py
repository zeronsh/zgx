import sys, cv2, numpy as np
src, t0, t1, tt = sys.argv[1], float(sys.argv[2]), float(sys.argv[3]), float(sys.argv[4])
cap = cv2.VideoCapture(src)
def frame(t):
    cap.set(cv2.CAP_PROP_POS_MSEC, t * 1000); ok, f = cap.read(); return cv2.cvtColor(f, cv2.COLOR_BGR2GRAY)
# Template: the held card's title, at time tt, found as the brightest text row band in the card.
f0 = frame(tt)
# The card's left part, title and checkbox: located by searching for the title text near the line.
band = f0[800:1130, 60:700]
rows = (band > 200).sum(1)
r = int(np.argmax(np.convolve(rows, np.ones(20), 'same')))
y0 = 800 + r - 30
tpl = f0[y0:y0 + 60, 200:640].astype(np.float32)
print(f"template at y={y0}..{y0+60} px")
cap.set(cv2.CAP_PROP_POS_MSEC, t0 * 1000)
t = t0
while t < t1:
    ok, f = cap.read()
    if not ok: break
    t = cap.get(cv2.CAP_PROP_POS_MSEC) / 1000
    g = cv2.cvtColor(f, cv2.COLOR_BGR2GRAY).astype(np.float32)
    area = g[700:1200, 140:700]
    res = cv2.matchTemplate(area, tpl, cv2.TM_CCOEFF_NORMED)
    _, score, _, loc = cv2.minMaxLoc(res)
    print(f"{t:6.3f} {(700 + loc[1] - y0) / 2:8.2f} {(140 + loc[0] - 200) / 2:7.2f} {score:5.2f}")
