# The body of scripts/base-subset.sh, run by GNU R 4.6.2 in its oracle image
# from the repository root:
#
#   Rscript --vanilla scripts/base-subset.R WORK OUT BOUND
#   Rscript --vanilla scripts/base-subset.R --inputs   # the input scripts in
#                                                      # the checkout, one a line
#
# WORK holds the pinned sources' src/library/base/R (as base-R/) and gets
# the build and run files; OUT is the subset file written, BOUND the bound
# robigo-rt's loader binds (the traced subset and the static bound). Each input runs
# twice, untraced and traced (scripts/base-subset-trace.R); the two must
# print the same bytes and exit the same way, or the tracer changed what it
# measures and the run fails. The capi cases' C is compiled here with
# GNU R 4.6.2's R CMD SHLIB.
args <- commandArgs(trailingOnly = TRUE)
# (--inputs: scripts/regen-expected.sh asks which of the scripts it recorded
# are inputs here, so that it can say the subset needs regenerating)
inputs_only <- identical(args, "--inputs")
if (!inputs_only && length(args) != 3L) {
  stop("usage: Rscript --vanilla scripts/base-subset.R WORK OUT BOUND | --inputs")
}
if (!inputs_only) {
  work <- normalizePath(args[[1L]], mustWork = TRUE)
  out <- args[[2L]]
  bound_out <- args[[3L]]
}
root <- normalizePath(".")
if (getRversion() != "4.6.2") stop("base-subset.R needs GNU R 4.6.2, not ", getRversion())
Sys.setenv(LANGUAGE = "en", LC_ALL = "C.UTF-8", TZ = "UTC", R_DEFAULT_PACKAGES = "NULL")
Sys.unsetenv("TZDIR")

# The M1 scripts (docs/g3.md §14.2 M1; §15.3 E1, E2, E6).
inputs <- list(
  list(id = "calls", file = "benchmarks/suite/calls.R"),
  list(id = "fib", file = "benchmarks/suite/fib.R"),
  list(id = "binarytrees", file = "benchmarks/suite/binarytrees.R"),
  # robigo-rt's variants of the capi corpus's cases, experiment A's probes,
  # weak-keys-collections and the stress script (A1): their C is
  # tests/rt/capi/support/*.c, loaded from RT_CAPI_LIBS
  list(id = "capi-eval", file = "tests/rt/capi/eval.R", c = "tests/rt/capi/support/eval.c"),
  list(id = "capi-pairlist", file = "tests/rt/capi/pairlist.R", c = "tests/rt/capi/support/pairlist.c"),
  list(id = "capi-unwind", file = "tests/rt/capi/unwind.R", c = "tests/rt/capi/support/unwind.c"),
  list(id = "capi-extptr", file = "tests/rt/capi/extptr.R", c = "tests/rt/capi/support/extptr.c"),
  list(id = "pin-premature", file = "tests/rt/capi/pin_premature.R", c = "tests/rt/capi/support/pin_premature.c"),
  list(id = "pin-extbuf", file = "tests/rt/capi/pin_extbuf.R", c = "tests/rt/capi/support/pin_extbuf.c"),
  list(id = "weak-keys", file = "tests/rt/capi/weak-keys-collections.R", c = "tests/rt/capi/support/weak_keys.c"),
  list(id = "stress", file = "tests/rt/stress/finalizer-depths.R", c = "tests/rt/capi/support/stress.c")
)
# The compiler package's R code (C1-BC): robigo-rt evaluates the pinned
# tarball's cmp.R into the compiler namespace it makes (src/rt ns.rs,
# INTERIM(L2)), so what cmp.R names is bound too; base-subset.sh copies it.
if (!inputs_only) {
  inputs <- c(inputs, list(list(id = "compiler", file = substring(file.path(work, "compiler-R", "cmp.R"), nchar(root) + 2L))))
  # The decision bars' D2S and E7 rows, written as scripts by base-subset.sh.
  for (f in sort(Sys.glob(file.path(work, "rows", "*.R")), method = "radix")) {
    inputs <- c(inputs, list(list(id = paste0("row:", sub("[.]R$", "", basename(f))),
                                  file = substring(f, nchar(root) + 2L))))
  }
}
# The conformance cases of the condition system and S3 dispatch (S1), of
# sorting (sort.R, rank.R and what they call), of environments (N1:
# locks, active bindings, mget, env.profile, list2env), of regular
# expressions (RX: grep.R, and try() reporting their errors), of
# serialization (L1: serialize, saveRDS, connections) and of namespaces,
# the search path and parse() (L2a): they reach those
# files' R code. Each exits with the status GNU R 4.6.2's recording of it has
# (tests/rt/R-4.6.2/AREA/NAME.Rstatus). (Not `args`: tracing changes the
# binding types its cases print.)
for (area in c("bytecode", "conditions", "env", "ns", "parse", "regex", "s3", "serialize", "sort")) {
  for (f in sort(Sys.glob(file.path(root, "tests", "rt", area, "*.R")), method = "radix")) {
    name <- sub("[.]R$", "", basename(f))
    st <- if (inputs_only) NA_integer_ else
      as.integer(readLines(file.path(root, "tests", "rt", "R-4.6.2", area, paste0(name, ".Rstatus"))))
    inputs <- c(inputs, list(list(id = paste0(area, ":", name), file = substring(f, nchar(root) + 2L), status = st)))
  }
}

