## A4's driver (docs/verification.md, "Builtin differential"): runs one
## batch's cases in GNU R 4.6.2 and in robigo-rt alike, and writes one record
## per case, `<A4_OUT>/<id>.rds` (saveRDS, version 3, uncompressed), whose
## bytes scripts/builtin-diff.py compares. A record is
##   list(value, visible, conds, error, env, globals, seed)
## conds: each warning and message in order, list(type, message, class,
##   call); error: list(message, class, call) or NULL; env: the case
##   environment's bindings after the call (the arguments a<i> and their
##   aliases alias<i>, bound to the same objects before it); globals: the
##   global environment's names; seed: .Random.seed after the call, or NULL;
##   log: what the fixture closures of a Calls row logged (.a4log), in order.
## A binding the call left as it was (its serialization unchanged) is
## recorded as list(unchanged = TRUE), any other as list(unchanged = FALSE,
## value).
## A value (the call's, or a changed binding's) whose serialization would
## be over A4_BIG_BYTES, made only of vectors and lists (no reference
## object, which a separate serialization could not share with the rest of
## the record), is recorded as its summary,
##   list(a4_big = k, type, length, attributes (their names), head, tail)
## (head and tail: its first and last 3 elements, of a vector or a list,
## when they are small), and the whole value goes to `<id>.big<k>.rds`
## (saveRDS version 2: plain vectors, never an ALTREP or compact form, so
## the same bytes on both runtimes), which the harness replaces by its
## sha256 at once; `<id>.big<k>.want` says the summary is there, so its
## digest is required on both sides. A value over A4_BIG_MAX_BYTES, or one the runtime cannot
## write that way, stays in the record. One case
## (`[<-`(a4cls, 1:1e5, long): 100,000 copies of a 10,000-byte string,
## shared in memory, each written out by serialize) recorded 1 GB. Each case's printed output follows a marker line on stdout,
## "\001A4 <id>\001", which the harness compares too.
##
## Parameters come from the environment (robigo-rt has no commandArgs yet):
##   A4_BATCH  the batch.rds gen.R wrote    A4_OUT    the records' directory
##   A4_FROM   the first case's index (1)    A4_LABEL  the runtime's name
##   A4_TO     the last case's index (an External row runs one per process)
##
## The driver uses only: readRDS, saveRDS, tryCatch, withCallingHandlers,
## withVisible, eval, as.call, as.name, new.env, assign, get0, ls, rm,
## serialize, unserialize, identical, RNGkind, set.seed, sprintf, Sys.getenv, stop;
## for a big value nchar(type = "bytes"), typeof, attributes, .subset,
## unclass, is.object, file.exists. The self-test checks them
## first; a runtime that fails it gets "driver unusable on <label>: <what>"
## and no records, never a wrong comparison.
local({
    A4_BIG_BYTES <- 16 * 1024^2       # a value over this goes to a sidecar, as its digest
    A4_BIG_MAX_BYTES <- 1024^3        # one over this stays whole (it may not expand under the cap)
    A4_ENDS_BYTES <- 1024^2           # a summary's first and last elements, at most
    batch_path <- Sys.getenv("A4_BATCH")
    out <- Sys.getenv("A4_OUT")
    from <- as.integer(Sys.getenv("A4_FROM", "1"))
    label <- Sys.getenv("A4_LABEL", "this R")
    unusable <- function(what) stop(paste0("driver unusable on ", label, ": ", what), call. = FALSE)
    check <- function(what, ok) if (!isTRUE(tryCatch(ok, error = function(e) FALSE))) unusable(what)

    ## ---- self-test: the driver's contract ---------------------------------
    check("withVisible", identical(withVisible(invisible(1))$visible, FALSE) &&
                         identical(withVisible(1)$visible, TRUE))
    check("tryCatch", identical(tryCatch(stop("x"), error = function(e) conditionMessage(e)), "x"))
    check("withCallingHandlers", {
        seen <- character()
        withCallingHandlers({ warning("w1"); message("m"); warning("w2") },
            warning = function(w) { seen <<- c(seen, conditionMessage(w)); invokeRestart("muffleWarning") },
            message = function(m) { seen <<- c(seen, conditionMessage(m)); invokeRestart("muffleMessage") })
        identical(seen, c("w1", "m\n", "w2"))
    })
    check("saveRDS/readRDS", {
        f <- file.path(out, "selftest.rds")
        saveRDS(list(1L, "a"), f, version = 3L, compress = FALSE)
        r <- identical(readRDS(f), list(1L, "a")); rm(f); r
    })
    check("RNGkind/set.seed", {
        RNGkind("Mersenne-Twister", "Inversion", "Rejection"); set.seed(1L)
        length(get0(".Random.seed", globalenv(), inherits = FALSE)) == 626L
    })
    check("as.call/eval", {
        e <- new.env(parent = baseenv()); assign("a1", 2, envir = e)
        f <- eval(as.call(list(as.name("function"), NULL, quote(.Internal(nchar(a1, "chars", FALSE, NA))))), e)
        identical(f(), 1L)
    })
    ## (no vector of length 3: the planted divergences, robigo-rtbuiltins'
    ## diff-plant, change results of length 3, and the driver must stay
    ## usable on the plant build)
    check("nchar(type = \"bytes\")/.subset/unclass/is.object/file.exists", {
        identical(nchar(c("ab", "\u00e9", NA, ""), type = "bytes", keepNA = FALSE), c(2L, 2L, 2L, 0L)) &&
            identical(.subset(structure(1:4, class = "a4x"), 2:3), 2:3) &&
            identical(unclass(structure(1:4, class = "a4x")), 1:4) &&
            is.object(structure(1, class = "a4x")) && !is.object(1) &&
            file.exists(batch_path) && !file.exists(file.path(out, "a4-not-there"))
    })
    check("readRDS of the batch", { b <- readRDS(batch_path); is.list(b$cases) })
    b <- readRDS(batch_path)
    invisible(file.remove(file.path(out, "selftest.rds")))
    saveRDS(TRUE, file.path(out, "started"), version = 3L, compress = FALSE)   # the harness's sign that it ran

    muffle <- function(expr) withCallingHandlers(expr, warning = function(w) invokeRestart("muffleWarning"))

    ## about the bytes serialize writes for v: each string's bytes, each
    ## element's size and an 8-byte header per element of a list, and the
    ## attributes' (a scalar with a big attribute is big)
    a4size <- function(v) {
        ## (the attributes' values, not the pairlist's own names, which have
        ## names of their own: no recursion into those)
        s <- 0
        for (x in attributes(v)) s <- s + a4size(x)
        if (is.character(v)) return(s + sum(nchar(v, type = "bytes", keepNA = FALSE)) + 8 * length(v))
        if (identical(typeof(v), "list")) { for (x in v) s <- s + 8 + a4size(x); return(s) }
        if (is.atomic(v)) return(s + length(v) * switch(typeof(v), logical = , integer = 4, double = 8,
                                                         complex = 16, raw = 1, 0))
        s
    }
    ## whether v holds no reference object (an environment, a closure, an
    ## external pointer...) anywhere, attributes included: only such a
    ## value goes to a sidecar of its own, whose serialization cannot share
    ## a reference with the rest of the record
    a4plain <- function(v) {
        for (x in attributes(v)) if (!a4plain(x)) return(FALSE)
        if (is.null(v) || is.atomic(v)) return(TRUE)
        ## (a list, not a pairlist: is.list() takes both)
        if (identical(typeof(v), "list")) { for (x in v) if (!a4plain(x)) return(FALSE); return(TRUE) }
        FALSE
    }
    ## v, or for a plain value over A4_BIG_BYTES (and not over
    ## A4_BIG_MAX_BYTES, which an expanded copy might not fit in the
    ## harness's address-space cap) its summary, the value itself in the
    ## case's k-th sidecar; any other value stays in the record as before
    a4big <- function(v, id, k) {
        ## (isTRUE: no estimate, whatever v holds, may stop the case)
        size <- tryCatch(a4size(v), error = function(e) NA)
        if (!isTRUE(size > A4_BIG_BYTES) || !isTRUE(size <= A4_BIG_MAX_BYTES) ||
            !isTRUE(tryCatch(a4plain(v), error = function(e) FALSE))) return(v)
        side <- file.path(out, sprintf("%06d.big%d.rds", id, k))
        if (!isTRUE(tryCatch({ saveRDS(v, side, version = 2L, compress = FALSE); TRUE },
                             error = function(e) FALSE))) {
            if (file.exists(side)) file.remove(side)
            return(v)
        }
        ## the first and last elements, of a vector or a list, by its own
        ## length (length() dispatches: a POSIXlt of 100 times has 11
        ## elements), unless they are big themselves
        n <- if (is.object(v)) length(unclass(v)) else length(v)
        h <- min(n, 3L)
        ends <- h > 0L && (is.atomic(v) || identical(typeof(v), "list"))
        head <- if (ends) .subset(v, seq_len(h))
        tail <- if (ends) .subset(v, (n - h + 1L):n)
        if (ends && !isTRUE(a4size(head) + a4size(tail) <= A4_ENDS_BYTES)) head <- tail <- NULL
        ## the harness's sign that this summary has a digest to compare: a
        ## summary whose digest is missing on either side never passes
        saveRDS(TRUE, file.path(out, sprintf("%06d.big%d.want", id, k)), version = 3L, compress = FALSE)
        list(a4_big = k, type = typeof(v), length = n, attributes = names(attributes(v)), head = head, tail = tail)
    }

    run1 <- function(c) {
        ## the RNG state the case starts from
        ## an empty global environment (no .Random.seed either: a case may
        ## have left one GetRNGstate refuses), then the case's RNG state
        g <- ls(globalenv(), all.names = TRUE)
        if (length(g)) rm(list = g, envir = globalenv())
        r <- c$rng
        setup <- tryCatch({
            muffle(RNGkind(r[[1L]], r[[2L]], r[[3L]]))
            s <- r[[4L]]
            if (identical(s, "none")) rm(".Random.seed", envir = globalenv())
            else if (identical(s, "corrupt"))
                assign(".Random.seed", c(10403L, 624L, 1:3), envir = globalenv())   # Mersenne-Twister with 3 of its 625 words
            else muffle(set.seed(s))
            NULL
        }, error = function(x) conditionMessage(x))
        if (!is.null(setup)) return(list(setup_error = setup))

        e <- new.env(parent = baseenv())
        if (c$kind == "extra") body <- c$expr else {
            k <- length(c$pos)
            syms <- vector("list", k)
            for (i in seq_len(k)) {
                if (c$missing[[i]]) syms[i] <- list(quote(expr = )) else {
                    n <- paste0("a", i)
                    ## a copy of its own, so a write through it reaches no later case
                    assign(n, unserialize(serialize(b$dom[[c$pos[[i]]]][[c$labels[[i]]]], NULL)), envir = e)
                    assign(paste0("alias", i), get0(n, e, inherits = FALSE), envir = e)
                    syms[[i]] <- as.name(n)
                }
            }
            nm <- c$names
            keep <- !(c$missing & nzchar(nm))
            syms <- syms[keep]; nm <- nm[keep]
            if (any(nzchar(nm))) names(syms) <- nm
            f <- as.name(if (c$kind == "wrapper") c$wrapper else b$name)
            body <- as.call(c(list(f), syms))
            if (b$internal && c$kind != "wrapper") body <- as.call(list(as.name(".Internal"), body))
        }
        snap <- list()
        for (n in ls(e, all.names = TRUE)) snap[[n]] <- serialize(get0(n, e, inherits = FALSE), NULL)
        a4case <- eval(as.call(list(as.name("function"), NULL, body)), e)
        assign("a4case", a4case, envir = e)

        conds <- list(); err <- NULL
        res <- tryCatch(withCallingHandlers(withVisible(eval(quote(a4case()), e)),
            warning = function(w) {
                conds[[length(conds) + 1L]] <<- list("warning", conditionMessage(w), class(w), conditionCall(w))
                invokeRestart("muffleWarning")
            },
            message = function(m) {
                conds[[length(conds) + 1L]] <<- list("message", conditionMessage(m), class(m), conditionCall(m))
                invokeRestart("muffleMessage")
            }),
            error = function(x) { err <<- list(conditionMessage(x), class(x), conditionCall(x)); NULL })
        seed <- get0(".Random.seed", globalenv(), inherits = FALSE)
        ## each binding: TRUE when it is the one made before the call, its
        ## serialization byte for byte the same (an in-place write through
        ## the shared argument shows here), else its value
        en <- sort(setdiff(ls(e, all.names = TRUE), "a4case"))
        env <- lapply(en, function(n) {
            v <- get0(n, e, inherits = FALSE)
            if (!is.null(snap[[n]]) && identical(serialize(v, NULL), snap[[n]])) list(unchanged = TRUE)
            else list(unchanged = FALSE, value = v)
        })
        names(env) <- en
        list(value = res$value, visible = res$visible, conds = conds, error = err, env = env,
             globals = sort(ls(globalenv(), all.names = TRUE)), seed = seed,
             log = get0(".a4log", globalenv(), inherits = FALSE))
    }

    n <- min(length(b$cases), as.integer(Sys.getenv("A4_TO", length(b$cases))))
    if (from <= n) for (i in from:n) {
        c <- b$cases[[i]]
        cat("\001A4 ", c$id, "\001\n", sep = "")
        rec <- run1(c)
        if (is.null(rec$setup_error) && is.null(rec$driver_error)) {
            ## (["value"] <- list(): a NULL value stays in the record)
            rec["value"] <- list(a4big(rec$value, c$id, 1L))
            k <- 1L
            for (n in names(rec$env)) if (!isTRUE(rec$env[[n]]$unchanged)) {
                k <- k + 1L
                rec$env[[n]]["value"] <- list(a4big(rec$env[[n]]$value, c$id, k))
            }
        }
        f <- file.path(out, sprintf("%06d.rds", c$id))
        tryCatch(saveRDS(rec, f, version = 3L, compress = FALSE),
                 error = function(x) saveRDS(list(driver_error = conditionMessage(x)), f, version = 3L, compress = FALSE))
        ## the case is done: its record is whole (robigo-rt has no file.rename)
        saveRDS(TRUE, file.path(out, sprintf("%06d.ok", c$id)), version = 3L, compress = FALSE)
    }
})
