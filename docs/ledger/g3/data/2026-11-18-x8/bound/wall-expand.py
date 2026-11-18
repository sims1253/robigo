import subprocess, time, os, sys, json, statistics
B = os.path.expanduser(os.environ.get("ROBIGO_RT", "~/.cache/robigo-research/ar-perf/robigo-rt-main"))
D = os.path.dirname(os.path.abspath(__file__))
progs = ["growdbl@1.05", "growdbl@2", "listgrow@1.05", "listgrow@2", "growdbl-pre@1.05", "listgrow-pre@1.05"]
modes = {"inline": {}, "noinline": {"ROBIGO_NO_INLINE": "1"}}
base = dict(os.environ, R_DEFAULT_PACKAGES="NULL", LC_ALL="C.UTF-8")
cells = [(p, m) for p in progs for m in modes]
samples = {f"{p}/{m}": [] for p, m in cells}
loads = []
R = int(sys.argv[1]) if len(sys.argv) > 1 else 11
for r in range(R):
    loads.append(os.getloadavg()[0])
    order = cells[r % len(cells):] + cells[:r % len(cells)]
    for p, m in order:
        env = dict(base, **modes[m])
        t = time.perf_counter()
        subprocess.run([B, "--vanilla", os.path.join(D, p.split("@")[0] + ".R")], env=dict(env, R_EXPAND_FRAC=p.split("@")[1]), stdout=subprocess.DEVNULL, check=True)
        samples[f"{p}/{m}"].append(time.perf_counter() - t)
loads.append(os.getloadavg()[0])
med = {k: statistics.median(v) for k, v in samples.items()}
json.dump({"samples": samples, "medians": med, "loads": loads}, open(os.path.join(D, "wall-expand.json"), "w"), indent=1)
for k in sorted(med): print(f"{k:22s} median {med[k]:.3f} s  (min {min(samples[k]):.3f} max {max(samples[k]):.3f})")
print("load", loads)
