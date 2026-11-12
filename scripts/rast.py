"""GNU R's own parse tree of an R script, as spans of its source text.

    tree = Parser(rscript).parse(text)       # None when GNU R cannot parse it
    tree.root.kids, node.start, node.end, node.token, src[node.start:node.end]

GNU R's `getParseData()` names every token and expression of a script; this
module places them on the source text, so a tool can delete, replace or
insert code by editing spans while every byte it does not touch (layout,
comments, string escapes) stays exactly as written. Three tools build on it:

- scripts/reduce.py: delta debugging over statements, arguments and
  subexpressions (`groups`, `hoists`).
- scripts/fuzz-eval.py --gen ast: mutants that parse (`mutate`).
- scripts/fuzz-eval.py --emi: code that never runs, inserted and removed
  (`insert_dead`, `strip_dead`).

The parser is one long-lived GNU R process reading file names on stdin:
starting R per query would cost more than the queries.
"""
import os
import random
import re
import subprocess
import tempfile
import threading

HELPER = r"""
con <- file("stdin"); open(con, blocking = TRUE)
hex <- function(t) vapply(t, function(s) paste(as.character(charToRaw(enc2utf8(s))), collapse = ""),
                          "", USE.NAMES = FALSE)
repeat {
  p <- readLines(con, n = 1L)
  if (!length(p)) break
  d <- tryCatch(utils::getParseData(parse(p, keep.source = TRUE, encoding = "UTF-8")),
                error = function(e) e, warning = function(w) w)
  if (inherits(d, "condition")) {
    cat("ERR\n")
  } else {
    if (is.null(d)) d <- data.frame(line1 = integer(), col1 = integer(), id = integer(), parent = integer(),
                                    token = character(), terminal = logical(), text = character())
    writeLines(paste(d$line1, d$col1, d$id, d$parent, d$token, as.integer(d$terminal), hex(d$text), sep = "\t"),
               paste0(p, ".pd"), useBytes = TRUE)
    cat("OK\n")
  }
  flush(stdout())
}
"""

# Tokens of an expression node (gram.y names them; `equal_assign` is
# `x = 1` at top level, `expr_or_assign_or_help` the same inside braces).
EXPR = {"expr", "expr_or_assign_or_help", "equal_assign", "exprlist"}
OPEN = {"'('": "')'", "'['": "']'", "LBB": "']'"}


class Node:
    __slots__ = ("id", "token", "terminal", "text", "start", "end", "parent", "kids")

    def __init__(self, id_, token, terminal, text):
        self.id, self.token, self.terminal, self.text = id_, token, terminal, text
        self.start = self.end = None
        self.parent = None
        self.kids = []

    def is_expr(self):
        return not self.terminal and self.token in EXPR

    def walk(self):
        """Pre-order: a node before its descendants, outermost first."""
        stack = [self]
        while stack:
            n = stack.pop()
            yield n
            stack.extend(reversed(n.kids))

    def __repr__(self):
        return f"<{self.token} {self.start}:{self.end}>"


class Tree:
    def __init__(self, src, root):
        self.src, self.root = src, root

    def text(self, node):
        return self.src[node.start:node.end]

    def terminals(self, *tokens):
        return [n for n in self.root.walk() if n.terminal and (not tokens or n.token in tokens)]


class ParseMismatch(Exception):
    """Parse data that cannot be placed on the source (a bug here)."""


def scan_string(src, pos):
    """End of the string literal at POS (getParseData abbreviates long
    ones as `[N chars quoted with '"']`)."""
    if src[pos] in "rR":
        q = src[pos + 1]
        j = pos + 2
        while src[j] == "-":
            j += 1
        close = {"(": ")", "[": "]", "{": "}"}[src[j]] + "-" * (j - pos - 2) + q
        k = src.index(close, j + 1)
        return k + len(close)
    q, j = src[pos], pos + 1
    while src[j] != q:
        j += 2 if src[j] == "\\" else 1
    return j + 1


