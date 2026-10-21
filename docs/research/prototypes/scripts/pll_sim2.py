import numpy as np
def sim(ppm=100.0, jitter_ms=1.5, outlier_p=0.01, outlier_ms=8.0, seconds=1800, packet=480,
        alpha=0.02, tau_s=20.0, clamp_ppm=1000.0, init_err_ms=10.0, seed=1):
    rng = np.random.default_rng(seed); fs=48000.0
    f_true = fs*(1+ppm*1e-6); n = int(seconds*f_true/packet); dt = packet/fs
    kp = 2.0/(tau_s*fs); ki = kp/tau_s
    pred = -init_err_ms*fs/1000.0; integ=0.0; elp=0.0
    true_err=np.empty(n); ratios=np.empty(n)
    for k in range(n):
        true_pos = k*packet/f_true*fs
        ts = true_pos + rng.uniform(-jitter_ms, jitter_ms)*fs/1000.0
        if rng.random() < outlier_p: ts += outlier_ms*fs/1000.0
        e = ts - pred
        e_c = max(-96.0, min(96.0, e - elp)) + elp
        elp += alpha*(e_c - elp)
        integ += elp*dt
        ratio = min(1+clamp_ppm*1e-6, max(1-clamp_ppm*1e-6, 1.0 + kp*elp + ki*integ))
        true_err[k] = (pred - true_pos)*1000/fs   # ms: where we place it minus where it belongs
        pred += packet*ratio
        ratios[k]=ratio
    t = np.arange(n)*dt
    ok = np.abs(true_err) < 0.5
    # first time after which it stays within 0.5 ms for the rest of the run
    bad = np.where(~ok)[0]
    settle = t[bad[-1]+1] if len(bad) else 0.0
    half = n//2
    return dict(settle_s=round(float(settle),1), p99_true_err_ms=round(float(np.percentile(np.abs(true_err[half:]),99)),3),
                ratio_ppm_std=round(float(np.std((ratios[half:]-1)*1e6)),2), ratio_ppm_mean=round(float(np.mean((ratios[half:]-1)*1e6)),2))
for tau in (5.0,10.0,20.0,40.0):
    print('tau',tau, sim(tau_s=tau))
print('tau 20, jitter 3ms', sim(tau_s=20.0, jitter_ms=3.0))
print('tau 20, 200ppm, init 40ms', sim(tau_s=20.0, ppm=200.0, init_err_ms=40.0))
