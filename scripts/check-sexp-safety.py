#!/usr/bin/env python3
"""No safe public function of the G3 runtime crates takes a `Sexp` (#736).

    scripts/check-sexp-safety.py            # check the crates, exit 1 on a hit
    scripts/check-sexp-safety.py FILE...    # check these files

A `Sexp` is a raw object pointer that safe code can name but not vouch for:
its field is private and `Sexp::from_ptr` is unsafe, so one reaching a safe
public function came from somewhere this crate cannot check. Every reader
through it is `unsafe` (robigo-heap's rule), so a safe `pub fn` that takes
one either dereferences it, which is unsound (docs/architecture.md: "a safe
function must be safe to call from any safe code"), or is a hazard for the
next edit that does. Such a function is an `unsafe fn` with a `# Safety`
section, or takes a `Ref<'_>`/`Root<'_>`. C exports are
`pub unsafe extern "C" fn` (C sees the same symbol).

A parameter counts when its type names `Sexp` outside a closure type (`impl
Fn(Sexp)`, `&mut dyn FnMut(Sexp)`, a bound `F: FnMut(Sexp)`): a callback is
handed objects by the callee, which vouches for them. Fully public
functions are checked (`pub fn`, `pub extern "C" fn`, a macro's `fn $name`,
and the methods of a `pub trait`, public without `pub`): a crate-internal
one is the crate's own to keep sound. `ALLOWED` names the trait methods
that take one by design, each with its reason. The check is textual: a
`Sexp` inside another public type passed by value is outside its view.
"""

import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CRATES = ["robigo-heap", "robigo-rtcore", "robigo-rt", "robigo-rtapi", "robigo-rtbuiltins"]

FN = re.compile(r'\bpub\s+(?P<qual>(?:(?:const|async|unsafe|extern(?:\s+"C")?)\s+)*)fn\s+(?P<name>[\w$]+)')
CLOSURE = re.compile(r"\bFn(?:Mut|Once)?\s*\(")


def uncomment(src):
    """`src` with every `//` comment blanked (line numbers kept)."""
    return re.sub(r"//[^\n]*", lambda m: " " * len(m.group(0)), src)


def params(src, start):
    """What a signature after `start` says of its inputs: the generic
    parameters and their bounds, the parameter list, and the `where`
    clause (the return type is not an input)."""
    i = start
    depth = 0
    while src[i] != "(" or depth:
        if src[i] == "<":
            depth += 1
        elif src[i] == ">" and src[i - 1] != "-":
            depth -= 1
        i += 1
    generics = src[start:i]
    depth = 0
    j = i
    while True:
        c = src[j]
        if c == "(":
            depth += 1
        elif c == ")":
            depth -= 1
            if depth == 0:
                break
        j += 1
    k = j + 1
    while k < len(src) and src[k] not in "{;":
        k += 1
    tail = src[j + 1 : k]
    where = tail[tail.index("where") :] if re.search(r"\bwhere\b", tail) else ""
    return generics + " " + src[i + 1 : j] + " " + where


def without_closures(text):
    """`text` with every closure type's argument list and return type cut."""
    out = []
    pos = 0
    for m in CLOSURE.finditer(text):
        if m.start() < pos:
            continue
        out.append(text[pos : m.end()])
        depth = 1
        k = m.end()
        while depth and k < len(text):
            depth += {"(": 1, ")": -1}.get(text[k], 0)
            k += 1
        # and its return type: `impl Fn(usize) -> Sexp` is a callback too
        ret = re.match(r"\s*->\s*[^,)>]+", text[k:])
        if ret:
            k += ret.end()
        pos = k
    out.append(text[pos:])
    return "".join(out)


TRAIT = re.compile(r"\bpub\s+(?:unsafe\s+)?trait\s+(?P<name>\w+)[^{;]*\{")
METHOD = re.compile(r"(?P<qual>(?:(?:const|async|unsafe|extern(?:\s+\"C\")?)\s+)*)fn\s+(?P<name>\w+)")

# Safe trait methods that take a Sexp by design, with the reason: the
# obligation sits where the objects come from, not on the method.
ALLOWED = {
    "Visitor::edge": "a collection's caller vouches for every edge its roots show (Core::collect's contract)",
}


def body(src, start):
    """The text of the braced block whose `{` is just before `start`."""
    depth = 1
    k = start
    while depth and k < len(src):
        depth += {"{": 1, "}": -1}.get(src[k], 0)
        k += 1
    return src[start : k - 1]


def takes_sexp(src, end):
    return re.search(r"\bSexp\b", without_closures(params(src, end)))


def hits(path, src):
    src = uncomment(src)
    for m in FN.finditer(src):
        if "unsafe" in m.group("qual"):
            continue
        if takes_sexp(src, m.end()):
            yield f"{path}:{src.count(chr(10), 0, m.start()) + 1}: safe pub fn {m.group('name')} takes a Sexp"
    # a `pub` trait's methods are public without `pub`
    for t in TRAIT.finditer(src):
        b = body(src, t.end())
        for m in METHOD.finditer(b):
            if "unsafe" in m.group("qual") or f"{t.group('name')}::{m.group('name')}" in ALLOWED:
                continue
            if takes_sexp(b, m.end()):
                line = src.count(chr(10), 0, t.end() + m.start()) + 1
                yield f"{path}:{line}: safe pub fn {t.group('name')}::{m.group('name')} takes a Sexp"


def files():
    for c in CRATES:
        for dirpath, _, names in os.walk(os.path.join(ROOT, "crates", c, "src")):
            for n in sorted(names):
                if n.endswith(".rs"):
                    yield os.path.join(dirpath, n)


def main(argv):
    paths = argv[1:] or list(files())
    found = []
    for p in paths:
        with open(p) as f:
            found += list(hits(os.path.relpath(p, ROOT), f.read()))
    for h in found:
        print(h)
    if found:
        print(
            f"{len(found)} safe public function(s) take a Sexp: make each an `unsafe fn` with a "
            "`# Safety` section, or take a Ref/Root (scripts/check-sexp-safety.py, #736)",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
