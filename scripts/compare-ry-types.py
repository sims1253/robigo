#!/usr/bin/env python3
"""Compare ry's static types with types rr observed at run time.

Usage:
  ry dump-facts script.R --format json > facts.json
  RR_TYPE_PROFILE=observed.json rr script.R
  scripts/compare-ry-types.py facts.json observed.json

Both describe scope-exit snapshots: the bindings of the top level and of
each function scope when it is left. For every binding the script reports

  agree       ry's mode (and known length) match every observed value
  conflict    ry claims a mode/length that some run contradicted
              (an unsound fact, or a code path ry does not model)
  opportunity ry says unknown/opaque but every run saw one mode
              (a fact a runtime could use if ry could prove it)
  unobserved  ry knows the binding but the scope never ran
"""
import json
import sys


def ry_scopes(facts):
    out = {}
    for f in facts["files"]:
        for sc in f["scopes"]:
            if sc.get("kind") == "top":
                key = "top"
            else:
                key = tuple(sc["span"]["start"]) if "span" in sc else None
                if key is None:
                    continue
            out[key] = {b["name"]: b["type"] for b in sc["bindings"]}
    return out


def rr_scopes(obs):
    out = {}
    for sc in obs["scopes"]:
        key = "top" if sc["kind"] == "top" else tuple(sc["start"])
        out[key] = {b["name"]: b["observed"] for b in sc["bindings"]}
    return out


def describe_ry(t):
    length = t["length"]
    n = length.get("value") if length["kind"] == "known" else length["kind"]
    return f"{t['mode']}[{n}]"


def describe_obs(ts):
    return " | ".join(f"{t['mode']}[{t['length']}]" + (" NA" if t["has_na"] else "") for t in ts)


def main():
    facts = json.load(open(sys.argv[1]))
    obs = json.load(open(sys.argv[2]))
    ry = ry_scopes(facts)
    rr = rr_scopes(obs)
    counts = {"agree": 0, "conflict": 0, "opportunity": 0, "unobserved": 0}
    for key, bindings in sorted(ry.items(), key=lambda kv: str(kv[0])):
        observed = rr.get(key)
        label = "top level" if key == "top" else f"function at {key[0]}:{key[1]}"
        for name, t in sorted(bindings.items()):
            if observed is None or name not in observed:
                counts["unobserved"] += 1
                continue
            o = observed[name]
            modes = {x["mode"] for x in o}
            lengths = {x["length"] for x in o}
            if t["mode"] in ("opaque", "union") or (t["length"]["kind"] != "known" and t["mode"] == "opaque"):
                if len(modes) == 1:
                    counts["opportunity"] += 1
                    print(f"opportunity  {label:28} {name:12} ry: {describe_ry(t):18} rr: {describe_obs(o)}")
                continue
            ok = modes == {t["mode"]}
            if t["length"]["kind"] == "known":
                ok = ok and lengths == {t["length"]["value"]}
            if ok:
                counts["agree"] += 1
            else:
                counts["conflict"] += 1
                print(f"CONFLICT     {label:28} {name:12} ry: {describe_ry(t):18} rr: {describe_obs(o)}")
    print()
    print(", ".join(f"{k}: {v}" for k, v in counts.items()))


if __name__ == "__main__":
    main()
