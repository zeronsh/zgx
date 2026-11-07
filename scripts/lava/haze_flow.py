import sys, cv2, numpy as np
src, t0, t1 = sys.argv[1], float(sys.argv[2]), float(sys.argv[3])
cap = cv2.VideoCapture(src); fps = cap.get(cv2.CAP_PROP_FPS)
cap.set(cv2.CAP_PROP_POS_MSEC, t0 * 1000)
frames = []
while cap.get(cv2.CAP_PROP_POS_MSEC) < t1 * 1000:
    ok, f = cap.read()
    if not ok: break
    frames.append(cv2.cvtColor(f, cv2.COLOR_BGR2GRAY).astype(np.float32))
F = np.stack(frames); H, W = F.shape[1:]
# The line: the brightest row in the lower half.
rows = F.mean(0).mean(1); line = int(np.argmax(rows[H // 2:]) + H // 2)
print(f"{len(frames)} frames at {fps:.0f} fps, {W}x{H}, line at y={line} px")
ref = np.median(F, 0)
band = slice(line - 600, line - 4)
dxs, dys = [], []
for f in F:
    flow = cv2.calcOpticalFlowFarneback(ref[band].astype(np.uint8), f[band].astype(np.uint8), None, 0.5, 3, 9, 3, 5, 1.1, 0)
    dxs.append(flow[..., 0]); dys.append(flow[..., 1])
dx, dy = np.stack(dxs), np.stack(dys)
# Only where there is texture to see (text, card edges): gradient of the median.
g = np.hypot(*np.gradient(ref[band]))
mask = g > np.percentile(g, 90)
print("distance above line (pt) | rms dx (pt) | rms dy (pt) | textured px")
n = dx.shape[1]
for lo in range(0, 300, 20):
    r0, r1 = n - (lo + 20) * 2, n - lo * 2
    m = mask[r0:r1][None].repeat(len(F), 0)
    if m.sum() < 50: continue
    print(f"{lo:4d}-{lo+20:<4d} | {np.sqrt((dx[:, r0:r1][m]**2).mean())/2:6.2f} | {np.sqrt((dy[:, r0:r1][m]**2).mean())/2:6.2f} | {m.sum()//len(F)}")
np.save(sys.argv[4], dx); np.save(sys.argv[4].replace('dx', 'dy'), dy); np.save(sys.argv[4].replace('dx', 'mask'), mask)