def build(src, rows):
    """Place parse-data ROWS (line1, col1, id, parent, token, terminal, text)
    on SRC: every terminal in source order at the next non-space text."""
    nodes = {0: Node(0, "root", False, "")}
    for _, _, id_, _, token, term, text in rows:
        nodes[id_] = Node(id_, token, term, text)
    orphans = []
    for _, _, id_, parent, *_ in rows:
        if parent > 0 and parent not in nodes:
            # getParseData() can leave out a node's parent row (seen in a
            # long-lived session): placed by its span below.
            orphans.append(nodes[id_])
            continue
        p = nodes.get(parent) or nodes[0]  # comments carry a negative parent
        nodes[id_].parent = p
        p.kids.append(nodes[id_])
    pos = 0
    for line1, col1, id_, *_ in sorted((r for r in rows if r[5]), key=lambda r: (r[0], r[1])):
        n = nodes[id_]
        j = src.find(n.text, pos) if n.text else pos
        if j < 0 or src[pos:j].strip():
            if n.token == "STR_CONST" and n.text.startswith("["):
                j = pos + len(src[pos:]) - len(src[pos:].lstrip())
                n.start, n.end = j, scan_string(src, j)
                n.text = src[n.start:n.end]
                pos = n.end
                continue
            raise ParseMismatch(f"token {n.token} {n.text!r} (line {line1}, col {col1}) not found at offset {pos}")
        n.start, n.end = j, j + len(n.text)
        pos = n.end

    def span(n):
        for k in n.kids:
            span(k)
        n.kids.sort(key=lambda k: k.start if k.start is not None else -1)
        placed = [k for k in n.kids if k.start is not None]
        if not n.terminal and placed:
            n.start, n.end = placed[0].start, max(k.end for k in placed)
    root = nodes[0]
    span(root)
    root.start, root.end = 0, len(src)
    for o in orphans:
        span(o)
        if o.start is None:
            continue
        home = root
        while True:
            inner = next((k for k in home.kids if not k.terminal and k.start is not None
                          and k.start <= o.start and o.end <= k.end and (k.start, k.end) != (o.start, o.end)), None)
            if inner is None:
                break
            home = inner
        o.parent = home
        home.kids.append(o)
        home.kids.sort(key=lambda k: k.start if k.start is not None else -1)
    return Tree(src, root)


class Parser:
    """GNU R's parser, one process for many queries (thread-safe)."""

    def __init__(self, rscript="Rscript"):
        self.dir = tempfile.mkdtemp(prefix="robigo-rast-")
        helper = os.path.join(self.dir, "helper.R")
        with open(helper, "w") as f:
            f.write(HELPER)
        env = dict(os.environ, R_DEFAULT_PACKAGES="NULL", LC_ALL="C.UTF-8")
        env.pop("R_HOME", None)  # Rscript finds its own; another R's would break it
        self.proc = subprocess.Popen([str(rscript), "--vanilla", helper], stdin=subprocess.PIPE,
                                     stdout=subprocess.PIPE, env=env, text=True)
        self.lock = threading.Lock()
        self.n = 0

    def parse(self, src, strict=False):
        """The Tree of SRC (str), or None when GNU R reports a syntax error
        or its parse data cannot be placed on SRC (invalid UTF-8, which R
        re-encodes in token texts; STRICT raises ParseMismatch then)."""
        with self.lock:
            self.n += 1
            path = os.path.join(self.dir, "query.R")
            with open(path, "w", encoding="utf-8", errors="surrogateescape") as f:
                f.write(src)
            self.proc.stdin.write(path + "\n")
            self.proc.stdin.flush()
            answer = self.proc.stdout.readline()
            if not answer:
                raise RuntimeError("the GNU R parse helper exited")
            if answer.strip() != "OK":
                return None
            with open(path + ".pd", encoding="utf-8") as f:
                lines = f.read().splitlines()
        rows = []
        for line in lines:
            l1, c1, id_, parent, token, term, hx = line.split("\t")
            rows.append((int(l1), int(c1), int(id_), int(parent), token, term == "1",
                         bytes.fromhex(hx).decode("utf-8", "surrogateescape")))
        try:
            return build(src, rows)
        except ParseMismatch:
            if strict:
                raise
            return None

    def close(self):
        try:
            self.proc.stdin.close()
            self.proc.wait(timeout=10)
        except (OSError, subprocess.TimeoutExpired):
            self.proc.kill()


