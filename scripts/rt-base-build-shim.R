# Stand-ins for internals robigo-rt lacks, for scripts/rt-base-build.sh
# --shim: read before each step's script, so that the step runs past what
# is still missing and reports what stops it next. Each names its owner;
# delete a stand-in when its owner's port is on the binary built. Never
# part of robigo-rt: a probe of how far the build gets.
.rt_shim <- function(name, f) {
    e <- baseenv()
    environment(f) <- .BaseNamespaceEnv
    unlockBinding(name, e)
    assign(name, f, envir = e)
    lockBinding(name, e)
}

## aperm (E, array.c PR: aperm, asplit, col, row): the permutation by index
## arithmetic, dimnames permuted (resize = TRUE) or the old dim kept
.rt_shim("aperm.default", function(a, perm = NULL, resize = TRUE, ...) {
    d <- dim(a)
    n <- length(d)
    if (is.null(perm)) perm <- n:1
    if (is.character(perm)) perm <- match(perm, names(dimnames(a)))
    nd <- d[perm]
    N <- prod(d)
    stride <- cumprod(c(1L, d))[seq_len(n)]
    rem <- seq_len(N) - 1L
    acc <- integer(N)
    for (k in seq_len(n)) {
        acc <- acc + (rem %% nd[k]) * stride[perm[k]]
        rem <- rem %/% nd[k]
    }
    r <- as.vector(a)[acc + 1L]
    if (resize) {
        dim(r) <- nd
        dn <- dimnames(a)
        if (!is.null(dn)) dimnames(r) <- dn[perm]
    } else dim(r) <- d
    r
})

## asplit (E, array.c PR): base's own prelude, then do_asplit's slicing
.rt_shim("asplit", function(x, MARGIN, drop = FALSE) {
    dl <- length(dim(x))
    if (!dl) stop("dim(x) must have a positive length")
    if (is.object(x)) x <- if (dl == 2L) as.matrix(x) else as.array(x)
    d <- dim(x)
    dn <- dimnames(x)
    ds <- seq_len(dl)
    if (is.character(MARGIN)) {
        if (is.null(dnn <- names(dn))) stop("'x' must have named dimnames")
        MARGIN <- match(MARGIN, dnn)
        if (anyNA(MARGIN)) stop("not all elements of 'MARGIN' are names of dimensions")
    }
    d.call <- d[-MARGIN]
    d.ans <- d[MARGIN]
    if (anyNA(d.call) || anyNA(d.ans)) stop("'MARGIN' does not match dim(X)")
    s.call <- ds[-MARGIN]
    s.ans <- ds[MARGIN]
    dn.call <- dn[-MARGIN]
    dn.ans <- dn[MARGIN]
    dimnames(x) <- NULL
    v <- aperm(x, c(s.call, s.ans))
    n1 <- prod(d.call)
    n2 <- prod(d.ans)
    keep <- !as.logical(drop)
    y <- vector("list", n2)
    for (i in seq_len(n2)) {
        e <- as.vector(v)[(i - 1L) * n1 + seq_len(n1)]
        if (keep) {
            dim(e) <- d.call
            if (length(dn.call)) dimnames(e) <- dn.call
        }
        y[[i]] <- e
    }
    dim(y) <- d.ans
    if (length(dn.ans)) dimnames(y) <- dn.ans
    y
})

## split (B, split.c's do_split): f recycled over x, NAs dropped, names kept,
## a group per level
.rt_shim("split.default", function(x, f, drop = FALSE, sep = ".", lex.order = FALSE, ...) {
    if (is.list(f)) f <- interaction(f, drop = drop, sep = sep, lex.order = lex.order)
    else if (!is.factor(f)) f <- as.factor(f)
    else if (drop) f <- factor(f)
    storage.mode(f) <- "integer"
    lv <- levels(f)
    sp <- function(v) {
        fi <- rep_len(unclass(f), length(v))
        res <- vector("list", length(lv))
        for (j in seq_along(lv)) res[[j]] <- v[which(fi == j)]
        names(res) <- lv
        res
    }
    if (is.null(attr(x, "class"))) return(sp(x))
    lapply(sp(seq_along(x)), function(i) x[i])
})

## Sys.time (B, datetime.c's do_systime): the time a fresh file was made
## (file.info's mtime), as a POSIXct
.rt_shim("Sys.time", function() {
    f <- paste0(Sys.getenv("TMPDIR", "/tmp"), "/rt-shim-time-", Sys.getpid())
    close(file(f, "w"))
    t <- file.info(f)$mtime
    file.remove(f)
    t
})

## iconv (B, sysutils.c's do_iconv): only between the native encoding and
## UTF-8 (the same text in a UTF-8 locale; non-ASCII is NA in ASCII), else
## an error naming the conversion
.rt_shim("iconv", function(x, from = "", to = "", sub = NA, mark = TRUE, toRaw = FALSE) {
    utf <- l10n_info()[["UTF-8"]]
    enc <- function(e) if (e %in% c("", "native.enc")) (if (utf) "UTF-8" else "ASCII") else toupper(sub("UTF8", "UTF-8", e, fixed = TRUE))
    f <- enc(from)
    t <- enc(to)
    if (f == t) return(x)
    if (f == "UTF-8" && t == "ASCII") {
        r <- x
        r[!is.na(x) & nchar(x, "bytes") > nchar(x, "chars", allowNA = TRUE)] <- NA
        return(r)
    }
    stop("rt-base-build shim: iconv from '", from, "' to '", to, "' is not stood in")
})
