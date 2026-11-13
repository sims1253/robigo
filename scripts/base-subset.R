# The body of scripts/base-subset.sh, run by GNU R 4.6.2 in its oracle image
# from the repository root:
#
#   Rscript --vanilla scripts/base-subset.R WORK OUT
#
# WORK holds the pinned sources' src/library/base/R (as base-R/) and gets
# the build and run files; OUT is the subset file written. Each input runs
# twice, untraced and traced (scripts/base-subset-trace.R); the two must
# print the same bytes and exit the same way, or the tracer changed what it
# measures and the run fails. The capi cases' C is compiled here with
# GNU R 4.6.2's R CMD SHLIB.
args <- commandArgs(trailingOnly = TRUE)
if (length(args) != 2L) stop("usage: Rscript --vanilla scripts/base-subset.R WORK OUT")
work <- normalizePath(args[[1L]], mustWork = TRUE)
out <- args[[2L]]
root <- normalizePath(".")
if (getRversion() != "4.6.2") stop("base-subset.R needs GNU R 4.6.2, not ", getRversion())
Sys.setenv(LANGUAGE = "en", LC_ALL = "C.UTF-8", TZ = "UTC", R_DEFAULT_PACKAGES = "NULL")
Sys.unsetenv("TZDIR")

# The M1 scripts (docs/g3.md §14.2 M1; §15.3 E1, E2, E6).
inputs <- list(
  list(id = "calls", file = "benchmarks/suite/calls.R"),
  list(id = "fib", file = "benchmarks/suite/fib.R"),
  list(id = "binarytrees", file = "benchmarks/suite/binarytrees.R"),
  list(id = "capi-eval", file = "tests/corpora/capi/cases/eval.R", c = "tests/corpora/capi/support/eval.c"),
  list(id = "capi-pairlist", file = "tests/corpora/capi/cases/pairlist.R",
       c = "tests/corpora/capi/support/pairlist.c"),
  list(id = "capi-unwind", file = "tests/corpora/capi/cases/unwind.R", c = "tests/corpora/capi/support/unwind.c"),
  list(id = "capi-extptr", file = "tests/corpora/capi/cases/extptr.R", c = "tests/corpora/capi/support/extptr.c")
)

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
  if (st != 0L && any(grepl("SET_TYPEOF", readLines(log)))) {
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
mask <- function(x) {
  if (!is.raw(x)) return(format(x))
  gsub("(stack imbalance in '[^']*', )[0-9]+ then [0-9]+", "\\1N then M", rawToChar(x))
}

trace_r <- file.path(root, "scripts", "base-subset-trace.R")

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
  plain <- run(dir, script, "--vanilla")
  keys_file <- file.path(dir, "keys.txt")
  traced <- run(dir, script, c("--no-environ", "--no-site-file", "--no-save", "--no-restore"),
                env = c(paste0("R_PROFILE_USER=", shQuote(trace_r)), "R_ENABLE_JIT=0",
                        paste0("ROBIGO_SUBSET_OUT=", shQuote(keys_file))))
  for (part in c("out", "err", "status")) {
    a <- mask(plain[[part]])
    b <- mask(traced[[part]])
    if (!identical(a, b)) {
      writeLines(c(a, "--- traced", b), file.path(dir, paste0("mismatch-", part, ".txt")))
      stop(sprintf("%s: the traced run's %s differs from the untraced one's:\n--- untraced\n%s\n--- traced\n%s",
                   inp$file, part, a, b))
    }
  }
  if (plain$status != 0L) stop(inp$file, " failed under GNU R 4.6.2:\n", rawToChar(plain$err))
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
files <- c(Sys.glob(file.path(base_r, "*.R")), Sys.glob(file.path(base_r, "unix", "*.R")))
files <- files[order(files, method = "radix")]
defined_in <- new.env(parent = emptyenv())
for (f in files) {
  rel <- substring(f, nchar(base_r) + 2L)
  for (e in parse(f, keep.source = FALSE)) {
    if (is.call(e) && as.character(e[[1L]])[1L] %in% c("<-", "=") && length(e) == 3L) {
      lhs <- e[[2L]]
      if (is.symbol(lhs) || (is.character(lhs) && length(lhs) == 1L)) assign(as.character(lhs), rel, envir = defined_in)
    } else if (is.call(e) && identical(e[[1L]], as.name("assign")) && is.character(e[[2L]])) {
      assign(e[[2L]], rel, envir = defined_in)
    }
  }
}

# One line per key: KIND NAME [FILE] INPUTS.
all_keys <- sort(unique(unlist(reached)), method = "radix")
lines <- character()
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
    lines <- c(lines, paste("closure", name, file, by))
  } else {
    lines <- c(lines, paste(kind, name, by))
  }
  counts[[kind]] <- counts[[kind]] + 1L
}
ord <- order(match(sub(" .*", "", lines), names(counts)), lines, method = "radix")
header <- c(
  "# robigo-rt's base subset: what GNU R 4.6.2 reaches of base running M1's",
  "# scripts (docs/g3.md §14.2 M1). Generated by scripts/base-subset.sh from the",
  "# pinned 4.6.2 tarball and oracle image; do not edit.",
  "#",
  "#   closure NAME FILE INPUTS   a base closure entered; FILE is the file of",
  "#                              src/library/base/R that defines it (collation:",
  "#                              LC_COLLATE=C ls R/*.R R/unix/*.R)",
  "#   primitive NAME INPUTS      a primitive called, or passed to a higher-order function",
  "#   internal NAME INPUTS       an .Internal called",
  "#",
  "# INPUTS names the scripts that reach it:",
  vapply(inputs, function(i) sprintf("#   %-14s %s", i$id, i$file), ""),
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
if (length(unresolved)) {
  stop("no file of src/library/base/R defines these reached closures: ", paste(unresolved, collapse = ", "))
}
message(sprintf("%s: %d closures, %d primitives, %d internals", out, counts[["closure"]],
                counts[["primitive"]], counts[["internal"]]))

# For comparison only (docs/ledger/g3/2026-11-13-rt-conformance.md): the
# static bound, as m1-plan's estimate made it with codetools::findGlobals
# (which the image lacks): the free variables of the same inputs that name
# base functions, transitively through base closures' bodies and defaults.
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
    if (n %in% c("<-", "=", "for")) {
      tgt <- e[[2L]]
      while (is.call(tgt)) tgt <- tgt[[2L]]
      if (is.symbol(tgt)) locals <<- c(locals, as.character(tgt))
    }
    for (i in seq_along(e)) if (!identical(e[[i]], quote(expr = ))) walk(e[[i]])
  }
  walk(e)
  list(acc = unique(acc), ints = unique(ints))
}
exprs <- unlist(lapply(inputs, function(i) as.list(parse(file.path(root, i$file), keep.source = FALSE))),
                recursive = FALSE)
