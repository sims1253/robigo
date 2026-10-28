#!/usr/bin/env python3
"""Import the corpus-scripts corpora from GNU R 4.5.0's sources:

    scripts/import-corpus-scripts.py gnur-demos   # every shipped demo()
    scripts/import-corpus-scripts.py r-intro      # the code of "An Introduction to R"

Both corpora are whole, realistic R programs (see tests/corpora/README.md),
not snippets: `gnur-demos` runs each demo included in GNU R 4.5.0's base
packages the way `demo(topic, ask = FALSE, echo = TRUE)` does, and `r-intro`
assembles the manual's runnable examples chapter by chapter.

The sources come from GNU R 4.5.0's source tree ($ROBIGO_RSRC, default
~/.cache/robigo-research/rsrc/R-4.5.0). The importer only needs Python,
except for `r-intro`, where non-prompt example blocks are validated by
`parse()` with an Rscript (skipped with a warning if there is none).

`scripts/corpus.py record NAME` then records GNU R's output; see
tests/corpora/{gnur-demos,r-intro}/README.md for what is included and why
some demos/sections are not.
"""
import os
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CORPORA = os.path.join(ROOT, "tests", "corpora")
RSRC = os.environ.get("ROBIGO_RSRC", os.path.expanduser("~/.cache/robigo-research/rsrc/R-4.5.0"))

# ---------------------------------------------------------------- gnur-demos

# tcltk's demos open Tk windows: they need a display, which a corpus case (a
# subprocess with no X server) has not. Everything else is included.
DEMOS_SKIP = {"tcltk"}

DEMOS_HEADER = """\
## GNU R 4.5.0 demo({topic}) -- src/library/{pkg}/demo/{topic}.R.
## Part of R, GPL-2 or later. Run as demo("{topic}", package="{pkg}",
## ask = FALSE, echo = TRUE) runs it: the demo() banner, then the file
## itself with echo (the corpus's echo mode supplies the prompts).
options(keep.source = TRUE, useFancyQuotes = FALSE, warn = 1)
set.seed(123)
cat("\\n\\n\\tdemo({topic})\\n\\t---- {under}\\n", sep = "")
"""


def import_demos(_conf):
    lib = os.path.join(RSRC, "src", "library")
    if not os.path.isdir(lib):
        sys.exit(f"no GNU R sources at {RSRC} (set ROBIGO_RSRC)")
    cases = os.path.join(CORPORA, "gnur-demos", "cases")
    shutil.rmtree(cases, ignore_errors=True)
    n = skipped = 0
    for pkg in sorted(os.listdir(lib)):
        d = os.path.join(lib, pkg, "demo")
        if not os.path.isdir(d):
            continue
        os.makedirs(os.path.join(cases, pkg), exist_ok=True)
        for fn in sorted(os.listdir(d)):
            if not fn.endswith(".R"):
                continue
            topic = fn[:-2]
            if pkg in DEMOS_SKIP:
                skipped += 1
                continue
            with open(os.path.join(d, fn), "rb") as f:
                body = f.read()
            head = DEMOS_HEADER.format(topic=topic, pkg=pkg, under="~" * len(topic))
            with open(os.path.join(cases, pkg, fn), "wb") as f:
                f.write(head.encode("utf-8") + body)
            n += 1
    print(f"gnur-demos: {n} demos ({skipped} tcltk demos skipped: no display)")


# ------------------------------------------------------------------ r-intro

# TeXinfo markups that wrap R code inside @example blocks, innermost first.
MARKUP_RE = re.compile(
    r"@(code|var|r|samp|file|kbd|key|env|strong|emph|dfn|acronym|math|t|w|option|command|literal|i|b|sc)\{([^{}]*)\}")


def unmarkup(s):
    s = s.replace("@@", "\0")
    for _ in range(8):
        new = MARKUP_RE.sub(r"\2", s)
        if new == s:
            break
        s = new
    s = s.replace("@dots{}", "...").replace("@enddots{}", "....")
    s = s.replace("@{", "{").replace("@}", "}").replace("@ ", " ")
    return s.replace("\0", "@")