if (inputs_only) {
  writeLines(vapply(inputs, function(i) i$file, ""))
  quit(save = "no")
}

run_dir <- file.path(work, "run")
unlink(run_dir, recursive = TRUE)
dir.create(run_dir, recursive = TRUE)

# R CMD SHLIB in `dir`; pairlist.c's SET_TYPEOF is not in 4.6.2's API (it
# is in a function the case does not call), so a failed build retries with
# a macro that makes that call an error.
shlib <- function(dir, c_file) {
  old <- setwd(dir)
  on.exit(setwd(old))
  file.copy(file.path(root, c_file), ".", overwrite = TRUE)
  src <- basename(c_file)
  log <- "shlib.log"
  st <- system2("R", c("CMD", "SHLIB", src), stdout = log, stderr = log)
  if (st != 0L && any(grepl("SET_TYPEOF", readLines(log), useBytes = TRUE))) {
    writeLines('#define SET_TYPEOF(x, t) Rf_error("SET_TYPEOF is not in GNU R 4.6\'s C API")', "shim.h")
    unlink(sub("\\.c$", ".o", src))
    st <- system2("R", c("CMD", "SHLIB", src), stdout = log, stderr = log,
                  env = paste0("PKG_CPPFLAGS='-include ", normalizePath("shim.h"), "'"))
  }
  if (st != 0L) stop("R CMD SHLIB ", c_file, " failed:\n", paste(readLines(log), collapse = "\n"))
}

# stdout, stderr and status of `Rscript ARGS script` in `dir`.
run <- function(dir, script, rargs, env = character()) {
  old <- setwd(dir)
  on.exit(setwd(old))
  o <- tempfile(tmpdir = dir)
  e <- tempfile(tmpdir = dir)
  st <- system2("Rscript", c(rargs, shQuote(script)), stdout = o, stderr = e, env = env)
  res <- list(out = readBin(o, "raw", file.size(o)), err = readBin(e, "raw", file.size(e)), status = st)
  unlink(c(o, e))
  res
}

# What a comparison sees. GNU R's "stack imbalance" warning names the
# protect stack's depth, which the tracer's own objects change (capi
# extptr's C leaves one object protected); the warning itself must match.
# By bytes: a transcript may hold bytes that are not valid in the session's
# locale (conditions/native-bytes), which gsub refuses under UTF-8.
mask <- function(x) {
  if (!is.raw(x)) return(format(x))
  gsub("(stack imbalance in '[^']*', )[0-9]+ then [0-9]+", "\\1N then M", rawToChar(x), useBytes = TRUE)
}

trace_r <- file.path(root, "scripts", "base-subset-trace.R")

