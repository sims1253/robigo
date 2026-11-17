# INTERIM(L5): robigo-rt's wide base index (crates/robigo-rt/proto/base-wide.txt),
# run by GNU R 4.6.2 in its oracle image from the repository root, by
# scripts/base-subset.sh:
#
#   Rscript --vanilla scripts/base-wide.R SRC OUT
#
# SRC holds the pinned sources' src/library (base/R, base/baseloader.R,
# profile/); OUT is the index written. L5's boot from GNU R-built databases
# replaces the loader that reads it, and deletes it with base-subset.sh.
#
# What GNU R 4.6.2 binds in base, and where each binding comes from:
#
# - base's R files in collation order (LC_COLLATE=C ls R/*.R R/unix/*.R,
#   base's Makefile), as the build sources their concatenation into base;
# - baseloader.R, the installed base file: its forms after the lazy load;
# - the system profile (Common.R, then Rprofile.unix), which R_LoadProfile
#   evaluates in base at startup.
#
# robigo-rt binds each name lazily to the form that defines it, evaluated in
# the base namespace on first use. The index says which form that is, by its
# byte range in its file (the srcref's), so that the loader reads and parses
# nothing at startup. Every form that is not an assignment to a base name is
# classified below; a form this script does not know fails it, so a new
# tarball's base cannot be indexed silently wrong.
args <- commandArgs(trailingOnly = TRUE)
stopifnot(length(args) == 2L)
src <- args[[1L]]
out <- args[[2L]]

base_names <- ls(baseenv(), all.names = TRUE)
is_prim <- vapply(base_names, function(n) is.primitive(get(n, envir = baseenv())), NA)

rfiles <- c(sort(Sys.glob(file.path(src, "base/R/*.R")), method = "radix"),
            sort(Sys.glob(file.path(src, "base/R/unix/*.R")), method = "radix"))
sources <- c(rfiles, file.path(src, c("base/baseloader.R", "profile/Common.R", "profile/Rprofile.unix")))
rel <- function(f) substring(f, nchar(src) + 2L)

# the byte range [start, end) of the srcref `sr` in a file whose lines start
# at the byte offsets `starts` (0-based)
byte_range <- function(sr, starts) {
    sr <- as.integer(sr)
    c(starts[sr[1L]] + sr[2L] - 1L, starts[sr[3L]] + sr[4L])
}

is_asg <- function(e) {
    is.call(e) && is.name(e[[1L]]) && as.character(e[[1L]]) %in% c("<-", "=") && length(e) == 3L &&
        (is.name(e[[2L]]) || is.character(e[[2L]]))
}

# The forms that are not assignments to a base name, by file and form number:
#   none            runs, binds no base name (nothing to do lazily)
#   after NAME      runs right after NAME's definition, on its first fetch
#   block           its names are those it assigns into the base namespace
#                   (found by running it on a scratch environment): it is
#                   their definition, run once on the first fetch of any
#   skip WHY        not run (the record the loader keeps and reports)
known <- list(
    "base/R/parse.R#2" = "none",  # if (FALSE) { ... }
    "base/R/zzz.R#114" = "block",  # the deprecated as.data.frame.<class> methods
    "base/baseloader.R#1" = "none",  # the lazy load itself
    "base/baseloader.R#4" = "block",  # .C_* and .F_* native symbols
    "base/baseloader.R#5" = "none", "base/baseloader.R#6" = "none", "base/baseloader.R#7" = "none",
    "base/baseloader.R#8" = "none", "base/baseloader.R#9" = "none",  # force()s
    "base/baseloader.R#10" = "after .__S3MethodsTable__.",  # base's S3 methods registered
    "base/baseloader.R#11" = "after .ArgsEnv", "base/baseloader.R#12" = "after .GenericArgsEnv",
    "profile/Common.R#2" = "skip the Autoloads environment: robigo-rt's search path is its own",
    "profile/Common.R#4" = "after .AutoloadEnv",  # assign(".Autoloaded", NULL, envir = .AutoloadEnv)
    "profile/Common.R#10" = "none", "profile/Common.R#11" = "none", "profile/Common.R#12" = "none",
    "profile/Common.R#13" = "none", "profile/Common.R#14" = "none", "profile/Common.R#15" = "none",
    "profile/Common.R#16" = "none", "profile/Common.R#17" = "none", "profile/Common.R#18" = "none",
    "profile/Common.R#19" = "none", "profile/Common.R#20" = "none", "profile/Common.R#21" = "none",
    "profile/Common.R#22" = "skip R_LIBS_SITE and R_LIBS_USER are not expanded",
    "profile/Common.R#23" = "skip R_LIBS_SITE and R_LIBS_USER are not expanded",
    "profile/Common.R#24" = "none", "profile/Common.R#27" = "none", "profile/Common.R#28" = "none",
    "profile/Common.R#30" = "skip .Library.site is a value, not GNU R's active binding (in base and the global environment)",
    "profile/Rprofile.unix#5" = "skip .libPaths() is not initialized from R_LIBS_USER",
    "profile/Rprofile.unix#6" = "block",  # .popath
    "profile/Rprofile.unix#7" = "none", "profile/Rprofile.unix#8" = "none",
    "profile/Rprofile.unix#9" = "none", "profile/Rprofile.unix#10" = "none")
