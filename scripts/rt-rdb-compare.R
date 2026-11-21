# Compare lazy-load databases record by record, run by GNU R 4.6.2 (scripts/
# rt-rdb-compare.sh): each database robigo-rt made in the base build
# (scripts/rt-base-build.sh) against the one GNU R's own build made (the
# oracle image's library).
#   Rscript scripts/rt-rdb-compare.R [--map FROM=TO ...] OURS_LIBRARY GNUR_LIBRARY [DB ...]
# DB is PKG/NAME (base/base, tools/tools, tools/sysdata); the default is
# every .rdb under OURS_LIBRARY.
#
# Per database:
# 1. the .rdx maps: the compression, the variables' and references' keys
#    (sets; references are numbered as they are written, so their keys
#    match only when the writes did; a reference may be an eager record and
#    lazy ones, lazyload.R's `eagerKey`/`lazyKeys`, each compared as
#    `KEY` and `KEY$NAME`);
# 2. each record both maps name, by key, decompressed as lazyLoadDBfetch
#    does (connections.c R_decompress1/2/3; compression 3's raw LZMA2,
#    type 'Z', is refused: R cannot decode it), compared byte for byte;
# 3. a record whose bytes differ, unserialized on both sides (a reference
#    to another record kept as its key, so a record is compared by itself)
#    and put in a class:
#    every comparison is identical() with num.eq = FALSE (0 and -0, the
#    NaN payloads apart), and a reference is a marker no value can equal;
#    - "value": identical(), byte code and srcrefs included (the bytes
#      are not: the writer's flags, order or reference numbering differ);
#    - "bytecode": identical() but for byte code (or srcrefs): one side's
#      closures are compiled and the other's are not, or differently;
#    - "path": identical(), byte code and srcrefs included, once each
#      --map's FROM (a directory of our build: its source or install tree)
#      is replaced by its TO (GNU R's) in our strings;
#    - "env": identical() once every closure is taken as its formals, body
#      and attributes, kept apart from a list by a class: closures'
#      environments aside;
#    - "differ": anything else;
#    - "unreadable": GNU R cannot unserialize ours (a promise as the
#      record's value, an internal GNU R does not have), counted by the
#      reason.
# Exits 0 when the maps and every record's bytes agree, 1 when anything
# differs, 2 on a setup error (no database, a file missing, a map that
# cannot be read, a compression it cannot decode).

args <- commandArgs(trailingOnly = TRUE)
maps <- list()
while (length(args) && args[[1L]] == "--map") {
    m <- regmatches(args[[2L]], regexpr("=", args[[2L]]), invert = TRUE)[[1L]]
    maps[[length(maps) + 1L]] <- m
    args <- args[-(1:2)]
}
if (length(args) < 2L) {
    cat("usage: rt-rdb-compare.R OURS_LIBRARY GNUR_LIBRARY [PKG/NAME ...]\n", file = stderr())
    quit(status = 2L)
}
ours_lib <- args[[1L]]
gnur_lib <- args[[2L]]
dbs <- args[-(1:2)]
if (!length(dbs)) {
    f <- list.files(ours_lib, pattern = "[.]rdb$", recursive = TRUE)
    dbs <- sub("/R/", "/", sub("[.]rdb$", "", f), fixed = TRUE)
}
fail <- function(...) { cat("rt-rdb-compare: ", ..., "\n", sep = "", file = stderr()); quit(status = 2L) }
if (!length(dbs)) fail("no database to compare under ", ours_lib)