def apply(src, edits):
    """SRC with EDITS [(start, end, replacement)] applied; overlapping
    deletions merge."""
    out, pos = [], 0
    for s, e, r in sorted(edits, key=lambda x: (x[0], x[1])):
        if s < pos:  # inside or overlapping the previous edit
            if r:
                raise ValueError("overlapping replacements")
            pos = max(pos, e)
            continue
        out.append(src[pos:s])
        out.append(r)
        pos = e
    out.append(src[pos:])
    return "".join(out)


# --- reduction units ---------------------------------------------------------

def statement_lists(tree):
    """Every statement list, outermost first: the top level and each `{ }`
    block, as lists of statement nodes."""
    lists = []
    for n in tree.root.walk():
        if n is tree.root:
            stmts = [k for k in n.kids if k.is_expr()]
        elif not n.terminal and n.kids and n.kids[0].token == "'{'":
            stmts = [k for k in n.kids if k.is_expr()]
        else:
            continue
        if stmts:
            lists.append((n, stmts))
    return lists


def arg_lists(tree):
    """Every argument list of a call, index or `function`, outermost first:
    (node, open token, close token, [argument [first, last] kid ranges])."""
    out = []
    for n in tree.root.walk():
        if n.terminal or not n.kids:
            continue
        ks = n.kids
        for i, k in enumerate(ks):
            if k.token in OPEN and (i > 0 and (ks[i - 1].is_expr() or ks[i - 1].token == "FUNCTION")):
                close = next((j for j in range(i + 1, len(ks)) if ks[j].token == OPEN[k.token]), None)
                if close is None:
                    continue
                args, cur = [], []
                for j in range(i + 1, close):
                    if ks[j].token == "','":
                        args.append(cur)
                        cur = []
                    elif ks[j].token != "COMMENT":
                        cur.append(ks[j])
                args.append(cur)
                if any(args):
                    out.append((n, k, ks[close], args))
                break
    return out


class Group:
    """Units one ddmin pass removes subsets of: `render(kept)` is the source
    with only the kept units (indices) left."""

    def __init__(self, src, units, render):
        self.src, self.units, self._render = src, units, render

    def render(self, kept):
        return self._render(kept)


def groups(tree):
    """Deletion groups of TREE, outermost first: each statement list, each
    argument list, and all comments."""
    src = tree.src
    out = []
    for _, stmts in statement_lists(tree):
        def span(i, stmts=stmts):
            s, e = stmts[i].start, stmts[i].end
            # take a separating `;` along
            nxt = src[e:].lstrip(" \t")
            if nxt.startswith(";"):
                e = len(src) - len(nxt) + 1
            return s, e

        def render(kept, stmts=stmts, span=span):
            gone = [span(i) for i in range(len(stmts)) if i not in kept]
            return apply(src, [(s, e, "") for s, e in gone])
        out.append(Group(src, stmts, render))
    for _, open_, close, args in arg_lists(tree):
        texts = [src[a[0].start:a[-1].end] if a else "" for a in args]

        def render(kept, texts=texts, open_=open_, close=close):
            return apply(src, [(open_.end, close.start, ", ".join(texts[i] for i in sorted(kept)))])
        out.append(Group(src, args, render))
    comments = tree.terminals("COMMENT")
    if comments:
        def render(kept, comments=comments):
            return apply(src, [(c.start, c.end, "") for i, c in enumerate(comments) if i not in kept])
        out.append(Group(src, comments, render))
    return out


def hoists(tree):
    """Candidates that replace an expression by one of its subexpressions
    (`f(g(x))` -> `g(x)`, `if (a) b else c` -> `b`), outermost first."""
    src = tree.src
    out = []
    for n in tree.root.walk():
        if not n.is_expr():
            continue
        for k in n.kids:
            if k.is_expr() and (k.start, k.end) != (n.start, n.end):
                out.append(apply(src, [(n.start, n.end, src[k.start:k.end])]))
    return out


