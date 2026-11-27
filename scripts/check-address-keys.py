#!/usr/bin/env python3
"""No unjustified address-keyed map or set in the G3 runtime crates.

    scripts/check-address-keys.py            # check the crates, exit 1 on a hit
    scripts/check-address-keys.py FILE...    # check these files

A map or set keyed by a raw `Sexp` or a `usize` taken from an object address
(`.addr()`, `as_ptr() as usize`) silently corrupts under the moving collector if
it outlives a collection. Maps scoped to a `&Cx` interval must use `Ref<'_>`
keys instead (`Ref` derives `Hash` and `Eq` by pointer address, and the borrow
checker forbids holding one across `&mut Cx`).

Any map or set (`HashMap`, `HashSet`, `BTreeMap`, `BTreeSet`, `PosMap`) that
still keys by raw `Sexp`, or by `usize` populated/queried from `.addr()` or
`as_ptr() as usize`, must carry a one-line justification comment at its
declaration:

    // address-key: <why no moving collection can invalidate this map>

`robigo-heap` is excluded: it is the collector itself.
"""

import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CRATES = ["robigo-rtcore", "robigo-rtbuiltins", "robigo-rt", "robigo-rtapi"]

MAP_NAMES = r"(?:HashMap|HashSet|BTreeMap|BTreeSet|PosMap)"
MAP_TYPE = re.compile(rf"\b{MAP_NAMES}\s*<")
ADDR_EXPR = re.compile(r"\.addr\s*\(\)|\bas_ptr\s*\(\)\s*as\s+usize\b")
JUSTIFY = re.compile(r"//\s*address-key:\s*\S")

LET_DECL = re.compile(
    rf"\blet\s+(?:mut\s+)?(?P<name>[A-Za-z_]\w*)"
    rf"(?:\s*:\s*(?P<ty>[^=;]+))?\s*=\s*(?P<rhs>[^;]+);"
)
FIELD_OR_PARAM = re.compile(
    rf"(?:^|[,\(\{{\n])\s*(?:pub(?:\([^)]*\))?\s+)?(?P<name>[A-Za-z_]\w*)\s*:\s*"
    rf"(?P<ty>(?:&(?:'\w+\s+)?(?:mut\s+)?)?(?:[\w:]+::)?{MAP_NAMES}\s*<)"
)
MAP_METHOD = re.compile(
    r"\b(?P<name>[A-Za-z_]\w*)\s*\.\s*(?:insert|get|get_mut|contains|contains_key|entry|remove|extend)\s*\("
)
ADDR_ALIAS = re.compile(
    r"\blet\s+(?:mut\s+)?(?P<name>[A-Za-z_]\w*)(?:\s*:\s*[^=;]+)?\s*=\s*(?P<rhs>[^;]+);"
)