# lazyLoadDBfetch's decompression of one record: compression TRUE (1) is
# R_decompress1 (zlib after the 4-byte length); 2 and 3 are
# R_decompress2/3, whose fifth byte names the method: '2' bzip2, '1' zlib,
# '0' stored, and for 3 also 'Z' raw LZMA2 (refused)
decompress <- function(b, compressed) {
    if (isFALSE(compressed) || identical(as.numeric(compressed), 0)) return(b)
    if (compressed %in% c(2, 3)) {
        type <- rawToChar(b[5L])
        body <- b[-(1:5)]
        if (type == "Z" && compressed == 3)
            fail("compression 3's raw LZMA2 records (type 'Z') are not decoded here")
        return(switch(type,
            "2" = memDecompress(body, "bzip2"), "1" = memDecompress(body, "gzip"), "0" = body,
            fail("record compression type '", type, "' is not lazyLoadDBfetch's")))
    }
    memDecompress(b[-(1:4)], "gzip")
}

# the records a map names, by label: each variable; each reference's
# record, or its eager record and lazy ones (`KEY$NAME`)
entries <- function(map) {
    out <- map$variables
    names(out) <- paste0("variables ", names(out))
    for (k in names(map$references)) {
        e <- map$references[[k]]
        if (is.list(e)) {
            out[[paste0("references ", k)]] <- e$eagerKey
            for (n in names(e$lazyKeys)) out[[paste0("references ", k, "$", n)]] <- e$lazyKeys[[n]]
        } else out[[paste0("references ", k)]] <- e
    }
    out
}

same <- function(x, y, ...) identical(x, y, num.eq = FALSE, ...)
strict <- function(x, y) same(x, y, ignore.bytecode = FALSE, ignore.srcref = FALSE)

# every closure as its formals, body and attributes, apart from a list
nzt <- function(x) {
    if (is.function(x) && !is.primitive(x))
        structure(list(formals(x), body(x), lapply(attributes(x), nzt)), class = "rt-rdb-closure")
    else if (is.list(x)) { a <- attributes(x); r <- lapply(x, nzt); attributes(r) <- a; r }
    else x
}

# a database: its map, and a reader of its records by key
open_db <- function(lib, db) {
    parts <- strsplit(db, "/", fixed = TRUE)[[1L]]
    base <- file.path(lib, parts[[1L]], "R", parts[[2L]])
    for (ext in c(".rdb", ".rdx")) if (!file.exists(paste0(base, ext))) fail(paste0(base, ext), " does not exist")
    map <- tryCatch(readRDS(paste0(base, ".rdx")), error = function(e) fail(base, ".rdx: ", conditionMessage(e)))
    con <- file(paste0(base, ".rdb"), "rb")
    rec <- function(pos) {
        seek(con, pos[[1L]])
        decompress(readBin(con, "raw", pos[[2L]]), map$compressed)
    }
    list(map = map, rec = rec, close = function() close(con))
}

# our value with our build's directories replaced by GNU R's (strings, in
# lists and attributes)
mapped <- function(x) {
    if (is.character(x)) {
        a <- attributes(x)
        for (m in maps) x <- gsub(m[[1L]], m[[2L]], x, fixed = TRUE)
        attributes(x) <- a
        x
    } else if (is.list(x)) {
        a <- attributes(x)
        x <- lapply(x, mapped)
        attributes(x) <- a
        x
    } else x
}

# a reference to another record: its key, marked with this run's token (a
# value cannot hold it by chance)
token <- paste(sample(c(0:9, letters[1:6]), 32L, TRUE), collapse = "")
ref <- function(key) structure(key, class = "rt-rdb-reference", token = token)

# a record's object, its references to others kept as their keys; an
# unreadable record's reason instead (a character with class "unreadable")
value <- function(bytes) {
    w <- NULL
    v <- withCallingHandlers(
        tryCatch(unserialize(bytes, refhook = ref),
                 error = function(e) structure(conditionMessage(e), class = "unreadable")),
        warning = function(c) { w <<- conditionMessage(c); invokeRestart("muffleWarning") })
    if (!is.null(w) && !inherits(v, "unreadable")) v <- structure(w, class = "unreadable")
    v
}