# (`none` forms of the profile set options and environment variables, which
# robigo-rt's init_options and its startup make as the profile does; the
# `if`s guard what does not apply: interactive sessions, R_BATCH, macOS)

# Names whose definition needs a form that is not run: bound to raise, with
# that form's record, when touched.
unrun <- c(.AutoloadEnv = "profile/Common.R#2")

# The names a block assigns into the base namespace, found by running it with
# .BaseNamespaceEnv bound to a scratch environment.
block_names <- function(e) {
    scratch <- new.env(hash = TRUE)
    eval(e, list2env(list(.BaseNamespaceEnv = scratch), parent = .BaseNamespaceEnv))
    sort(ls(scratch, all.names = TRUE), method = "radix")
}

defs <- list(); others <- list(); seen <- character()
for (f in sources) {
    bytes <- readBin(f, "raw", file.info(f)$size)
    starts <- c(0L, which(bytes == as.raw(10L)))
    ex <- parse(f, keep.source = TRUE, encoding = "UTF-8")
    srs <- attr(ex, "srcref")
    for (i in seq_along(ex)) {
        e <- ex[[i]]
        r <- byte_range(srs[[i]], starts)
        key <- paste0(rel(f), "#", i)
        if (is_asg(e)) {
            depth <- 0L
            while (is_asg(e)) {
                defs[[length(defs) + 1L]] <- data.frame(name = as.character(e[[2L]]), src = rel(f), start = r[1L],
                                                        end = r[2L], depth = depth, form = i)
                e <- e[[3L]]
                depth <- depth + 1L
            }
            next
        }
        # assign(NAME, VALUE, envir = TARGET) into one of base's argument
        # environments: after TARGET. Any other assign() is classified by
        # `known` below, or stops there as unclassified.
        if (is.call(e) && identical(e[[1L]], as.name("assign")) && is.name(e$envir) &&
            as.character(e$envir) %in% c(".ArgsEnv", ".GenericArgsEnv")) {
            others[[length(others) + 1L]] <- data.frame(kind = "after", name = as.character(e$envir), src = rel(f),
                                                        start = r[1L], end = r[2L], why = "", form = i)
            next
        }
        k <- known[[key]]
        if (is.null(k)) stop("an unclassified top-level form: ", key, ": ",
                             substr(paste(deparse(e), collapse = " "), 1L, 200L))
        seen <- c(seen, key)
        if (k == "none") next
        if (k == "block") {
            for (n in block_names(e))
                others[[length(others) + 1L]] <- data.frame(kind = "block", name = n, src = rel(f), start = r[1L],
                                                            end = r[2L], why = "", form = i)
        } else if (startsWith(k, "after ")) {
            others[[length(others) + 1L]] <- data.frame(kind = "after", name = substring(k, 7L), src = rel(f),
                                                        start = r[1L], end = r[2L], why = "", form = i)
        } else if (startsWith(k, "skip ")) {
            others[[length(others) + 1L]] <- data.frame(kind = "skip", name = "", src = rel(f), start = r[1L],
                                                        end = r[2L], why = substring(k, 6L), form = i)
        } else stop("bad classification ", k)
    }
}
stale <- setdiff(names(known), seen)
if (length(stale)) stop("classified forms that are no longer there: ", paste(stale, collapse = ", "))
defs <- do.call(rbind, defs)
others <- do.call(rbind, others)

