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
#    match only when the writes did);
# 2. each record GNU R's map names, by key, decompressed as
#    lazyLoadDBfetch does (serialize.c, connections.c R_decompress1/2/3),
#    compared byte for byte;
# 3. a record whose bytes differ, unserialized on both sides (a reference
#    to another record kept as its key, so a record is compared by itself)
#    and put in a class:
#    - "value": the objects are identical(), byte code and srcrefs
#      included (the bytes are not: the writer's flags, order or reference
#      numbering differ);
#    - "bytecode": identical() but for byte code (or srcrefs): one side's
#      closures are compiled and the other's are not, or differently;
#    - "path": they agree once each --map's FROM (a directory of our
#      build: its source or install tree) is replaced by its TO (GNU R's)
#      in our strings;
#    - "env": they agree once every closure is taken as its formals, body
#      and attributes (scripts/lazyload-ck.R's nz): closures' environments
#      aside;
#    - "differ": anything else;
#    - "unreadable": GNU R cannot unserialize ours (a promise as the
#      record's value, an internal GNU R does not have), counted by the
#      reason.
# Exits 0 when every record's bytes agree, 1 when any record differs, 2 on
# a setup error (a database missing, a map that cannot be read).

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
here <- dirname(sub("^--file=", "", grep("^--file=", commandArgs(), value = TRUE)[1L]))
source(file.path(here, "lazyload-ck.R"))

fail <- function(...) { cat("rt-rdb-compare: ", ..., "\n", sep = "", file = stderr()); quit(status = 2L) }

# lazyLoadDBfetch's decompression of one record
decompress <- function(b, compressed) {
    if (identical(compressed, FALSE) || identical(compressed, 0L) || identical(compressed, 0)) return(b)
    if (compressed == 3) {
        type <- rawToChar(b[5L])
        body <- b[-(1:5)]
        return(switch(type,
            Z = memDecompress(body, "gzip"), "2" = memDecompress(body, "bzip2"),
            X = memDecompress(body, "xz"), "0" = body,
            fail("record compression type '", type, "' is not handled")))
    }
    memDecompress(b[-(1:4)], if (compressed == 2) "bzip2" else "gzip")
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

# a record's object, its references to others kept as their keys; an
# unreadable record's reason instead (a character with class "unreadable")
value <- function(bytes) {
    w <- NULL
    v <- withCallingHandlers(
        tryCatch(unserialize(bytes, refhook = function(key) paste0("<ref ", key, ">")),
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
    for (part in c("variables", "references")) {
        o <- names(ours$map[[part]]); g <- names(gnur$map[[part]])
        cat(part, ": ", length(o), " ours, ", length(g), " GNU R", sep = "")
        if (length(setdiff(g, o))) cat("; missing ", length(setdiff(g, o)), ": ", paste(head(setdiff(g, o), 8L), collapse = " "), sep = "")
        if (length(setdiff(o, g))) cat("; extra ", length(setdiff(o, g)), ": ", paste(head(setdiff(o, g), 8L), collapse = " "), sep = "")
        cat("\n")
    }
    # 2. and 3. the records, by GNU R's keys
    classes <- c(same = 0L, value = 0L, bytecode = 0L, path = 0L, env = 0L, differ = 0L, unreadable = 0L)
    compiled <- c(ours = 0L, gnur = 0L)
    reasons <- list()
    shown <- 0L
    for (part in c("variables", "references")) {
        keys <- intersect(names(gnur$map[[part]]), names(ours$map[[part]]))
        for (k in keys) {
            a <- ours$rec(ours$map[[part]][[k]])
            b <- gnur$rec(gnur$map[[part]][[k]])
            if (identical(a, b)) { classes[["same"]] <- classes[["same"]] + 1L; next }
            va <- value(a); vb <- value(b)
            cls <- if (inherits(va, "unreadable")) {
                # the reason without the record's own names
                r <- sub("\"[^\"]*\"", "\"...\"", unclass(va))
                reasons[[r]] <- c(reasons[[r]], k)
                "unreadable"
            } else if (identical(va, vb, ignore.bytecode = FALSE, ignore.srcref = FALSE)) "value"
            else if (identical(va, vb)) {
                bc <- function(f) is.function(f) && !is.primitive(f) && typeof(.Internal(bodyCode(f))) == "bytecode"
                compiled <- compiled + c(bc(va), bc(vb))
                "bytecode"
            } else if (length(maps) && identical(mapped(va), vb)) "path"
            else if (identical(nz(va), nz(vb))) "env" else "differ"
            classes[[cls]] <- classes[[cls]] + 1L
            if (cls == "differ" && shown < 20L) {
                shown <- shown + 1L
                at <- which(a[seq_len(min(length(a), length(b)))] != b[seq_len(min(length(a), length(b)))])[1L]
                cat("differ: ", part, " ", k, " (", length(a), " bytes ours, ", length(b), " GNU R; first at ",
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
