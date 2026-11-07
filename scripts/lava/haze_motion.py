import sys, numpy as np
dx = np.load(sys.argv[1]); mask = np.load(sys.argv[1].replace('dx','mask'))
n = dx.shape[1]
r0, r1 = n - 100 * 2, n - 30 * 2       # 30..100 pt above the line
D = dx[:, r0:r1] - np.median(dx[:, r0:r1], 0)  # what moves, not what's static
k = 6                                     # frames apart (0.1 s)
best = []
for s in range(-40, 41, 2):               # rows (device px), negative: up
    a = D[:-k, max(0, s):D.shape[1] + min(0, s)]
    b = D[k:, max(0, -s):D.shape[1] - max(0, s)]
    best.append((np.corrcoef(a.ravel(), b.ravel())[0, 1], s))
c, s = max(best)
print(f"best shift {s} px over {k} frames: rise {-s/2/(k/60):.1f} pt/s (corr {c:.2f})")
# Horizontal scale: where the autocorrelation along x first falls to half.
row = D.mean(0)
ac = [np.corrcoef(D[:, :, :-d].ravel(), D[:, :, d:].ravel())[0, 1] for d in range(1, 120)]
half = next(d for d, v in enumerate(ac, 1) if v < 0.5)
print(f"x autocorrelation halves at {half/2:.1f} pt; first zero at {next((d for d,v in enumerate(ac,1) if v<0), None)} px")
acy = [np.corrcoef(D[:, :-d].ravel(), D[:, d:].ravel())[0, 1] for d in range(1, 60)]
print(f"y autocorrelation halves at {next(d for d,v in enumerate(acy,1) if v<0.5)/2:.1f} pt")
act = [np.corrcoef(D[:-d].ravel(), D[d:].ravel())[0, 1] for d in range(1, 30)]
print(f"time autocorrelation halves at {next((d for d,v in enumerate(act,1) if v<0.5), None)} frames")
