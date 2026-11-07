import sys, cv2, numpy as np
src, t0, t1, tt = sys.argv[1], float(sys.argv[2]), float(sys.argv[3]), float(sys.argv[4])
cap = cv2.VideoCapture(src)
def frame(t):
    cap.set(cv2.CAP_PROP_POS_MSEC, t * 1000); ok, f = cap.read(); return cv2.cvtColor(f, cv2.COLOR_BGR2GRAY).astype(np.float32)
f0 = frame(tt)
# The card's title row: its brightest text band near the line.
band = f0[800:1130, 60:700]; rows = (band > 200).sum(1)
r = int(np.argmax(np.convolve(rows, np.ones(20), 'same'))); title = 800 + r
# Its top edge: the strongest rise in brightness above the title, over the card's width.
prof = f0[title - 90:title, 200:1500].mean(1); top = title - 90 + int(np.argmax(np.abs(np.diff(prof)))) + 1
print(f"title row {title/2:.1f} pt, card top {top/2:.1f} pt")
tpl = f0[top - 8:top + 12, 120:1680]      # the top border across the card, rounded corners included
cap.set(cv2.CAP_PROP_POS_MSEC, t0 * 1000)
while True:
    ok, f = cap.read()
    if not ok: break
    t = cap.get(cv2.CAP_PROP_POS_MSEC) / 1000
    if t > t1: break
    g = cv2.cvtColor(f, cv2.COLOR_BGR2GRAY).astype(np.float32)
    area = g[top - 40:1160, 100:1700]
    res = cv2.matchTemplate(area, tpl, cv2.TM_CCOEFF_NORMED)
    _, sc, _, loc = cv2.minMaxLoc(res)
    print(f"{t:6.3f} top {(top - 40 + loc[1] + 8) / 2:7.1f} pt  score {sc:4.2f}")