# Examples that cannot run in a batch session: they want a terminal, a
# browser, the network, a file that does not ship with R, or they stop R.
_SKIP = (
    r"\b(source|sys\.source|sink|write|writeLines|write\.table|save|load|dput|dump|history|savehistory|"
    r"loadhistory|library|require|requireNamespace|help|help\.start|example|demo|vignette|"
    r"install\.packages|update\.packages|setwd|getwd|system|system2|shell|q|quit|Sys\.sleep|locator|"
    r"identify|browser|edit|fix|file\.show|file\.choose|file\.path|png|pdf|postscript|jpeg|x11|windows|"
    r"quartz|dev\.new|dev\.off|dev\.set|dev\.cur|bmp|tiff|svg|"
    r"browseURL|download\.file|installed\.packages|available\.packages|R\.version|sessionInfo)\s*\(")
# .Last runs when R exits, so a block defining it would print the time of
# the recording; date()/Sys.time() are the same problem wherever they run.
SKIP_TIME_RE = re.compile(r"(^|[^[:alnum:]_.])(\.Last|date|Sys\.time|Sys\.Date|proc\.time|system\.time)\s*\(")
# Reading data needs a file: the manual's own examples read files that are
# not shipped with R, so those blocks only run where the file exists (the
# sample session reads datasets' morley.tab through system.file()).
SKIP_FILES_RE = re.compile(r"\b(scan|readLines|read\.table|read\.csv|read\.csv2|read\.delim|readRDS|file|url|gzfile)\s*\(")
SKIP_RE = re.compile(_SKIP)
# `library(pkg)` etc. without a call (prose patterns) and comments that are
# placeholders rather than code.
SKIP_TEXT_RE = re.compile(r"\[omitted|\[more|\[remaining|\[\.\.\.|@var|\.\.\.\]")


def blocks_of(lines):
    """[(first_line_no, [block lines])] for every @example in the file."""
    out, cur, start = [], None, 0
    for i, l in enumerate(lines, 1):
        if l.startswith("@example"):
            cur, start = [], i
        elif l.startswith("@end example") and cur is not None:
            out.append((start, cur))
            cur = None
        elif cur is not None:
            cur.append(l)
    return out


# Help requests, shell-script shebangs and elisions are not runnable code.
SKIP_LINE_RE = re.compile(r"^\s*(\?\??\S|#!)|^\s*\.\.\.\s*$")


def drop_lines(code):
    return [l for l in code if not SKIP_LINE_RE.search(l)]


def open_depth(lines):
    """(brackets still open, inside a string) at the end of these lines."""
    depth, quote = 0, None
    for l in lines:
        j = 0
        while j < len(l):
            ch = l[j]
            if quote:
                if ch == "\\":
                    j += 2
                    continue
                if ch == quote:
                    quote = None
            elif ch in "'\"":
                quote = ch
            elif ch == "#":
                break
            elif ch in "([{":
                depth += 1
            elif ch in ")]}":
                depth -= 1
            j += 1
    return depth, quote is not None


def prompt_code(block):
    """The R commands of a transcript block: `> ` lines, plus the lines that
    continue an unfinished expression (the manual aligns continuations
    instead of using R's `+ ` prompt); everything else is output."""
    code = []
    i = 0
    while i < len(block):
        if not block[i].startswith("> "):
            i += 1
            continue
        chunk = [block[i][2:]]
        i += 1
        # A command may run over several lines; output never does.
        while i < len(block) and any(open_depth(chunk)):
            nxt = block[i]
            chunk.append(nxt[2:] if nxt.startswith("+ ") else nxt)
            i += 1
        code.extend(chunk)
    return code


