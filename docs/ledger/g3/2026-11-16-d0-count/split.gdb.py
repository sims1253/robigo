# D0c's builtin trace: gdb -batch -x split.gdb.py --args R ...
#
# $D0C_FUNS names a file of lines `CFUN NROWS`: every C function of names.c's
# R_FunTab with the number of rows it serves. A breakpoint at each one's entry
# address records the row called: for a function with one row, its first
# call (then the breakpoint is deleted); for a function shared by NROWS rows,
# the row is read from its second argument, `op` (%rsi under the SysV ABI,
# valid at the first instruction), as R_FunTab[op->u.primsxp.offset].name,
# PRIMNAME, and the breakpoint is deleted once all NROWS rows have been seen.
#
# A breakpoint at run_Rmainloop splits the run in two phases: `startup`
# (setup_Rmainloop: the base package, from source or from base.rdb, the
# profiles, the JIT's compiler, .First.sys) and `script` (the expressions R
# reads). $D0C_SPLIT_OUT receives one line per (function, row):
#   SEQ PHASE CFUN NAME HITS
# SEQ is the order of first calls in the run; HITS counts the calls seen
# (exact for a shared function until its breakpoint goes; 1 otherwise).
import gdb
import os

nrows = {}
for line in open(os.environ["D0C_FUNS"]):
    f, n = line.split()
    nrows[f] = int(n)
out = os.environ["D0C_SPLIT_OUT"]
rows = {}  # (fn, name) -> [seq, phase, hits]
state = {"seq": 0, "phase": "startup"}


def record(fn, name):
    k = (fn, name)
    if k in rows:
        rows[k][2] += 1
    else:
        rows[k] = [state["seq"], state["phase"], 1]
        state["seq"] += 1


class Entry(gdb.Breakpoint):
    def __init__(self, fn):
        super().__init__("*" + fn, internal=True)
        self.fn = fn
        self.names = set()

    def stop(self):
        if nrows[self.fn] == 1:
            record(self.fn, "")
            self.enabled = False
            return False
        try:
            op = gdb.parse_and_eval("(SEXP)$rsi")
            ty = int(op["sxpinfo"]["type"])
            if ty not in (7, 8):  # SPECIALSXP, BUILTINSXP
                name = "?type%d" % ty
            else:
                off = int(op["u"]["primsxp"]["offset"])
                name = gdb.parse_and_eval("R_FunTab[%d].name" % off).string()
        except gdb.error:
            name = "?error"
        record(self.fn, name)
        if not name.startswith("?"):
            self.names.add(name)
            if len(self.names) >= nrows[self.fn]:
                self.enabled = False
        return False


class Mainloop(gdb.Breakpoint):
    def stop(self):
        state["phase"] = "script"
        self.enabled = False
        return False


def write(event=None):
    with open(out, "w") as f:
        for (fn, name), (seq, phase, hits) in sorted(rows.items(), key=lambda kv: kv[1][0]):
            f.write("%d\t%s\t%s\t%s\t%d\n" % (seq, phase, fn, name, hits))


gdb.execute("set pagination off")
gdb.execute("set confirm off")
gdb.execute("handle SIGPIPE SIGCHLD SIGALRM SIGUSR1 SIGUSR2 SIGWINCH nostop noprint pass")
gdb.events.exited.connect(write)
# libR.so is a DT_NEEDED of bin/exec/R: loaded by main
gdb.execute("tbreak main")
gdb.execute("run")
for fn in nrows:
    try:
        gdb.parse_and_eval("&" + fn)
    except gdb.error:
        continue
    Entry(fn)
Mainloop("*run_Rmainloop", internal=True)
gdb.execute("continue")