# a name's binding: its last definition, unless a block assigns it later
# (zzz.R's and the loader's come after every base file)
blocks <- others[others$kind == "block", ]
lines <- character()
for (n in unique(c(defs$name, blocks$name))) {
    d <- defs[defs$name == n, ]
    b <- blocks[blocks$name == n, ]
    if (n %in% names(unrun)) {
        k <- others[others$kind == "skip" & paste0(others$src, "#", others$form) == unrun[[n]], ]
        stopifnot(nrow(k) == 1L)
        lines <- c(lines, sprintf("unrun %s %s %d %d", n, k$src, k$start, k$end))
        next
    }
    if (nrow(b)) {
        # (every block comes after the files that define its names first)
        stopifnot(!nrow(d) || match(b$src[1L], rel(sources)) > max(match(d$src, rel(sources))))
        lines <- c(lines, sprintf("block %s %s %d %d", n, b$src[nrow(b)], b$start[nrow(b)], b$end[nrow(b)]))
        next
    }
    # a redefinition that reads the name (the profile's .Library.site) runs
    # every definition in order
    last <- d[nrow(d), ]
    seq <- nrow(d) > 1L && local({
        p <- parse(text = rawToChar(readBin(file.path(src, last$src), "raw", 1e7)[(last$start + 1L):last$end]),
                   keep.source = FALSE)[[1L]]
        for (k in seq_len(last$depth)) p <- p[[3L]]
        n %in% all.names(p[[3L]])
    })
    if (seq) {
        for (k in seq_len(nrow(d)))
            lines <- c(lines, sprintf("def %s %s %d %d %d seq", n, d$src[k], d$start[k], d$end[k], d$depth[k]))
    } else {
        lines <- c(lines, sprintf("def %s %s %d %d %d", n, last$src, last$start, last$end, last$depth))
    }
}
for (k in which(others$kind == "after" & !others$name %in% names(unrun)))
    lines <- c(lines, sprintf("after %s %s %d %d", others$name[k], others$src[k], others$start[k], others$end[k]))
for (k in which(others$kind == "skip"))
    lines <- c(lines, sprintf("skip %s %d %d %s", others$src[k], others$start[k], others$end[k], others$why[k]))

# what GNU R's C binds at startup
bound <- unique(c(defs$name, blocks$name))
from_c <- setdiff(base_names[!is_prim], bound)
for (n in from_c) lines <- c(lines, sprintf("c %s", n))

# base's Makefile substitutes configure's `which` for @WHICH@ (mkRbase's
# sed): the installed Sys.which's
which <- grep("^/", unlist(lapply(as.list(body(Sys.which)), function(e)
    if (is.call(e) && identical(e[[1L]], as.name("<-")) && identical(e[[2L]], as.name("which"))) e[[3L]])), value = TRUE)
stopifnot(length(which) == 1L)
lines <- c(lines, sprintf("subst @WHICH@ %s", which))

prims <- sort(base_names[is_prim], method = "radix")
internals <- sort(setdiff(builtins(internal = TRUE), base_names[is_prim]), method = "radix")
lines <- c(lines, sprintf("primitive %s", prims), sprintf("internal %s", internals))

# every name GNU R binds in base is in the index
covered <- c(bound, from_c, prims)
missing <- setdiff(base_names, covered)
if (length(missing)) stop("names of base the index does not cover: ", paste(missing, collapse = " "))

ndef <- length(unique(defs$name[!defs$name %in% blocks$name]))
header <- c(
    "# INTERIM(L5): robigo-rt's wide base, what its loader binds (crates/robigo-rt/src/base.rs).",
    "# Generated by scripts/base-subset.sh (scripts/base-wide.R) on GNU R 4.6.2; do not edit.",
    "# SRC is a file under the pinned tarball's src/library; START END its bytes [START, END).",
    "#",
    "#   def NAME SRC START END DEPTH [seq]  NAME's binding: the DEPTH-th assignment (0 the outer) of the",
    "#                                       chain the form is, its right-hand side evaluated in the base",
    "#                                       namespace on first use; `seq`: every such line runs, in order",
    "#   block NAME SRC START END            NAME's binding: what the form (a local()) assigns, run once",
    "#   after NAME SRC START END            run right after NAME's definition, on its first use",
    "#   skip SRC START END WHY              not run",
    "#   unrun NAME SRC START END            NAME's definition needs that skipped form: raises when touched",
    "#   c NAME                              bound by GNU R's C at startup (robigo-rt's init binds it)",
    "#   subst FROM TO                       a configure substitution in base's sources (mkRbase's sed)",
    "#   primitive NAME                      a primitive; \"not implemented yet\" where robigo-rt has none",
    "#   internal NAME                       an .Internal; the same",
    "#",
    sprintf("# %d names: %d definitions, %d of blocks, %d from C, %d primitives; %d internals.",
            length(base_names), ndef, length(unique(blocks$name)), length(from_c), length(prims),
            length(internals)))
writeLines(c(header, lines), out)