def strip_comments_and_strings(src):
    """Return `src` with comments and string/byte literals blanked (newlines kept)."""
    out = list(src)
    n = len(src)
    i = 0
    while i < n:
        c = src[i]
        # line comment
        if c == "/" and i + 1 < n and src[i + 1] == "/":
            j = i
            while j < n and src[j] != "\n":
                out[j] = " "
                j += 1
            i = j
            continue
        # block comment (nested)
        if c == "/" and i + 1 < n and src[i + 1] == "*":
            depth = 1
            out[i] = " "
            out[i + 1] = " "
            j = i + 2
            while j < n and depth > 0:
                if src[j] == "/" and j + 1 < n and src[j + 1] == "*":
                    depth += 1
                    out[j] = " "
                    out[j + 1] = " "
                    j += 2
                elif src[j] == "*" and j + 1 < n and src[j + 1] == "/":
                    depth -= 1
                    out[j] = " "
                    out[j + 1] = " "
                    j += 2
                else:
                    if src[j] != "\n":
                        out[j] = " "
                    j += 1
            i = j
            continue
        # raw string literal r#"..."# or br#"..."#
        if c in ("r", "b"):
            k = i + (2 if c == "b" and i + 1 < n and src[i + 1] == "r" else 1 if c == "r" else 0)
            if k > i:
                hashes = 0
                while k < n and src[k] == "#":
                    hashes += 1
                    k += 1
                if k < n and src[k] == '"':
                    end_seq = '"' + "#" * hashes
                    for p in range(i, k + 1):
                        out[p] = " "
                    j = k + 1
                    while j < n:
                        if src.startswith(end_seq, j):
                            for p in range(j, j + len(end_seq)):
                                out[p] = " "
                            j += len(end_seq)
                            break
                        if src[j] != "\n":
                            out[j] = " "
                        j += 1
                    i = j
                    continue
        # regular string literal "..."
        if c == '"':
            out[i] = " "
            j = i + 1
            while j < n:
                if src[j] == "\\" and j + 1 < n:
                    if src[j] != "\n":
                        out[j] = " "
                    if src[j + 1] != "\n":
                        out[j + 1] = " "
                    j += 2
                elif src[j] == '"':
                    out[j] = " "
                    j += 1
                    break
                else:
                    if src[j] != "\n":
                        out[j] = " "
                    j += 1
            i = j
            continue
        i += 1
    return "".join(out)


def first_generic_arg(src, lt_pos):
    """Return the first balanced generic argument starting just after `<` at `lt_pos`."""
    i = lt_pos + 1
    n = len(src)
    angle = 0
    paren = 0
    bracket = 0
    while i < n:
        c = src[i]
        if c == "<":
            angle += 1
        elif c == ">":
            if angle == 0 and paren == 0 and bracket == 0:
                return src[lt_pos + 1 : i]
            if i == 0 or src[i - 1] != "-":
                angle -= 1
        elif c == "(":
            paren += 1
        elif c == ")":
            paren -= 1
        elif c == "[":
            bracket += 1
        elif c == "]":
            bracket -= 1
        elif c == "," and angle == 0 and paren == 0 and bracket == 0:
            return src[lt_pos + 1 : i]
        i += 1
    return src[lt_pos + 1 :]


def first_call_arg(src, lparen_pos):
    """Return the first balanced call argument starting just after `(` at `lparen_pos`."""
    i = lparen_pos + 1
    n = len(src)
    paren = 0
    bracket = 0
    brace = 0
    while i < n:
        c = src[i]
        if c == "(":
            paren += 1
        elif c == ")":
            if paren == 0 and bracket == 0 and brace == 0:
                return src[lparen_pos + 1 : i]
            paren -= 1
        elif c == "[":
            bracket += 1
        elif c == "]":
            bracket -= 1
        elif c == "{":
            brace += 1
        elif c == "}":
            brace -= 1
        elif c == "," and paren == 0 and bracket == 0 and brace == 0:
            return src[lparen_pos + 1 : i]
        i += 1
    return src[lparen_pos + 1 :]


def has_justification(raw_lines, line_no):
    """True if `line_no` (1-based) or the contiguous comment/attribute block
    immediately preceding it contains `// address-key: <reason>`."""
    idx = line_no - 1
    if 0 <= idx < len(raw_lines) and JUSTIFY.search(raw_lines[idx]):
        return True
    i = idx - 1
    while i >= 0:
        s = raw_lines[i].strip()
        if not s:
            break
        if s.startswith("//"):
            if JUSTIFY.search(raw_lines[i]):
                return True
            i -= 1
            continue
        if s.startswith("#["):
            i -= 1
            continue
        break
    return False


def brace_spans(code):
    """Map each `{` offset to its matching `}` offset."""
    stack = []
    spans = {}
    for i, c in enumerate(code):
        if c == "{":
            stack.append(i)
        elif c == "}" and stack:
            spans[stack.pop()] = i
    return spans