# The failure when tracing changes what an input prints. Not a stale list
# (regenerating cannot fix it): the case prints something the tracer
# changes. Shows the first line that differs, not the whole transcripts.
traced_differs <- function(file, part, a, b, saved) {
  la <- strsplit(a, "\n", fixed = TRUE)[[1L]]
  lb <- strsplit(b, "\n", fixed = TRUE)[[1L]]
  n <- max(length(la), length(lb))
  i <- which(vapply(seq_len(n), function(k) !identical(la[k], lb[k]), NA))[1L]
  show <- function(x) if (is.na(x)) "(no line)" else x
  paste0(
    sprintf("base-subset: %s prints differently when traced: its %s differs at line %d.\n", file,
            c(out = "stdout", err = "stderr", status = "exit status")[[part]], i),
    sprintf("  untraced: %s\n  traced:   %s\n", show(la[i]), show(lb[i])),
    "This is not a stale list, and regenerating cannot fix it: the case prints something that\n",
    "scripts/base-subset-trace.R changes: it gives every closure of base (and of the script) a body\n",
    "that records what runs, so a deparsed function or body, a binding's type, sys.function(), or\n",
    "the protect stack's depth differ.\n",
    "Make the case print what does not depend on that, or leave the case out of the inputs in\n",
    "scripts/base-subset.R and say why there.\n",
    sprintf("Both transcripts: %s", saved))
}

# The tracer's own check, first: a branch not taken and a closure not
# called must not count; what runs must, including a closure bound at top
# level, a primitive passed to lapply and a replacement function.
selftest <- list(
  src = c(
    "f <- function(x) if (x) sum(1) else prod(2)",
    "f(TRUE)",
    "g <- function() { h <- function() nchar('a'); if (FALSE) rev(1); for (i in 1:2) h() }",
    "invisible(g())",
    "k <- function(a) a",
    "k(abs(-1))",
    "e <- new.env()",
    "e$v <- lapply(1:2, length)",
    "cat(paste0('a', 'b'), '\\n')"
  ),
  want = c("user:f", "primitive:sum", "user:g", "user:k", "closure:nchar", "internal:nchar", "primitive:for",
           "primitive:invisible", "primitive:abs", "closure:new.env", "primitive:$<-", "closure:lapply",
           "internal:lapply", "primitive:length", "closure:paste0", "internal:paste0", "closure:cat"),
  never = c("primitive:prod", "closure:rev")
)

# The primitives a capi case's C evaluates: R code records nothing when C
# builds a call (`lang2(install("sqrt"), ...)`) or parses text
# (`R_ParseVector(mkString("..."))`) and evaluates it. Every primitive the C
# names this way counts, evaluated or not; closures it reaches run R code
# and record themselves.
c_keys <- function(c_file) {
  src <- paste(readLines(file.path(root, c_file)), collapse = "\n")
  installed <- regmatches(src, gregexpr('install\\("[^"]+"\\)', src))[[1L]]
  installed <- sub('^install\\("(.*)"\\)$', "\\1", installed)
  parsed <- regmatches(src, gregexpr('R_ParseVector\\(mkString\\("([^"\\\\]|\\\\.)*"\\)', src))[[1L]]
  parsed <- sub('^R_ParseVector\\(mkString\\("(.*)"\\)$', "\\1", parsed)
  heads <- installed
  calls <- function(e) {
    if (!is.call(e)) return(character())
    h <- if (is.symbol(e[[1L]])) as.character(e[[1L]]) else character()
    c(h, unlist(lapply(as.list(e), calls)))
  }
  for (text in parsed) {
    text <- eval(parse(text = paste0('"', text, '"'))) # the C string's escapes
    heads <- c(heads, unlist(lapply(as.list(parse(text = text, keep.source = FALSE)), calls)))
  }
  heads <- unique(heads)
  isprim <- vapply(heads, function(n) exists(n, envir = baseenv(), inherits = FALSE) &&
                     is.primitive(get(n, envir = baseenv())), NA)
  if (any(isprim)) paste0("primitive:", heads[isprim]) else character()
}