g <- free(as.call(c(as.name("{"), exprs)), character())
todo <- g$acc
ints <- g$ints
seen <- character()
prims <- character()
while (length(todo)) {
  n <- todo[[1L]]
  todo <- todo[-1L]
  if (n %in% seen || !exists(n, envir = baseenv(), inherits = FALSE)) next
  f <- get(n, envir = baseenv())
  if (!is.function(f)) next
  seen <- c(seen, n)
  if (is.primitive(f)) {
    prims <- c(prims, n)
    next
  }
  fl <- formals(f)
  for (x in c(list(body(f)), Filter(function(d) !identical(d, quote(expr = )), as.list(fl)))) {
    gg <- free(x, names(fl))
    todo <- c(todo, setdiff(gg$acc, seen))
    ints <- c(ints, gg$ints)
  }
}
static <- sort(c(paste("closure", setdiff(seen, prims)), paste("primitive", prims), paste("internal", unique(ints))),
               method = "radix")
traced <- sort(sub("^(\\S+ \\S+).*", "\\1", lines), method = "radix")
writeLines(static, file.path(work, "static.txt"))
writeLines(setdiff(traced, static), file.path(work, "traced-only.txt"))
writeLines(setdiff(static, traced), file.path(work, "static-only.txt"))
count <- function(x, kind) sum(startsWith(x, paste0(kind, " ")))
message(sprintf("static bound: %d closures, %d primitives, %d internals; %d in both; %s/{static,traced-only,static-only}.txt",
                count(static, "closure"), count(static, "primitive"), count(static, "internal"),
                length(intersect(static, traced)), work))