# --- AST mutation --------------------------------------------------------------

LITERALS = ["0L", "1L", "-1L", "2147483647L", "-2147483647L", "NA", "NA_integer_", "NA_real_",
            "NA_character_", "NaN", "Inf", "-Inf", "0", "-0", "0.5", "1e308", "1e-320", "TRUE", "FALSE",
            "NULL", '""', '"a"', '"NA"', "1i", "integer(0)", "numeric(0)", "character(0)", "list()",
            "c(1L, NA)", "c(a = 1, b = 2)", "1:3", "as.raw(255)", "factor(\"x\")"]
OPERATORS = [["'+'", "'-'", "'*'", "'/'", "'^'", "SPECIAL"],
             ["GT", "GE", "LT", "LE", "EQ", "NE"],
             ["AND", "OR", "AND2", "OR2"]]
OP_TEXT = {"'+'": "+", "'-'": "-", "'*'": "*", "'/'": "/", "'^'": "^", "GT": ">", "GE": ">=", "LT": "<",
           "LE": "<=", "EQ": "==", "NE": "!=", "AND": "&", "OR": "|", "AND2": "&&", "OR2": "||"}
SPECIALS = ["%%", "%/%", "%in%", "%o%"]
WRAPS = ["as.numeric({})", "as.integer({})", "as.character({})", "as.logical({})", "c({0}, {0})", "rev({})",
         "-({})", "!({})", "({})[-1]", "list({})", "unlist(list({}))", "invisible({})", "suppressWarnings({})",
         "{{ {} }}", "(function() {})()", "local({})", "identity({})", "structure({}, names = NULL)",
         "tryCatch({}, error = function(e) conditionMessage(e))"]
LOOPS = ["for (.i in 1:20) {}", "for (.i in 1:20) {{ .r <- {} }}; print(.r)",
         "local({{ .f <- function() {}; for (.i in 1:600) .r <- .f(); print(.r) }})"]


def fragments(tree, limit=200):
    """Expression texts of TREE short enough to transplant."""
    return [tree.text(n) for n in tree.root.walk() if n.is_expr() and 0 < n.end - n.start <= limit
            and "\n" not in tree.text(n)]


def mutate(rnd, tree, pool, rounds=None):
    """A mutant of TREE's source that keeps its syntax: literals, operators,
    names and subexpressions replaced, statements dropped, duplicated,
    wrapped in loops. POOL is fragments() of all seeds."""
    src = tree.src
    exprs = [n for n in tree.root.walk() if n.is_expr() and n is not tree.root]
    if not exprs:
        return src
    lits = tree.terminals("NUM_CONST", "STR_CONST", "NULL_CONST")
    ops = tree.terminals(*[t for g in OPERATORS for t in g])
    syms = tree.terminals("SYMBOL")
    stmts = [s for _, ss in statement_lists(tree) for s in ss]
    edits = []
    taken = []

    def free(s, e):
        return all(e <= a or s >= b for a, b in taken)

    def add(s, e, r):
        if free(s, e):
            taken.append((s, e))
            edits.append((s, e, r))
    for _ in range(rounds or rnd.choice([1, 1, 1, 2, 2, 3])):
        k = rnd.randrange(8)
        if k == 0 and lits:
            n = rnd.choice(lits)
            add(n.start, n.end, rnd.choice(LITERALS))
        elif k == 1 and ops:
            n = rnd.choice(ops)
            if n.parent is not None and n.parent.kids[0] is n:  # unary: -x, +x
                add(n.start, n.end, rnd.choice(["-", "+", "!"]))
                continue
            g = next(g for g in OPERATORS if n.token in g)
            t = rnd.choice(g)
            add(n.start, n.end, rnd.choice(SPECIALS) if t == "SPECIAL" else OP_TEXT[t])
        elif k == 2 and len(syms) > 1:
            n = rnd.choice(syms)
            add(n.start, n.end, rnd.choice(syms).text)
        elif k == 3:
            n = rnd.choice(exprs)
            add(n.start, n.end, rnd.choice(pool) if pool else tree.text(rnd.choice(exprs)))
        elif k == 4:
            n = rnd.choice(exprs)
            add(n.start, n.end, rnd.choice(WRAPS).format(tree.text(n)))
        elif k == 5 and stmts:
            n = rnd.choice(stmts)
            add(n.start, n.end, "")
        elif k == 6 and stmts:
            n = rnd.choice(stmts)
            add(n.start, n.end, tree.text(n) + "\n" + tree.text(n))
        elif k == 7 and stmts:
            n = rnd.choice(stmts)
            add(n.start, n.end, rnd.choice(LOOPS).format(tree.text(n)))
    return apply(src, edits)


