#!/usr/bin/env python3
"""Analysis of the estimated-cycles study: do callgrind's estimated cycles
predict wall time better than instructions, across changes and machines?

    study-analyze.py CG.json [--cgvar NAME=CG.json ...] --wall NAME=WALL.json ... [--json OUT]

CG.json: {sha: {on1|on2|off1|off2: {row: [instr, cycles]}}} (inlining on/off).
WALL.json: study-wall.py's output (median_ns, samples per "sha|row").
"""
import argparse
import json
import math
import statistics

CHAIN = ["2671f453", "5bb0eabf", "90f3d400", "9dbf19d9", "a2683854", "4a59f7bc", "bc750fc7", "cc5d091e",
         "c5db31c8", "17ed87e9", "c536597d"]
PRS = {"5bb0eabf": "#718", "90f3d400": "#717", "9dbf19d9": "#719", "a2683854": "#725", "4a59f7bc": "#722",
       "bc750fc7": "#728", "cc5d091e": "#726", "c5db31c8": "#727", "17ed87e9": "#730", "c536597d": "#731",
       "7849ccf9": "#702 (D5)"}
PAIRS = list(zip(CHAIN, CHAIN[1:])) + [("4169b271", "7849ccf9")]
GATE = 0.5  # percent


def med_se_rel(xs):
    """The relative standard error of a median of xs (normal approximation)."""
    if len(xs) < 4:
        return float("nan")
    q = statistics.quantiles(xs, n=4)
    m = statistics.median(xs)
    sigma = (q[2] - q[0]) / 1.349
    return 1.2533 * sigma / math.sqrt(len(xs)) / m


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("cg")
    ap.add_argument("--wall", action="append", required=True, help="MACHINE=FILE (inlining on), MACHINE-off=FILE")
    ap.add_argument("--json")
    a = ap.parse_args()
    cg = json.load(open(a.cg))
    walls = {}
    for w in a.wall:
        name, path = w.split("=", 1)
        walls[name] = json.load(open(path))
    points = []
    for wname, w in walls.items():
        mode = "off" if wname.endswith("-off") else "on"
        machine = wname[:-4] if mode == "off" else wname
        cgv = "off1" if mode == "off" else "on1"
        samples = w["samples"]
        for p, c in PAIRS:
            for row in sorted({k.split("|")[1] for k in samples}):
                kp, kc = f"{p}|{row}", f"{c}|{row}"
                if not samples.get(kp) or not samples.get(kc) or row not in cg[p][cgv] or row not in cg[c][cgv]:
                    continue
                wp, wc = statistics.median(samples[kp]), statistics.median(samples[kc])
                ip, cp = cg[p][cgv][row]
                ic, cc = cg[c][cgv][row]
                noise = 100 * math.hypot(med_se_rel(samples[kp]), med_se_rel(samples[kc]))
                points.append({"machine": machine, "mode": mode, "change": PRS[c], "row": row,
                               "d_instr": 100 * (ic / ip - 1), "d_cyc": 100 * (cc / cp - 1),
                               "d_wall": 100 * (wc / wp - 1), "noise": noise})
    out = {"points": points, "summary": {}}
    for machine in sorted({p["machine"] for p in points}):
        for mode in ("on", "off", "all"):
            pts = [p for p in points if p["machine"] == machine and (mode == "all" or p["mode"] == mode)]
            if not pts:
                continue
            real = [p for p in pts if abs(p["d_wall"]) > max(2 * p["noise"], GATE)]
            s = {"n": len(pts), "n_real": len(real)}
            for pred in ("d_instr", "d_cyc"):
                err = [p["d_wall"] - p[pred] for p in pts]
                s[f"{pred}_rmse"] = math.sqrt(sum(e * e for e in err) / len(err))
                s[f"{pred}_mae"] = sum(abs(e) for e in err) / len(err)
                if real:
                    s[f"{pred}_sign_ok"] = sum((p[pred] > 0) == (p["d_wall"] > 0) for p in real) / len(real)
                    s[f"{pred}_rmse_real"] = math.sqrt(sum((p["d_wall"] - p[pred]) ** 2 for p in real) / len(real))
                xs, ys = [p[pred] for p in pts], [p["d_wall"] for p in pts]
                if len(pts) > 2 and statistics.pstdev(xs) > 0:
                    s[f"{pred}_r"] = statistics.correlation(xs, ys)
                # the gate: flags a change more than GATE% slower; a real
                # regression is one whose wall time rose beyond its noise and GATE
                tp = sum(p[pred] > GATE and p in real and p["d_wall"] > 0 for p in pts)
                fp = sum(p[pred] > GATE and not (p in real and p["d_wall"] > 0) for p in pts)
                fn = sum(p[pred] <= GATE and p in real and p["d_wall"] > 0 for p in pts)
                s[f"{pred}_gate"] = {"tp": tp, "fp": fp, "fn": fn}
            out["summary"][f"{machine}/{mode}"] = s
    # absolute: ns per instruction and per estimated cycle across rows and
    # binaries (coefficient of variation: lower tracks time more closely)
    out["absolute"] = {}
    for wname, w in walls.items():
        mode = "off" if wname.endswith("-off") else "on"
        cgv = "off1" if mode == "off" else "on1"
        per_i, per_c = [], []
        for key, xs in w["samples"].items():
            if not xs:
                continue
            sha, row = key.split("|")
            if row not in cg[sha][cgv]:
                continue
            ns = statistics.median(xs) * 1e9
            i, c = cg[sha][cgv][row]
            per_i.append(ns / i)
            per_c.append(ns / c)
        out["absolute"][wname] = {
            "ns_per_instr_cv": statistics.pstdev(per_i) / statistics.mean(per_i),
            "ns_per_cyc_cv": statistics.pstdev(per_c) / statistics.mean(per_c),
            "ns_per_instr_mean": statistics.mean(per_i), "ns_per_cyc_mean": statistics.mean(per_c)}
    if a.json:
        json.dump(out, open(a.json, "w"), indent=1)
    for k, s in out["summary"].items():
        print(f"== {k}: {s['n']} points, {s['n_real']} with a real wall change")
        for pred in ("d_instr", "d_cyc"):
            g = s[f"{pred}_gate"]
            print(f"  {pred:8} r={s.get(pred + '_r', float('nan')):.3f} rmse={s[pred + '_rmse']:.2f} "
                  f"mae={s[pred + '_mae']:.2f} sign_ok(real)={s.get(pred + '_sign_ok', float('nan')):.2f} "
                  f"rmse(real)={s.get(pred + '_rmse_real', float('nan')):.2f} gate tp/fp/fn={g['tp']}/{g['fp']}/{g['fn']}")
    for k, s in out["absolute"].items():
        print(f"abs {k}: ns/instr cv {s['ns_per_instr_cv']:.3f}, ns/cyc cv {s['ns_per_cyc_cv']:.3f}")


if __name__ == "__main__":
    main()