reached <- list() # input id -> keys
inputs <- c(list(list(id = "selftest", file = NULL)), inputs)
for (inp in inputs) {
  dir <- file.path(run_dir, inp$id)
  dir.create(dir)
  if (!is.null(inp$c)) shlib(dir, inp$c)
  script <- if (is.null(inp$file)) file.path(dir, "selftest.R") else file.path(root, inp$file)
  if (is.null(inp$file)) writeLines(selftest$src, script)
  # (the cases load their C from RT_CAPI_LIBS, here the input's directory)
  libs <- if (is.null(inp$c)) character() else paste0("RT_CAPI_LIBS=", shQuote(dir))
  plain <- run(dir, script, "--vanilla", env = libs)
  keys_file <- file.path(dir, "keys.txt")
  traced <- run(dir, script, c("--no-environ", "--no-site-file", "--no-save", "--no-restore"),
                env = c(paste0("R_PROFILE_USER=", shQuote(trace_r)), "R_ENABLE_JIT=0",
                        paste0("ROBIGO_SUBSET_OUT=", shQuote(keys_file)), libs))
  for (part in c("out", "err", "status")) {
    a <- mask(plain[[part]])
    b <- mask(traced[[part]])
    if (!identical(a, b)) {
      saved <- file.path(dir, paste0("mismatch-", part, ".txt"))
      writeLines(c(a, "--- traced", b), saved)
      stop(traced_differs(if (is.null(inp$file)) "the self-test" else inp$file, part, a, b, saved), call. = FALSE)
    }
  }
  want <- if (is.null(inp$status)) 0L else inp$status
  if (plain$status != want) {
    rec <- if (startsWith(inp$file, "tests/rt/")) {
      sprintf(" Its recording (tests/rt/R-4.6.2/%s.Rstatus) says %d: if the case changed, re-record it with\n  scripts/oracle-run.sh 4.6.2 scripts/regen-expected.sh %s\nand then run scripts/base-subset.sh.",
              sub("^tests/rt/(.*)[.]R$", "\\1", inp$file), want, inp$file)
    } else ""
    stop(sprintf("base-subset: %s exits with status %d under GNU R 4.6.2, not %d.%s\nIts stderr:\n%s",
                 inp$file, plain$status, want, rec, rawToChar(plain$err)), call. = FALSE)
  }
  keys <- readLines(keys_file)
  if (is.null(inp$file)) {
    missed <- setdiff(selftest$want, keys)
    wrong <- intersect(selftest$never, keys)
    if (length(missed) || length(wrong)) {
      stop("the tracer's self-test failed: missed ", paste(missed, collapse = " "), "; counted ",
           paste(wrong, collapse = " "))
    }
    next
  }
  if (!is.null(inp$c)) keys <- union(keys, c_keys(inp$c))
  reached[[inp$id]] <- keys
  message(sprintf("%-14s %4d keys", inp$id, length(keys)))
}
inputs <- inputs[-1L]
# capi eval's C evaluates sqrt (the case that showed C-evaluated calls).
if (!"primitive:sqrt" %in% reached[["capi-eval"]]) stop("capi-eval: primitive:sqrt was not counted")

# Which file of src/library/base/R defines each name: the last top-level
# assignment in collation order (base's Makefile: LC_COLLATE=C ls R/*.R
# R/unix/*.R), as GNU R's build concatenates them.
base_r <- file.path(work, "base-R")
files <- c(sort(Sys.glob(file.path(base_r, "*.R")), method = "radix"),
           sort(Sys.glob(file.path(base_r, "unix", "*.R")), method = "radix"))
defined_in <- new.env(parent = emptyenv())
# and the right-hand side of that assignment (a variable's definition).
# Only `<-` and `=` define base's bindings: base's top-level assign() calls
# (zzz.R's) all write to .ArgsEnv or .GenericArgsEnv, not base.
defined_rhs <- new.env(parent = emptyenv())
for (f in files) {
  rel <- substring(f, nchar(base_r) + 2L)
  for (e in parse(f, keep.source = FALSE)) {
    if (is.call(e) && as.character(e[[1L]])[1L] %in% c("<-", "=") && length(e) == 3L) {
      lhs <- e[[2L]]
      if (is.symbol(lhs) || (is.character(lhs) && length(lhs) == 1L)) {
        assign(as.character(lhs), rel, envir = defined_in)
        assign(as.character(lhs), list(e[[3L]]), envir = defined_rhs)
      }
    }
  }
}