total_differ <- 0L
for (db in dbs) {
    ours <- open_db(ours_lib, db)
    gnur <- open_db(gnur_lib, db)
    cat("== ", db, "\n", sep = "")
    # 1. the maps
    cat("compressed: ours ", format(ours$map$compressed), ", GNU R ", format(gnur$map$compressed), "\n", sep = "")
    if (!identical(ours$map$compressed, gnur$map$compressed)) total_differ <- total_differ + 1L
    eo <- entries(ours$map); eg <- entries(gnur$map)
    for (part in c("variables", "references")) {
        o <- grep(paste0("^", part, " "), names(eo), value = TRUE)
        g <- grep(paste0("^", part, " "), names(eg), value = TRUE)
        cat(part, ": ", length(o), " ours, ", length(g), " GNU R", sep = "")
        miss <- sub("^[a-z]+ ", "", setdiff(g, o)); extra <- sub("^[a-z]+ ", "", setdiff(o, g))
        if (length(miss)) cat("; missing ", length(miss), ": ", paste(head(miss, 8L), collapse = " "), sep = "")
        if (length(extra)) cat("; extra ", length(extra), ": ", paste(head(extra, 8L), collapse = " "), sep = "")
        cat("\n")
        total_differ <- total_differ + length(miss) + length(extra)
    }
    # 2. and 3. the records, by GNU R's keys
    classes <- c(same = 0L, value = 0L, bytecode = 0L, path = 0L, env = 0L, differ = 0L, unreadable = 0L)
    compiled <- c(ours = 0L, gnur = 0L)
    reasons <- list()
    shown <- 0L
    {
        for (k in intersect(names(eg), names(eo))) {
            a <- ours$rec(eo[[k]])
            b <- gnur$rec(eg[[k]])
            if (identical(a, b)) { classes[["same"]] <- classes[["same"]] + 1L; next }
            va <- value(a); vb <- value(b)
            cls <- if (inherits(va, "unreadable")) {
                # the reason without the record's own names
                r <- sub("\"[^\"]*\"", "\"...\"", unclass(va))
                reasons[[r]] <- c(reasons[[r]], k)
                "unreadable"
            } else if (strict(va, vb)) "value"
            else if (same(va, vb)) {
                bc <- function(f) is.function(f) && !is.primitive(f) && typeof(.Internal(bodyCode(f))) == "bytecode"
                compiled <- compiled + c(bc(va), bc(vb))
                "bytecode"
            } else if (length(maps) && strict(mapped(va), vb)) "path"
            else if (same(nzt(va), nzt(vb))) "env" else "differ"
            classes[[cls]] <- classes[[cls]] + 1L
            if (cls == "differ" && shown < 20L) {
                shown <- shown + 1L
                at <- which(a[seq_len(min(length(a), length(b)))] != b[seq_len(min(length(a), length(b)))])[1L]
                cat("differ: ", k, " (", length(a), " bytes ours, ", length(b), " GNU R; first at ",
                    if (is.na(at)) min(length(a), length(b)) + 1L else at, ")\n", sep = "")
                # a short value, both sides
                one <- function(v) paste(deparse(v, nlines = 3L), collapse = " ")
                if (length(a) + length(b) < 2000L)
                    cat("  ours:  ", substr(one(va), 1L, 200L), "\n  GNU R: ", substr(one(vb), 1L, 200L), "\n", sep = "")
            }
        }
    }
    cat("records: ", classes[["same"]], " the same bytes, ", classes[["value"]], " the same value, ",
        classes[["bytecode"]], " the same but for byte code (compiled: ", compiled[["ours"]], " ours, ",
        compiled[["gnur"]], " GNU R), ", classes[["path"]], " the same but for build paths, ",
        classes[["env"]], " closures' environments aside, ",
        classes[["differ"]], " differ, ", classes[["unreadable"]], " unreadable\n", sep = "")
    for (r in names(reasons))
        cat("unreadable (", length(reasons[[r]]), "): ", r, ": ", paste(head(reasons[[r]], 6L), collapse = " "), "\n", sep = "")
    total_differ <- total_differ + sum(classes[-1L])
    ours$close(); gnur$close()
}
quit(status = if (total_differ) 1L else 0L)
