## A4's driver (docs/verification.md, "Builtin differential"): runs one
## batch's cases in GNU R 4.6.2 and in robigo-rt alike, and writes one record
## per case, `<A4_OUT>/<id>.rds` (saveRDS, version 3, uncompressed), whose
## bytes scripts/builtin-diff.py compares. A record is
##   list(value, visible, conds, error, env, globals, seed)
## conds: each warning and message in order, list(type, message, class,
##   call); error: list(message, class, call) or NULL; env: the case
##   environment's bindings after the call (the arguments a<i> and their
##   aliases alias<i>, bound to the same objects before it); globals: the
##   global environment's names; seed: .Random.seed after the call, or NULL.
##
## Parameters come from the environment (robigo-rt has no commandArgs yet):
##   A4_BATCH  the batch.rds gen.R wrote    A4_OUT    the records' directory
##   A4_FROM   the first case's index (1)    A4_LABEL  the runtime's name
##
## The driver uses only: readRDS, saveRDS, tryCatch, withCallingHandlers,
## withVisible, eval, as.call, as.name, new.env, assign, get0, ls, rm,
## RNGkind, set.seed, sprintf, Sys.getenv, stop. The self-test checks them
## first; a runtime that fails it gets "driver unusable on <label>: <what>"
## and no records, never a wrong comparison.
local({
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
    check("readRDS of the batch", { b <- readRDS(batch_path); is.list(b$cases) })
    b <- readRDS(batch_path)
    invisible(file.remove(file.path(out, "selftest.rds")))

    muffle <- function(expr) withCallingHandlers(expr, warning = function(w) invokeRestart("muffleWarning"))

    run1 <- function(c) {
        ## the RNG state the case starts from
        r <- c$rng
        muffle(RNGkind(r[[1L]], r[[2L]], r[[3L]]))
        s <- r[[4L]]
        if (identical(s, "none")) {
            if (!is.null(get0(".Random.seed", globalenv(), inherits = FALSE))) rm(".Random.seed", envir = globalenv())
        } else if (identical(s, "corrupt")) {
            assign(".Random.seed", c(10403L, 624L, 1:3), envir = globalenv())   # Mersenne-Twister with 3 of its 625 words
        } else muffle(set.seed(s))
        g <- setdiff(ls(globalenv(), all.names = TRUE), ".Random.seed")
        if (length(g)) rm(list = g, envir = globalenv())

        e <- new.env(parent = baseenv())
        if (c$kind == "extra") body <- c$expr else {
            k <- length(c$args)
            syms <- vector("list", k)
            for (i in seq_len(k)) {
                if (c$missing[[i]]) syms[i] <- list(quote(expr = )) else {
                    n <- paste0("a", i)
                    assign(n, c$args[[i]], envir = e)
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
        en <- sort(setdiff(ls(e, all.names = TRUE), "a4case"))
        env <- lapply(en, function(n) get0(n, e, inherits = FALSE)); names(env) <- en
        list(value = res$value, visible = res$visible, conds = conds, error = err, env = env,
             globals = sort(ls(globalenv(), all.names = TRUE)), seed = seed)
    }

    n <- length(b$cases)
    if (from <= n) for (i in from:n) {
        c <- b$cases[[i]]
        rec <- run1(c)
        f <- file.path(out, sprintf("%06d.rds", c$id))
        tryCatch(saveRDS(rec, f, version = 3L, compress = FALSE),
                 error = function(x) saveRDS(list(driver_error = conditionMessage(x)), f, version = 3L, compress = FALSE))
    }
})