# One line per key, KIND NAME [FILE], in `out`; and, with the inputs that
# reach it, KIND NAME [FILE] INPUTS, in WORK/reached-by.txt, which is not
# committed: a new input then changes `out` only where it reaches a new name.
all_keys <- sort(unique(unlist(reached)), method = "radix")
lines <- character()
by_lines <- character()
counts <- c(closure = 0L, primitive = 0L, internal = 0L)
unresolved <- character()
for (k in all_keys) {
  kind <- sub(":.*", "", k)
  name <- sub("^[^:]*:", "", k)
  if (kind == "user") next
  if (!kind %in% names(counts)) stop("unknown key ", k)
  if (!nzchar(name) || grepl("[[:space:]]", name)) stop("an empty name or one with white space: ", k)
  by <- paste(names(reached)[vapply(reached, function(r) k %in% r, NA)], collapse = ",")
  if (kind == "closure") {
    file <- get0(name, envir = defined_in, inherits = FALSE, ifnotfound = "?")
    if (file == "?") unresolved <- c(unresolved, name)
    lines <- c(lines, paste("closure", name, file))
  } else {
    lines <- c(lines, paste(kind, name))
  }
  by_lines <- c(by_lines, paste(lines[length(lines)], by))
  counts[[kind]] <- counts[[kind]] + 1L
}
ord <- order(match(sub(" .*", "", lines), names(counts)), lines, method = "radix")
header <- c(
  "# robigo-rt's base subset: what GNU R 4.6.2 reaches of base running M1's",
  "# scripts and the rt slice's cases (docs/g3.md §14.2 M1). Generated by",
  "# scripts/base-subset.sh from the pinned 4.6.2 tarball and oracle image; do",
  "# not edit.",
  "#",
  "#   closure NAME FILE   a base closure entered; FILE is the file of",
  "#                       src/library/base/R that defines it (collation:",
  "#                       LC_COLLATE=C ls R/*.R R/unix/*.R)",
  "#   primitive NAME      a primitive called, or passed to a higher-order function",
  "#   internal NAME       an .Internal called",
  "#",
  "# Which scripts reach each name is not kept here, so that a new case changes",
  "# this file only where it reaches a new name: scripts/base-subset.sh writes",
  "# it to .robigo/base-subset/reached-by.txt, and CI's base-subset job keeps",
  "# that as an artifact.",
  "#",
  "# Closures and internals are counted where they run. Primitives are counted",
  "# per statement, so an untaken `&&`/`||` operand or an unforced argument in",
  "# a statement that runs counts too; primitives that C code calls only by",
  "# name (do.call(\"c\", ...)) do not, except that a capi case counts every",
  "# primitive its C names (install(\"sqrt\")) or parses. Top-level printing in",
  "# C does not count.",
  sprintf("# %d closures, %d primitives, %d internals.", counts[["closure"]], counts[["primitive"]],
          counts[["internal"]])
)
writeLines(c(header, lines[ord]), out)
# the inputs and, per name, the ones that reach it
writeLines(c(
  "# The scripts that reach each name of crates/robigo-rt/proto/base-subset.txt",
  "# (scripts/base-subset.sh; not committed). INPUTS ids:",
  vapply(inputs, function(i) sprintf("#   %-14s %s", i$id, i$file), ""),
  "#",
  "#   closure NAME FILE INPUTS, primitive NAME INPUTS, internal NAME INPUTS",
  by_lines[ord]
), file.path(work, "reached-by.txt"))
if (length(unresolved)) {
  stop("no file of src/library/base/R defines these reached closures: ", paste(unresolved, collapse = ", "))
}
message(sprintf("%s: %d closures, %d primitives, %d internals", out, counts[["closure"]],
                counts[["primitive"]], counts[["internal"]]))

