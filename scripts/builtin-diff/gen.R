## A4's case generator (docs/verification.md, "Builtin differential"). Runs in
## GNU R 4.6.2 only (the oracle image): the inputs are GNU R's own objects,
## saved with saveRDS and read back by both runtimes, so both see the same
## bits, encoding marks, attributes and compact sequences.
##
##   Rscript --vanilla gen.R SPEC OUTDIR
##
## SPEC is R code (written by scripts/builtin-diff.py) that defines `rows`, a
## list of rows:
##   list(name, internal, arity, effect, args, domain, extra, budget, seed)
## For each row OUTDIR/<name>/ gets batch.rds (the cases) and cases.tsv
## (id, kind, the call as GNU R deparses it). The case kinds:
##   single  every lattice value alone in every position, the others typical
##   pair    a greedy pairwise covering set over the positions
##   random  seeded random tuples
##   extra   the row's own R expressions
##   wrapper the base closure that calls the .Internal, with the same tuples
##           (rows of .Internal builtins that have one)
local({
    LATTICE_VERSION <- "1"
    args <- commandArgs(TRUE)
    spec <- args[[1L]]; outdir <- args[[2L]]
    rows <- local({ source(spec, local = TRUE); rows })

    ## ---- the value lattice (version 1) -------------------------------------
    lattice <- function() {
        RNGkind("Mersenne-Twister", "Inversion", "Rejection"); set.seed(20261118L)
        u <- "é"; l <- iconv(u, "UTF-8", "latin1"); b <- u; Encoding(b) <- "bytes"
        v <- list(
            "NULL" = NULL,
            "lgl0" = logical(), "T" = TRUE, "lglNA" = NA, "lgl3" = c(TRUE, NA, FALSE),
            "int0" = integer(), "0L" = 0L, "-1L" = -1L, "intNA" = NA_integer_,
            "intmax" = .Machine$integer.max, "int3" = c(1L, NA, 3L), "1:10" = 1:10,
            "10:1" = 10:1, "1:1e5" = 1:100000,
            "dbl0" = double(), "0" = 0, "-0" = -0, "1.5" = 1.5, "-2.5" = -2.5,
            "dblNA" = NA_real_, "NaN" = NaN, "Inf" = Inf, "-Inf" = -Inf, "1e308" = 1e308,
            "5e-324" = 5e-324, "2^53+1" = 2^53 + 1, "dbl4" = c(1, NA, NaN, Inf),
            "dblint" = c(2, 3, 4), "runif" = stats::runif(1e5),
            "cplx0" = complex(), "1+2i" = 1 + 2i, "cplxNA" = NA_complex_,
            "cplx2" = c(0i, complex(real = Inf, imaginary = NaN)),
            "chr0" = character(), "\"\"" = "", "a" = "a", "chrNA" = NA_character_,
            "chr3" = c("a", NA, "b"), "utf8" = u, "latin1" = l, "bytes" = b,
            "\"1e3\"" = "1e3", "\" 3 \"" = " 3 ", "\"0x1A\"" = "0x1A", "\"TRUE\"" = "TRUE",
            "long" = strrep("ab", 5000L),
            "raw0" = raw(), "raw2" = as.raw(c(0, 255)),
            "list0" = list(), "list2" = list(1, "a"), "nested" = list(a = 1, b = list(2, NULL)),
            "sym" = quote(x), "call" = quote(f(x, 1)), "expr" = expression(1, x),
            "formula" = local({ f <- y ~ x; environment(f) <- baseenv(); f })   # (not this frame, which holds the lattice)
        )
        dec <- list(
            "named" = c(a = 1, b = 2, 3),
            "NAnames" = structure(1:2, names = c("", NA)),
            "matrix" = matrix(1:6, 2L, dimnames = list(c("r1", "r2"), NULL)),
            "array3" = array(1:8, c(2L, 2L, 2L)),
            "factor" = factor(c("b", "a", NA, "b")),
            "data.frame" = data.frame(x = 1:2, y = c("a", "b")),
            "Date" = as.Date("2026-11-18"),
            "a4cls" = structure(list(1), class = "a4cls"),
            "chrmatrix" = matrix(c("a", "b", NA, ""), 2L)
        )
        c(v, dec)
    }
    L <- lattice()
    ## the values whose serialization is large: at most LARGE_CASES generated
    ## cases (beyond the single ones) may hold one, so that they do not
    ## dominate a row through the pairs
    large <- names(L)[vapply(L, function(x) length(serialize(x, NULL)) > 65536L, NA)]
    LARGE_CASES <- 25L
    ## the RNG states an Rng row's cases cycle through (a Pure row's cases
    ## all start from the first): kind, normal.kind, sample.kind, then the
    ## seed, or "none" (no .Random.seed) or "corrupt" (one GetSeedsFromVar
    ## refuses)
    rng_settings <- list(
        list("Mersenne-Twister", "Inversion", "Rejection", 42L),
        list("Wichmann-Hill", "Inversion", "Rejection", 1L),
        list("Marsaglia-Multicarry", "Kinderman-Ramage", "Rejection", 2L),
        list("Super-Duper", "Ahrens-Dieter", "Rejection", 3L),
        list("Knuth-TAOCP", "Box-Muller", "Rejection", 4L),
        list("Knuth-TAOCP-2002", "Inversion", "Rounding", 5L),
        list("L'Ecuyer-CMRG", "Inversion", "Rejection", 6L),
        list("Mersenne-Twister", "Inversion", "Rejection", "none"),
        list("Mersenne-Twister", "Inversion", "Rejection", "corrupt")
    )
    domains <- list(
        any = names(L),
        count = c("0L", "-1L", "intNA", "intmax", "1.5", "dblNA", "Inf", "dbl0", "int3", "\"1e3\"", "T"),
        num = c("0L", "-1L", "intNA", "int3", "1:10", "0", "-0", "1.5", "-2.5", "dblNA", "NaN",
                "Inf", "-Inf", "1e308", "5e-324", "2^53+1", "dbl4", "dbl0", "int0", "1+2i"),
        chr = c("chr0", "\"\"", "a", "chrNA", "chr3", "utf8", "latin1", "bytes", "\"1e3\"",
                "\" 3 \"", "\"0x1A\"", "\"TRUE\"", "long", "chrmatrix"),
        lgl = c("lgl0", "T", "lglNA", "lgl3", "0L", "1.5", "a", "NULL")
    )
    ## a domain: a vector of names, each a lattice value's name, a domain's
    ## ("@num"), or an R expression ("=expr") evaluated here
    resolve <- function(d) {
        if (is.null(d)) d <- "@any"
        out <- list()
        for (x in d) {
            if (startsWith(x, "@")) for (n in domains[[substring(x, 2L)]]) out[n] <- list(L[[n]])
            else if (startsWith(x, "=")) out[x] <- list(eval(str2lang(substring(x, 2L)), baseenv()))
            else if (x == "<missing>") out[x] <- list(quote(expr = ))
            else out[x] <- list(L[[x]])
        }
        out
    }

    ## ---- the .Internal's base wrapper and its argument names ---------------
    wrapper_of <- function(name) {
        base <- baseenv()
        inner <- function(f) {
            b <- body(f); hit <- NULL
            walk <- function(e) {
                if (!is.null(hit) || !is.call(e)) return()
                if (identical(e[[1L]], as.name(".Internal")) && is.call(e[[2L]]) &&
                    identical(e[[2L]][[1L]], as.name(name))) { hit <<- e[[2L]]; return() }
                for (x in as.list(e)[-1L]) if (!missing(x)) walk(x)
            }
            walk(b); hit
        }
        cands <- c(name, ls(base, all.names = TRUE))
        for (n in cands) {
            f <- get0(n, base, inherits = FALSE)
            if (is.function(f) && !is.primitive(f)) {
                h <- inner(f)
                if (!is.null(h)) return(list(name = n, call = h))
            }
        }
        NULL
    }

    one <- function(row) {
        set.seed(row$seed + sum(utf8ToInt(row$name)))
        w <- if (row$internal) wrapper_of(row$name) else NULL
        prim <- if (!row$internal) get0(row$name, baseenv(), inherits = FALSE) else NULL
        argn <- row$args
        if (is.null(argn)) {
            if (!is.null(w)) argn <- vapply(as.list(w$call)[-1L], function(e) if (is.name(e)) as.character(e) else "", "")
            else if (is.function(prim)) { f <- formals(args(prim)); argn <- setdiff(names(f), "...") }
            else argn <- character()
        }
        k <- if (row$arity >= 0L) row$arity else max(length(argn), 1L)
        if (length(argn) < k) argn <- c(argn, rep("", k - length(argn)))
        argn <- argn[seq_len(k)]
        dom <- lapply(seq_len(k), function(i) resolve(row$domain[[argn[[i]]]] %||% row$domain[[paste0("#", i)]]))
        if (!row$internal) for (i in seq_len(k)) if (!"<missing>" %in% names(dom[[i]])) dom[[i]]["<missing>"] <- list(quote(expr = ))
        named <- !row$internal && all(nzchar(argn))
        cases <- list(); seen <- new.env(hash = TRUE)
        nlarge <- 0L
        add <- function(kind, pick) {
            key <- paste(pick, collapse = "\r")
            if (kind != "wrapper" && exists(key, seen, inherits = FALSE)) return(FALSE)
            if (kind %in% c("pair", "random") && any(pick %in% large)) {
                if (nlarge >= LARGE_CASES) return(FALSE)
                nlarge <<- nlarge + 1L
            }
            assign(key, TRUE, seen)
            miss <- vapply(seq_len(k), function(i) pick[[i]] == "<missing>", NA)
            ## the values themselves are the batch's `dom`, once: a case names them
            cases[[length(cases) + 1L]] <<- list(kind = kind, labels = pick, pos = seq_len(k), missing = miss,
                                                   names = if (named || kind == "wrapper") argn else rep("", k))
            TRUE
        }
        ## each position's typical value: the row's, else the first of an
        ## explicit domain, else a missing argument (a primitive) or 1.5
        typ <- vapply(seq_len(k), function(i) {
            t <- row$typical[[argn[[i]]]]
            if (!is.null(t)) return(t)
            d <- row$domain[[argn[[i]]]] %||% row$domain[[paste0("#", i)]]
            if (!is.null(d) && !startsWith(d[[1L]], "@")) return(d[[1L]])
            if (!row$internal) "<missing>" else "1.5"
        }, "")
        for (i in seq_len(k)) if (!typ[[i]] %in% names(dom[[i]])) dom[[i]][typ[[i]]] <- resolve(typ[[i]])
        budget <- row$budget
        if (k > 0L) {
            for (i in seq_len(k)) for (v in names(dom[[i]])) { p <- typ; p[[i]] <- v; add("single", p) }
            ## greedy pairwise: pick, of 30 random tuples, the one covering most
            ## uncovered pairs, until all are covered or the budget is spent
            if (k >= 2L) {
                nv <- lengths(dom)
                pairs <- list(); unc <- list()
                for (i in 1:(k - 1L)) for (j in (i + 1L):k) {
                    pairs[[length(pairs) + 1L]] <- c(i, j)
                    unc[[length(unc) + 1L]] <- matrix(TRUE, nv[[i]], nv[[j]])
                }
                nunc <- sum(vapply(unc, length, 0L))
                draw <- function() vapply(seq_len(k), function(i) sample.int(nv[[i]], 1L), 0L)
                gain <- function(p) { g <- 0L; for (q in seq_along(pairs)) g <- g + unc[[q]][p[[pairs[[q]][[1L]]]], p[[pairs[[q]][[2L]]]]]; g }
                while (nunc > 0L && length(cases) < budget) {
                    best <- NULL; bg <- -1L
                    for (t in 1:30) { p <- draw(); g <- gain(p); if (g > bg) { best <- p; bg <- g } }
                    if (bg <= 0L) break
                    pick <- vapply(seq_len(k), function(i) names(dom[[i]])[[best[[i]]]], "")
                    if (add("pair", pick)) for (q in seq_along(pairs)) {
                        x <- best[[pairs[[q]][[1L]]]]; y <- best[[pairs[[q]][[2L]]]]
                        if (unc[[q]][x, y]) { unc[[q]][x, y] <- FALSE; nunc <- nunc - 1L }
                    }
                }
            }
            n0 <- length(cases)
            tries <- 0L
            while (length(cases) < min(budget, n0) + 200L && tries < 2000L) {
                tries <- tries + 1L
                add("random", vapply(seq_len(k), function(i) sample(names(dom[[i]]), 1L), ""))
            }
        } else add("single", character())
        if (length(cases) > budget + 200L) cases <- cases[seq_len(budget + 200L)]
        if (!is.null(w)) {
            base <- cases[vapply(cases, function(c) c$kind == "single", NA)]
            ## the positions the wrapper passes on from a formal of its own
            keep <- nzchar(argn) & argn %in% names(formals(get(w$name, baseenv())))
            for (c in base[seq_len(min(length(base), row$wrapper_budget))]) {
                c$kind <- "wrapper"; c$wrapper <- w$name
                c$pos <- c$pos[keep]; c$missing <- c$missing[keep]; c$labels <- c$labels[keep]
                c$names <- argn[keep]
                cases[[length(cases) + 1L]] <- c
            }
        }
        for (e in row$extra) cases[[length(cases) + 1L]] <- list(kind = "extra", expr = str2lang(e), labels = e)
        for (i in seq_along(cases)) {
            cases[[i]]$id <- i
            cases[[i]]$rng <- if (row$effect == "Rng") rng_settings[[(i - 1L) %% length(rng_settings) + 1L]] else rng_settings[[1L]]
        }
        for (i in seq_len(k)) dom[[i]] <- dom[[i]][names(dom[[i]]) != "<missing>"]
        list(name = row$name, internal = row$internal, effect = row$effect, dom = dom,
             lattice = LATTICE_VERSION, seed = row$seed, cases = cases)
    }

    ## the call a case makes, as the driver builds it (deparsed for cases.tsv)
    show_call <- function(b, c) {
        if (c$kind == "extra") return(paste(deparse(c$expr, width.cutoff = 500L), collapse = " "))
        a <- paste0(ifelse(nzchar(c$names), paste0(c$names, " = "), ""), c$labels)
        a[c$missing] <- ""                       # a positional gap: f(x, , y)
        a <- a[!(c$missing & nzchar(c$names))]   # a named one is left out
        f <- if (c$kind == "wrapper") c$wrapper else b$name
        s <- paste0(f, "(", paste(a, collapse = ", "), ")")
        if (b$internal && c$kind != "wrapper") paste0(".Internal(", s, ")") else s
    }
    for (row in rows) {
        b <- one(row)
        d <- file.path(outdir, row$name); dir.create(d, recursive = TRUE, showWarnings = FALSE)
        saveRDS(b, file.path(d, "batch.rds"), version = 3L, compress = FALSE)
        tsv <- vapply(b$cases, function(c) paste(c$id, c$kind, gsub("[\t\n]", " ", show_call(b, c)), sep = "\t"), "")
        writeLines(tsv, file.path(d, "cases.tsv"), useBytes = TRUE)
    }
})
