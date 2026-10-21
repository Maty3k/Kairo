# Simulate Kairo's per-source drift loop (PI on timestamp error) to pick gains.
import numpy as np, sys
def sim(ppm=100.0, jitter_ms=1.5, outlier_p=0.01, outlier_ms=8.0, hours=1.0, packet=480,
        alpha=0.02, kp=None, ki=None, tau_s=20.0, clamp_ppm=1000.0, seed=1):
    rng = np.random.default_rng(seed)
    fs = 48000.0
    f_true = fs*(1+ppm*1e-6)            # source's real rate in QPC seconds
    n_packets = int(hours*3600*f_true/packet)
    # Loop gains: second-order loop, natural period ~tau_s
    # error e in samples; ratio_rel = 1 + kp*e + ki*integ(e*dt)
    if kp is None: kp = 2.0/(tau_s*fs)          # 1/(samples*s) scaled per sample error
    if ki is None: ki = kp/(tau_s)              # integral gain
    pred = 0.0          # predicted timeline position of next packet's first frame (samples)
    ratio = 1.0
    integ = 0.0
    elp = 0.0
    dt = packet/fs
    errs=[]; ratios=[]
    for k in range(n_packets):
        true_t = k*packet/f_true                       # seconds when frame k*packet was captured
        ts = true_t*fs + rng.uniform(-jitter_ms, jitter_ms)*fs/1000.0
        if rng.random() < outlier_p: ts += outlier_ms*fs/1000.0
        e = ts - pred
        # outlier rejection: clip the error fed to the loop
        e_c = max(-96.0, min(96.0, e - elp)) + elp     # limit innovation to +-2 ms
        elp += alpha*(e_c - elp)
        integ += elp*dt
        ratio = 1.0 + kp*elp + ki*integ
        ratio = min(1+clamp_ppm*1e-6, max(1-clamp_ppm*1e-6, ratio))
        pred += packet*ratio
        errs.append(e); ratios.append(ratio)
    errs=np.array(errs); ratios=np.array(ratios)
    last = errs[len(errs)//2:]
    r = (ratios[len(ratios)//2:]-1)*1e6
    # true placement error = pred - true position (no jitter)
    return dict(mean_err_ms=float(np.mean(last))*1000/fs, p99_abs_err_ms=float(np.percentile(np.abs(last),99))*1000/fs,
                ratio_ppm_mean=float(np.mean(r)), ratio_ppm_std=float(np.std(r)),
                settle_s=float(np.argmax(np.abs(np.convolve(errs, np.ones(100)/100, 'same'))[200:] < 0.25*fs/1000)+200)*dt)
for tau in (5.0, 10.0, 20.0, 40.0):
    for ppm in (100.0, -60.0):
        print('tau',tau,'ppm',ppm, sim(ppm=ppm, tau_s=tau, hours=0.5))