# The static bound, as m1-plan's estimate made it with codetools::findGlobals
# (which the image lacks): the free variables of the same inputs that name
# base functions, transitively through base closures' bodies and defaults;
# and those that name base variables base's R code defines (letters,
# ..deparseOpts), through their definitions.
# Compared with the trace (docs/ledger/g3/2026-11-13-rt-conformance.md),
# and with it the bound robigo-rt's loader binds (BOUND).
free <- function(e, locals) {
  acc <- character()
  ints <- character()
  walk <- function(e) {
    if (is.symbol(e)) {
      n <- as.character(e)
      if (nzchar(n) && !n %in% locals) acc <<- c(acc, n)
      return()
    }
    if (!is.call(e)) return()
    h <- e[[1L]]
    n <- if (is.symbol(h)) as.character(h) else ""
    if (n %in% c("quote", "~")) {
      acc <<- c(acc, n)
      return()
    }
    # UseMethod("gen") reaches gen's default method (robigo-rt's loader
    # evaluates a base variable's definition at run time, where GNU R's
    # build evaluated it: what it dispatches to must be bound)
    if (n == "UseMethod" && length(e) >= 2L && is.character(e[[2L]]))
      acc <<- c(acc, paste0(e[[2L]], ".default"))
    if (n == ".Internal") {
      acc <<- c(acc, n)
      inner <- e[[2L]]
      ints <<- c(ints, as.character(inner[[1L]]))
      for (i in seq_along(inner)[-1L]) if (!identical(inner[[i]], quote(expr = ))) walk(inner[[i]])
      return()
    }
    if (n == "function") {
      acc <<- c(acc, n)
      fl <- as.list(e[[2L]])
      g <- free(e[[3L]], c(locals, names(fl)))
      acc <<- c(acc, g$acc)
      ints <<- c(ints, g$ints)
      for (i in seq_along(fl)) if (!identical(fl[[i]], quote(expr = ))) walk(fl[[i]])
      return()
    }
    if (n %in% c("<-", "=", "<<-", "for")) {
      tgt <- e[[2L]]
      while (is.call(tgt)) {
        # f(x) <- v calls `f<-`
        if (n != "for" && is.symbol(tgt[[1L]])) acc <<- c(acc, paste0(as.character(tgt[[1L]]), "<-"))
        tgt <- tgt[[2L]]
      }
      if (is.symbol(tgt) && n != "<<-") locals <<- c(locals, as.character(tgt))
    }
    for (i in seq_along(e)) if (!identical(e[[i]], quote(expr = ))) walk(e[[i]])
  }
  walk(e)
  list(acc = unique(acc), ints = unique(ints))
}
# A script's top-level expressions as the console reads it
# (main.c:R_ReplConsole): a statement at a time, and a syntax error drops
# the lines read for its statement, then reading goes on at the next line.
# GNU R runs the lines after it when options(catch.script.errors) or
# options(error) is set (main.c:check_session_exit) and otherwise halts
# there; they are walked either way, which can only add names. What is not
# walked is reported.
script_exprs <- function(file) {
  p <- tryCatch(parse(file, keep.source = FALSE), error = identity)
  if (!inherits(p, "error")) return(as.list(p))
  lines <- readLines(file, warn = FALSE)
  out <- list()
  buf <- character()
  start <- 1L
  for (i in seq_along(lines)) {
    if (!length(buf)) start <- i
    buf <- c(buf, lines[[i]])
    r <- tryCatch(parse(text = buf, keep.source = FALSE), error = identity)
    if (!inherits(r, "error")) {
      out <- c(out, as.list(r))
      buf <- character()
    } else if (!grepl("unexpected (end of input|INCOMPLETE_STRING)", conditionMessage(r))) {
      message(sprintf("base-subset: %s:%d-%d not walked: %s", substring(file, nchar(root) + 2L), start, i,
                      sub("\n.*", "", sub("^<text>:[0-9]+:[0-9]+: ", "", conditionMessage(r)))))
      buf <- character()
    }
  }
  if (length(buf)) {
    message(sprintf("base-subset: %s:%d-%d not walked: unexpected end of input",
                    substring(file, nchar(root) + 2L), start, length(lines)))
  }
  out
}
exprs <- unlist(lapply(inputs, function(i) script_exprs(file.path(root, i$file))), recursive = FALSE)
g <- free(as.call(c(as.name("{"), exprs)), character())
# and through the bodies of the closures the trace entered (S3 methods and
# what C calls are reached by no name in the inputs' text, and their
# branches GNU R did not take name functions too)
todo <- c(g$acc, sub("^closure (\\S+) .*", "\\1", lines[startsWith(lines, "closure ")]))
ints <- g$ints
seen <- character()
prims <- character()
vars <- character()
# base variables GNU R's C reads by name, bound with the internal that
# reads them: do_args's argument environments
c_vars <- list(args = c(".ArgsEnv", ".GenericArgsEnv"))
repeat {
while (length(todo)) {
  n <- todo[[1L]]
  todo <- todo[-1L]
  if (n %in% seen || !exists(n, envir = baseenv(), inherits = FALSE)) next
  f <- get(n, envir = baseenv())
  if (!is.function(f)) {
    def <- get0(n, envir = defined_rhs, inherits = FALSE)
    # (not .GlobalEnv: its definition, `environment()`, is the global
    # environment only as GNU R's base build evaluates it, not in a promise
    # in the base namespace)
    if (!is.null(def) && n != ".GlobalEnv") {
      seen <- c(seen, n)
      vars <- c(vars, n)
      gg <- free(def[[1L]], character())
      todo <- c(todo, setdiff(gg$acc, seen))
      ints <- c(ints, gg$ints)
    }
    next
  }
  seen <- c(seen, n)
  if (is.primitive(f)) {
    prims <- c(prims, n)
    next
  }
  # robigo-rt's loader binds a closure to its definition, evaluated: one that
  # is not a function literal names what it needs too (as.R's `as.name <-
  # as.symbol`)
  def <- get0(n, envir = defined_rhs, inherits = FALSE)
  if (!is.null(def) && !(is.call(def[[1L]]) && identical(def[[1L]][[1L]], as.name("function")))) {
    gg <- free(def[[1L]], character())
    todo <- c(todo, setdiff(gg$acc, seen))
    ints <- c(ints, gg$ints)
  }
  fl <- formals(f)
  for (x in c(list(body(f)), Filter(function(d) !identical(d, quote(expr = )), as.list(fl)))) {
    gg <- free(x, names(fl))
    todo <- c(todo, setdiff(gg$acc, seen))
    ints <- c(ints, gg$ints)
  }
}
todo <- setdiff(unlist(c_vars[intersect(names(c_vars), ints)], use.names = FALSE), seen)
if (!length(todo)) break
}
static <- sort(c(paste("closure", setdiff(seen, c(prims, vars))), paste("primitive", prims),
                 paste("internal", unique(ints)), if (length(vars)) paste("variable", vars)),
               method = "radix")