# --- equivalence modulo inputs ---------------------------------------------------

# Code that never runs. A guard of each kind: a constant the compiler can
# see through, and a variable it cannot.
GUARDS = ['FALSE && "emi"', ".emi.dead"]
PRELUDE = ".emi.dead <- FALSE\n"
PRELUDE_RX = re.compile(r"\A\.emi\.dead <- FALSE\n")
DEAD = ["{v} <- \"dead\"", "{v} <- NULL", "{v}[2] <- 1.5", "{v} <<- 0i", "rm({v})", "assign(\"{v}\", list())",
        "eval(quote({v} <- TRUE))", "delayedAssign(\"{v}\", stop(\"dead\"))", "return(NULL)", "break", "next",
        "stop(\"dead\")", "repeat {{}}", "{v} <- function(...) NULL", "on.exit(print(\"dead\"))",
        "makeActiveBinding(\"{v}\", function() 0, environment())", "local({v} <- 1L)", "{v}$x <- 1",
        "attr({v}, \"dead\") <- TRUE", "class({v}) <- \"dead\"", "sys.function()", "browser()",
        "while (TRUE) {v} <- {v} + 1"]


def insert_dead(rnd, tree, n=None):
    """TREE's source with N blocks that never run, each before an existing
    statement: the program means the same (the last statement of a block,
    its value, stays last)."""
    src = tree.src
    stmts = [s for _, ss in statement_lists(tree) for s in ss]
    if not stmts:
        return None
    names = sorted({t.text for t in tree.terminals("SYMBOL")} - {"T", "F"}) or ["x"]
    edits, used_var = [], False
    for s in rnd.sample(stmts, min(len(stmts), n or rnd.choice([1, 2, 3]))):
        guard = rnd.choice(GUARDS)
        used_var |= guard == ".emi.dead"
        if rnd.random() < 0.3:
            body = tree.text(rnd.choice(stmts))
        else:
            body = rnd.choice(DEAD).format(v=rnd.choice(names))
        edits.append((s.start, s.start, f"if ({guard}) {{ {body} }}\n"))
    out = apply(src, edits)
    return PRELUDE + out if used_var else out


def strip_dead(tree):
    """TREE's source without the blocks insert_dead() adds."""
    src = tree.src
    edits = []
    for n in tree.root.walk():
        ks = n.kids
        if (not n.terminal and len(ks) == 5 and ks[0].token == "IF" and ks[2].is_expr()
                and tree.text(ks[2]) in GUARDS):
            e = n.end + 1 if src[n.end:n.end + 1] == "\n" else n.end
            edits.append((n.start, e, ""))
    out = apply(src, edits)
    return PRELUDE_RX.sub("", out, count=1)


def selftest(rscript="Rscript"):
    """Round trips on a small script (scripts/test-reduce.py runs more)."""
    p = Parser(rscript)
    try:
        src = 'x <- c(1L, 2L) # two\nf <- function(a, b = "s\\"q") { a + b; a }\nprint(f(x, 3))\n'
        t = p.parse(src)
        assert t is not None
        assert [t.text(s) for s in statement_lists(t)[0][1]][0] == "x <- c(1L, 2L)"
        assert p.parse("f(") is None
        r = random.Random(0)
        d = insert_dead(r, t, 2)
        assert strip_dead(p.parse(d)) == src, (d, strip_dead(p.parse(d)))
    finally:
        p.close()
