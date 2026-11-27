# coerce.c installTrChar / translateChar under LC_ALL=C: VectorToPairList,
# coerceToPairList, do_asfunction, do_ascall, do_docall, mapply(MoreArgs),
# call subset/subassign.
# This copy runs under LC_ALL=C (its .locale sidecar).
x <- intToUtf8(c(99L, 97L, 102L, 233L))
y <- intToUtf8(c(110L, 97L, 239L, 118L, 101L))
b <- "\x80"
Encoding(b) <- "bytes"

probe <- function(lab, expr) {
  cat("--", lab, "\n")
  r <- withCallingHandlers(
    tryCatch(expr, error = function(e) { cat("error:", conditionMessage(e), "\n"); invisible(NULL) }),
    warning = function(w) { cat("warning:", conditionMessage(w), "\n"); invokeRestart("muffleWarning") }
  )
  if (!is.null(r)) print(r)
}

probe("as.pairlist(alist)", { a <- alist(x = ); names(a) <- x; names(as.pairlist(a)) })
probe("as.pairlist(list)", { l <- list(1); names(l) <- x; as.pairlist(l) })
probe("as.pairlist(list 2 names)", { l <- list(1, 2); names(l) <- c(x, y); names(as.pairlist(l)) })
probe("as.pairlist(list utf8+bytes)", { l <- list(1, 2); names(l) <- c(x, b); as.pairlist(l) })
probe("as.pairlist(list NA name)", { l <- list(1); names(l) <- NA_character_; names(as.pairlist(l)) })
probe("as.pairlist(atomic)", { v <- 1:2; names(v) <- c(x, y); names(as.pairlist(v)) })
probe("as.pairlist(atomic NA name)", { v <- 1; names(v) <- NA_character_; names(as.pairlist(v)) })
probe("as.pairlist(atomic bytes)", { v <- 1; names(v) <- b; as.pairlist(v) })

probe("as.function", { l <- list(1, quote(1)); names(l) <- c(x, ""); names(formals(as.function(l))) })
probe("as.function bytes", { l <- list(1, quote(1)); names(l) <- c(b, ""); as.function(l) })

probe("as.call arg name", { l <- list(quote(f), 1); names(l) <- c("", x); names(as.call(l)) })
probe("as.call head name", { l <- list(quote(f), 1); names(l) <- c(x, y); names(as.call(l)) })
probe("as.call bytes", { l <- list(quote(f), 1); names(l) <- c("", b); as.call(l) })

probe("do.call arg name", { l <- list(1); names(l) <- x; names(do.call(list, l)) })
probe("do.call fun utf8", { do.call(x, list()) })
probe("do.call fun bytes", { do.call(b, list()) })
probe("do.call arg bytes", { l <- list(1); names(l) <- b; do.call(list, l) })

probe("mapply MoreArgs utf8", { l <- list(1); names(l) <- x; mapply(function(...) names(list(...)), 1, MoreArgs = l) })
probe("call subassign utf8", { cl <- quote(f(1)); cl[x] <- 2; names(cl) })
probe("call subassign bytes", { cl <- quote(f(1)); cl[b] <- 2; names(cl) })