def enclosing_block(pos, spans):
    """Return `(start, end)` of the innermost `{...}` block containing `pos`, or `(0, -1)`."""
    best = None
    for s, e in spans.items():
        if s < pos < e:
            if best is None or s > best[0]:
                best = (s, e)
    return best if best is not None else (0, 10**12)


def hits(path, raw_src):
    raw_lines = raw_src.splitlines()
    code = strip_comments_and_strings(raw_src)
    spans = brace_spans(code)
    reported = set()

    def line_of(offset):
        return code.count("\n", 0, offset) + 1

    def report(line_no, msg):
        key = (line_no, msg)
        if key not in reported:
            reported.add(key)
            return f"{path}:{line_no}: {msg}"
        return None

    # 1. Raw `Sexp` as key type in any map/set type.
    for m in MAP_TYPE.finditer(code):
        key_ty = first_generic_arg(code, m.end() - 1)
        if re.search(r"\bSexp\b", key_ty):
            ln = line_of(m.start())
            if not has_justification(raw_lines, ln):
                r = report(ln, f"raw Sexp key in {m.group(0).strip()}{key_ty.strip()}> without `// address-key:`")
                if r:
                    yield r

    # Collect local variables bound to `.addr()` / `as_ptr() as usize` per block.
    addr_aliases = []  # (block, name)
    for m in ADDR_ALIAS.finditer(code):
        if ADDR_EXPR.search(m.group("rhs")):
            addr_aliases.append((enclosing_block(m.start(), spans), m.group("name")))

    def expr_is_addr(expr, pos):
        if ADDR_EXPR.search(expr):
            return True
        blk_s, blk_e = enclosing_block(pos, spans)
        for (as_, ae), aname in addr_aliases:
            if as_ <= blk_s and blk_e <= ae and re.search(rf"\b{re.escape(aname)}\b", expr):
                return True
        return False

    # Collect all map/set lookup/mutation calls whose first argument is an address.
    addr_used_names = []  # (pos, name)
    for m in MAP_METHOD.finditer(code):
        arg = first_call_arg(code, m.end() - 1)
        if expr_is_addr(arg, m.start()):
            addr_used_names.append((m.start(), m.group("name")))

    # 2a. `let` declarations of maps/sets (by type annotation or RHS constructor).
    map_ctor = re.compile(rf"\b{MAP_NAMES}\b")
    for m in LET_DECL.finditer(code):
        name = m.group("name")
        ty = m.group("ty") or ""
        rhs = m.group("rhs")
        is_map = bool(map_ctor.search(ty) or map_ctor.search(rhs))
        if not is_map:
            continue
        ln = line_of(m.start())
        blk_s, blk_e = enclosing_block(m.start(), spans)
        used_with_addr = ADDR_EXPR.search(rhs) is not None or any(
            uname == name and m.end() <= upos < blk_e for upos, uname in addr_used_names
        )
        if used_with_addr and not has_justification(raw_lines, ln):
            r = report(ln, f"address-keyed map/set `{name}` without `// address-key:`")
            if r:
                yield r

    # 2b. Struct fields and function parameters typed with `usize` keys that are
    # queried/populated from an address anywhere in the file.
    file_addr_names = {uname for _, uname in addr_used_names}
    for m in FIELD_OR_PARAM.finditer(code):
        name = m.group("name")
        key_ty = first_generic_arg(code, m.end() - 1)
        if not re.search(r"\busize\b", key_ty):
            continue
        if name not in file_addr_names:
            continue
        ln = line_of(m.start("name"))
        if not has_justification(raw_lines, ln):
            r = report(ln, f"address-keyed map/set `{name}` without `// address-key:`")
            if r:
                yield r


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
            f"{len(found)} unjustified address-keyed map/set(s): use `Ref<'_>` keys "
            "for `&Cx`-scoped maps, or add `// address-key: <reason>` at the declaration "
            "(scripts/check-address-keys.py)",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