def validate(sources):
    """Which of the snippets parse as R; {} if no Rscript is available."""
    rscript = shutil.which("Rscript")
    if not rscript:
        print("warning: no Rscript: example blocks are not parse-checked")
        return None
    with tempfile.TemporaryDirectory() as tmp:
        for i, src in enumerate(sources):
            with open(os.path.join(tmp, f"b{i}.R"), "w", encoding="utf-8") as f:
                f.write(src)
        prog = (
            r"""a <- Sys.getenv('CORPUS_CHECK_DIR'); fs <- list.files(a, '[.]R$', full.names = TRUE)""" "\n"
            r"""for (f in fs) if (inherits(tryCatch(parse(file = f), error = function(e) e), 'error'))""" "\n"
            r"""  cat(basename(f), '\n', sep = '')""" "\n"
        )
        p = subprocess.run([rscript, "--vanilla", "-e", prog], capture_output=True, text=True,
                           env=dict(os.environ, CORPUS_CHECK_DIR=tmp))
        if p.returncode != 0:
            sys.exit("parse check failed: " + p.stderr)
        bad = {int(l[1:-2]) for l in p.stdout.splitlines() if re.fullmatch(r"b\d+\.R", l)}
    return {i for i in range(len(sources)) if i not in bad}


def prune(chapter_title, blocks, header):
    """Drop the example blocks a chapter cannot run at their place in the
    manual: a batch session stops at the first error, so a block whose
    objects come from a section that was left out would hide every later
    example. Run the chapter (with a marker before each block) and drop the
    block the session dies in, until it runs through.

    Returns the blocks to keep."""
    rscript = shutil.which("Rscript")
    if not rscript:
        return blocks
    blocks = list(blocks)
    while blocks:
        with tempfile.TemporaryDirectory() as tmp:
            f = os.path.join(tmp, "chapter.R")
            with open(f, "w", encoding="utf-8") as fh:
                fh.write("\n".join(header) + "\n")
                for i, code in enumerate(blocks):
                    fh.write(f'cat("@@BLOCK{i}@@\\n")\n')
                    fh.write("\n".join(code) + "\n\n")
            p = subprocess.run([rscript, "--vanilla", f], capture_output=True, text=True, timeout=300)
        if p.returncode == 0:
            return blocks
        marks = re.findall(r"@@BLOCK(\d+)@@", p.stdout)
        if not marks:
            sys.exit(f"{chapter_title}: cannot run its header:\n{p.stderr}")
        why = next((l for l in p.stderr.splitlines() if l.startswith("Error")), p.stderr.strip().splitlines()[-1])
        print(f"  {chapter_title}: drop block {marks[-1]} ({why[:90]})")
        del blocks[int(marks[-1])]
    return blocks


CHAPTERS_FILE = "R-intro.texi"