traced <- sort(sub("^(\\S+ \\S+).*", "\\1", lines), method = "radix")
writeLines(static, file.path(work, "static.txt"))
writeLines(setdiff(traced, static), file.path(work, "traced-only.txt"))
writeLines(setdiff(static, traced), file.path(work, "static-only.txt"))
count <- function(x, kind) sum(startsWith(x, paste0(kind, " ")))
message(sprintf("static bound: %d closures, %d primitives, %d internals; %d in both; %s/{static,traced-only,static-only}.txt",
                count(static, "closure"), count(static, "primitive"), count(static, "internal"),
                length(intersect(static, traced)), work))

# The bound robigo-rt's loader binds (the foreman's rule for M1, docs/ledger/g3/
# 2026-11-13-l1-loader.md): every closure of the trace or the static bound,
# so that a branch GNU R did not take still finds its R function, with the
# file whose last top-level assignment defines it; and the primitives and
# internals of both, which the loader binds to a "not implemented yet"
# error where robigo-rt has no implementation.
both <- sort(union(traced, static), method = "radix")
bound_lines <- vapply(both, function(l) {
  kind <- sub(" .*", "", l)
  name <- sub("^\\S+ ", "", l)
  if (!kind %in% c("closure", "variable")) return(l)
  file <- get0(name, envir = defined_in, inherits = FALSE, ifnotfound = "?")
  if (file == "?") stop("no file of src/library/base/R defines the bound ", kind, " ", name)
  paste(l, file)
}, "")
bound_lines <- bound_lines[order(match(sub(" .*", "", bound_lines), c(names(counts), "variable")), bound_lines,
                                 method = "radix")]
writeLines(c(
  "# robigo-rt's base bound: what its loader binds (crates/robigo-rt/src/base.rs).",
  "# Generated by scripts/base-subset.sh with base-subset.txt; do not edit.",
  "#",
  "#   closure NAME FILE   bound to its definition: the last top-level assignment",
  "#                       to NAME in FILE of src/library/base/R (collation order)",
  "#   primitive NAME      bound in base; \"not implemented yet\" where robigo-rt has none",
  "#   internal NAME       an .Internal; the same",
  "#   variable NAME FILE  a base variable a bound closure names, bound as a closure is",
  "#",
  "# The union of base-subset.txt (the trace) and the static bound (the free",
  "# variables of the same inputs that name base functions, transitively through",
  "# base closures), so that a branch GNU R did not take still finds its function.",
  sprintf("# %d closures, %d primitives, %d internals, %d variables.", count(bound_lines, "closure"),
          count(bound_lines, "primitive"), count(bound_lines, "internal"), count(bound_lines, "variable")),
  bound_lines), bound_out)
message(sprintf("%s: %d closures, %d primitives, %d internals, %d variables", bound_out, count(bound_lines, "closure"),
                count(bound_lines, "primitive"), count(bound_lines, "internal"), count(bound_lines, "variable")))
