import sys, numpy as np
dx = np.load(sys.argv[1]); dy = np.load(sys.argv[1].replace('dx','dy')); mask = np.load(sys.argv[1].replace('dx','mask'))
n = dx.shape[1]
# Per pixel over time: the median absolute displacement (embers pass in a few frames; the haze is always there).
mx = np.median(np.abs(dx), 0); my = np.median(np.abs(dy), 0)
print("above line (pt) | median |dx| (pt) | median |dy| (pt)")
out = []
for lo in range(0, 300, 10):
    r0, r1 = n - (lo + 10) * 2, n - lo * 2
    m = mask[r0:r1]
    if m.sum() < 200: continue
    a, b = np.median(mx[r0:r1][m]) / 2, np.median(my[r0:r1][m]) / 2
    out.append((lo + 5, a, b)); print(f"{lo:4d}-{lo+10:<4d} | {a:6.3f} | {b:6.3f}")
np.save(sys.argv[1].replace('dx','profile'), np.array(out))