def import_r_intro(_conf):
    path = os.path.join(RSRC, "doc", "manual", CHAPTERS_FILE)
    if not os.path.isfile(path):
        sys.exit(f"no {path} (set ROBIGO_RSRC)")
    with open(path, encoding="utf-8") as f:
        lines = f.read().split("\n")
    cases = os.path.join(CORPORA, "r-intro", "cases")
    shutil.rmtree(cases, ignore_errors=True)
    os.makedirs(cases)

    # Split into chapters (appendices included) and remember where each
    # section starts, so the examples can be grouped with a comment.
    chapters = []  # [{"title", "kind", "start", "sections": [(line, title)]}]
    for i, l in enumerate(lines, 1):
        m = re.match(r"@(chapter|appendix) (.*)", l)
        if m:
            chapters.append({"title": unmarkup(m.group(2)), "kind": m.group(1), "start": i, "sections": []})
            continue
        m = re.match(r"@(section|appendixsection) (.*)", l)
        if m and chapters:
            chapters[-1]["sections"].append((i, unmarkup(m.group(2))))

    blocks = blocks_of(lines)
    # Turn each example block into code (or nothing) and give it to the
    # chapter it appears in. `>`/`+` transcripts give their commands; a
    # block with no prompt is code only if it parses as R.
    cand = []  # [(chapter index, first line, [code lines])]
    for start, blk in blocks:
        ch = max((k for k, c in enumerate(chapters) if c["start"] < start), default=-1)
        if ch < 0:
            continue
        code = prompt_code(blk) or blk
        code = drop_lines([unmarkup(l) for l in code])
        text = "\n".join(code)
        if not text.strip() or SKIP_FILES_RE.search(text) or SKIP_RE.search(text) \
                or SKIP_TIME_RE.search(text) or SKIP_TEXT_RE.search(text) or "@" in text:
            continue
        cand.append((ch, start, code))
    ok = validate(["\n".join(c) for _, _, c in cand])
    kept = [k for i, k in enumerate(cand) if ok is None or i in ok]

    n = 0
    for ch, chapter in enumerate(chapters):
        body = [k for k in kept if k[0] == ch]
        if not body:
            continue
        slug = re.sub(r"[^a-z0-9]+", "-", chapter["title"].lower()).strip("-")
        name = f"{n + 1:02d}-{slug}.R"
        header = [
            f"## An Introduction to R (GNU R 4.5.0), {chapter['kind']}: {chapter['title']}",
            f"## The manual's runnable examples, from doc/manual/{CHAPTERS_FILE} in the R",
            "## sources (part of R, GPL-2 or later). Examples that need a terminal,",
            "## a browser, the network or a data file are left out (see README.md).",
            "options(keep.source = TRUE, useFancyQuotes = FALSE, warn = 1)",
            "set.seed(123)",
        ]
        runs = prune(chapter["title"], [c for _, _, c in body], header)
        runs = {id(c) for c in runs}
        out = list(header)
        section = None
        for _, start, code in body:
            if id(code) not in runs:
                continue
            title = [t for line, t in chapter["sections"] if line < start]
            if title and title[-1] != section:
                section = title[-1]
                out.append("")
                out.append(f"## ---- {section}")
            out.extend(code)
            out.append("")
        if len([1 for l in out[len(header):] if l.strip() and not l.startswith("##")]) < 2:
            print(f"  {chapter['title']}: no runnable examples left, no case")
            continue
        with open(os.path.join(cases, name), "w", encoding="utf-8") as f:
            f.write("\n".join(out).rstrip("\n") + "\n")
        n += 1
    # Appendix A is a @table of session commands, not @example blocks.
    app = appendix_a(lines)
    if app:
        with open(os.path.join(cases, "appendix-a-sample-session.R"), "w", encoding="utf-8") as f:
            f.write("\n".join([
                "## An Introduction to R (GNU R 4.5.0), appendix: A sample session",
                f"## The session's commands, from doc/manual/{CHAPTERS_FILE} in the R sources",
                "## (part of R, GPL-2 or later). Commands that need a browser or a pager",
                "## (help.start(), file.show()) and q() are left out.",
                "options(keep.source = TRUE, useFancyQuotes = FALSE, warn = 1)",
                "set.seed(123)",
            ] + app) + "\n")
        n += 1
    print(f"r-intro: {n} case scripts from {len(blocks)} example blocks "
          f"({len(kept)} blocks kept, {len(blocks) - len(kept)} left out)")


def appendix_a(lines):
    """The commands of "A sample session": `@item` lines of its @table, with
    `@itemx` continuations joined onto the command they continue."""
    start = next((i for i, l in enumerate(lines) if l.startswith("@appendix A sample session")), None)
    if start is None:
        return []
    end = next((i for i, l in enumerate(lines[start + 1:], start + 1) if l.startswith("@appendix ")), len(lines))
    cmds = []
    for l in lines[start:end]:
        m = re.match(r"@item(x?) (.*)", l)
        if not m:
            continue
        txt = unmarkup(m.group(2))
        if m.group(1) and cmds:
            cmds[-1] += "\n" + txt
        else:
            cmds.append(txt)
    return [c for c in drop_lines(cmds) if not SKIP_RE.search(c)]


IMPORTERS = {"gnur-demos": import_demos, "r-intro": import_r_intro}


def main():
    if len(sys.argv) != 2 or sys.argv[1] not in IMPORTERS:
        sys.exit(f"usage: {sys.argv[0]} {{{'|'.join(IMPORTERS)}}}")
    IMPORTERS[sys.argv[1]](None)


if __name__ == "__main__":
    main()
