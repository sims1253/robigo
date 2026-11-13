# The tracer of scripts/base-subset.sh: R_PROFILE_USER for a GNU R 4.6.2
# run of one script, which writes the base functions the script reaches to
# $ROBIGO_SUBSET_OUT, one key per line:
#
#   closure:NAME     a closure of base was entered
#   primitive:NAME   a primitive of base was called, or passed to a
#                    higher-order function (lapply(x, length))
#   internal:NAME    .Internal(NAME(...)) was called
#   user:NAME        a closure of the script was entered (not base)
#
# How. After GNU R's own start-up (`.First.sys`, which this profile wraps),
# every closure of base gets a body that records what it does: one
# statement that stores keys, built of constants and .Internal(list2env())
# so that it calls no closure, before each statement of each block and at
# the start of each branch, loop body and switch arm. A statement's keys
# are the primitives and internals written in it, outside the branches and
# function literals that record their own; a closure's first record also
# names the closure and covers its default arguments. Arguments of closure
# calls are left as they are (only their blocks and function literals are
# instrumented), because a closure can deparse its arguments' expressions
# (`substitute`, `match.call`); their keys are recorded with the statement.
# So the list errs on the side of a statement's untaken `&&`/`||` operands
# and lazy arguments, never on a branch not taken.
#
# Top-level code is recorded by a task callback after each top-level
# expression: its keys, and the closures it bound in the global
# environment, which are instrumented the same way before they run. The
# key set is written after each top-level expression, so GNU R's exit
# (`.Last.sys`) does not count, nor does the callback's own work.
#
# The JIT stays off (R_ENABLE_JIT=0): compiling would run the compiler's
# R code. scripts/base-subset.R checks that a traced run prints exactly what
# an untraced one does.
local({
  out <- Sys.getenv("ROBIGO_SUBSET_OUT")
  if (!nzchar(out)) stop("base-subset-trace.R: ROBIGO_SUBSET_OUT is not set")
  base <- baseenv()
  E <- new.env(parent = emptyenv()) # the keys reached
  INTERNAL <- get(".Internal", base)
  prim <- new.env(parent = emptyenv())
  for (n in ls(base, all.names = TRUE)) {
    if (is.primitive(get(n, base))) assign(n, TRUE, envir = prim)
  }
  isprim <- function(n) exists(n, envir = prim, inherits = FALSE)
  # Functions whose arguments are functions: a primitive named there (as a
  # symbol or a string) is called by them, from C.
  HOF <- c("lapply", "sapply", "vapply", "mapply", "Map", "Reduce", "Filter", "Position", "Find",
           "do.call", "match.fun", "outer", "apply", "tapply", "rapply", "eapply", "Vectorize",
           "Negate", "Recall")
  # Calls whose arguments are not evaluated.
  QUOTING <- c("quote", "substitute", "expression", "~", "bquote", "alist")
  CONTROL <- c("if", "for", "while", "repeat", "switch", "{", "function")

  # The statement that stores `keys` in E: constants and one .Internal.
  hit <- function(keys) {
    l <- rep(list(TRUE), length(keys))
    names(l) <- keys
    as.call(list(INTERNAL, as.call(list(as.name("list2env"), l, E))))
  }
  is_fun_literal <- function(x) is.call(x) && identical(x[[1L]], as.name("function"))
  is_block <- function(x) is.call(x) && identical(x[[1L]], as.name("{"))

  # The keys written in `e`. deep = FALSE leaves out what records itself:
  # branches, loop bodies, switch arms, blocks and function literals
  # (except the cond/seq parts, which run with the statement), and the
  # blocks and function literals among a closure call's arguments.
  keys <- function(e, deep) {
    acc <- character()
    add <- function(k) acc <<- c(acc, k)
    # The replacement functions an assignment to `lhs` calls.
    lhs_keys <- function(lhs, outer) {
      if (!is.call(lhs)) return()
      f <- lhs[[1L]]
      if (is.symbol(f)) {
        n <- as.character(f)
        if (isprim(paste0(n, "<-"))) add(paste0("primitive:", n, "<-"))
        if (!outer && isprim(n)) add(paste0("primitive:", n))
      }
      if (length(lhs) >= 2L) lhs_keys(lhs[[2L]], FALSE)
      if (length(lhs) >= 3L) {
        for (i in 3:length(lhs)) if (!identical(lhs[[i]], quote(expr = ))) walk(lhs[[i]])
      }
    }
    walk <- function(e) {
      if (!is.call(e)) return()
      h <- e[[1L]]
      n <- if (is.symbol(h)) as.character(h) else ""
      if (!is.symbol(h)) walk(h)
      if (n == ".Internal") {
        add("primitive:.Internal")
        inner <- e[[2L]]
        if (is.call(inner) && is.symbol(inner[[1L]])) {
          add(paste0("internal:", as.character(inner[[1L]])))
          if (length(inner) >= 2L) {
            for (i in 2:length(inner)) if (!identical(inner[[i]], quote(expr = ))) walk(inner[[i]])
          }
        }
        return()
      }
      if (nzchar(n) && isprim(n)) add(paste0("primitive:", n))
      if (n %in% QUOTING) return()
      if (n %in% c("<-", "=", "<<-") && length(e) == 3L) {
        lhs_keys(e[[2L]], TRUE)
        walk(e[[3L]])
        return()
      }
      if (n %in% HOF && length(e) >= 2L) {
        for (i in 2:length(e)) {
          a <- if (identical(e[[i]], quote(expr = ))) NULL else e[[i]]
          nm <- if (is.symbol(a)) as.character(a) else if (is.character(a) && length(a) == 1L) a else ""
          if (nzchar(nm) && isprim(nm)) add(paste0("primitive:", nm))
        }
      }
      own <- function(i) {
        if (deep) walk(e[[i]])
      }
      if (n == "function") {
        if (deep) {
          fl <- as.list(e[[2L]])
          for (i in seq_along(fl)) if (!identical(fl[[i]], quote(expr = ))) walk(fl[[i]])
          walk(e[[3L]])
        }
        return()
      }
      if (n == "{") {
        if (deep && length(e) >= 2L) for (i in 2:length(e)) walk(e[[i]])
        return()
      }
      if (n == "if") {
        walk(e[[2L]])
        own(3L)
        if (length(e) == 4L) own(4L)
        return()
      }
      if (n == "for") {
        walk(e[[3L]])
        own(4L)
        return()
      }
      if (n == "while") {
        walk(e[[2L]])
        own(3L)
        return()
      }
      if (n == "repeat") {
        own(2L)
        return()
      }
      if (n == "switch") {
        walk(e[[2L]])
        if (length(e) >= 3L) for (i in 3:length(e)) if (!identical(e[[i]], quote(expr = ))) own(i)
        return()
      }
      closure_call <- !(nzchar(n) && isprim(n))
      if (length(e) >= 2L) {
        for (i in 2:length(e)) {
          if (identical(e[[i]], quote(expr = ))) next
          if (closure_call && !deep && (is_block(e[[i]]) || is_fun_literal(e[[i]]))) next
          walk(e[[i]])
        }
      }
    }
    walk(e)
    unique(acc)
  }

  # `x` with its own regions instrumented; `x` itself is not wrapped.
  rw <- function(e) {
    if (!is.call(e)) return(e)
    h <- e[[1L]]
    n <- if (is.symbol(h)) as.character(h) else ""
    if (!is.symbol(h)) e[1L] <- list(rw(h))
    if (n %in% QUOTING || n == ".Internal") return(e)
    if (n == "function") {
      e[3L] <- list(body_rw(e[[3L]], defaults_keys(e[[2L]])))
      return(e)
    }
    if (n == "{") return(block(e, character()))
    if (n == "if") {
      e[2L] <- list(rw(e[[2L]]))
      e[3L] <- list(region(e[[3L]]))
      if (length(e) == 4L) e[4L] <- list(region(e[[4L]]))
      return(e)
    }
    if (n == "for") {
      e[3L] <- list(rw(e[[3L]]))
      e[4L] <- list(region(e[[4L]]))
      return(e)
    }
    if (n == "while") {
      e[2L] <- list(rw(e[[2L]]))
      e[3L] <- list(region(e[[3L]]))
      return(e)
    }
    if (n == "repeat") {
      e[2L] <- list(region(e[[2L]]))
      return(e)
    }
    if (n == "switch") {
      e[2L] <- list(rw(e[[2L]]))
      if (length(e) >= 3L) {
        for (i in 3:length(e)) if (!identical(e[[i]], quote(expr = ))) e[i] <- list(region(e[[i]]))
      }
      return(e)
    }
    closure_call <- !(nzchar(n) && isprim(n))
    if (length(e) >= 2L) {
      for (i in 2:length(e)) {
        if (identical(e[[i]], quote(expr = ))) next
        a <- e[[i]]
        if (closure_call) {
          if (is_block(a)) e[i] <- list(block(a, character()))
          else if (is_fun_literal(a)) e[i] <- list(rw(a))
        } else {
          e[i] <- list(rw(a))
        }
      }
    }
    e
  }
  # A branch or body: it records its own keys.
  region <- function(x) {
    if (is_block(x)) return(block(x, character()))
    k <- keys(x, FALSE)
    x <- rw(x)
    if (length(k)) as.call(list(as.name("{"), hit(k), x)) else x
  }
  # A block: a record before each statement; `extra` joins the first.
  block <- function(b, extra) {
    out <- list(as.name("{"))
    first <- TRUE
    if (length(b) >= 2L) {
      for (i in 2:length(b)) {
        s <- b[[i]]
        k <- keys(s, FALSE)
        if (first) k <- unique(c("primitive:{", extra, k))
        first <- FALSE
        if (length(k)) out <- c(out, list(hit(k)))
        out <- c(out, list(rw(s)))
      }
    } else if (length(extra)) {
      out <- c(out, list(hit(extra), NULL))
    }
    as.call(out)
  }
  body_rw <- function(body, extra) {
    if (is_block(body)) return(block(body, extra))
    k <- unique(c(extra, keys(body, FALSE)))
    body <- rw(body)
    if (length(k)) as.call(list(as.name("{"), hit(k), body)) else body
  }
  defaults_keys <- function(formals) {
    k <- character()
    fl <- as.list(formals)
    for (i in seq_along(fl)) if (!identical(fl[[i]], quote(expr = ))) k <- c(k, keys(fl[[i]], TRUE))
    unique(k)
  }
  instrument <- function(f, key) {
    g <- f
    body(g) <- body_rw(body(f), unique(c(key, defaults_keys(formals(f)))))
    a <- attributes(f)
    a$srcref <- NULL
    if (length(a)) attributes(g) <- a
    g
  }

  # Remove every key not in `keep`, calling no closure.
  clean <- function(keep) {
    now <- .Internal(ls(E, TRUE, FALSE))
    drop <- now[is.na(.Internal(match(now, keep, NA_integer_, NULL)))]
    if (length(drop)) .Internal(remove(drop, E, FALSE))
  }

  done <- new.env(parent = emptyenv()) # the instrumented closures of the script
  callback <- function(expr, value, ok, visible) {
    before <- .Internal(ls(E, TRUE, FALSE))
    # A closure bound at top level is instrumented and records itself; any
    # other top-level code ran already, so all of it counts.
    if (is.call(expr) && as.character(expr[[1L]])[1L] %in% c("<-", "=") && is.symbol(expr[[2L]]) &&
        is_fun_literal(expr[[3L]])) {
      k <- c(paste0("primitive:", as.character(expr[[1L]])), "primitive:function")
    } else {
      k <- keys(expr, TRUE)
    }
    l <- rep(list(TRUE), length(k))
    names(l) <- k
    g <- globalenv()
    for (n in ls(g, all.names = TRUE)) {
      f <- get(n, envir = g)
      if (typeof(f) == "closure" && !identical(f, get0(n, envir = done, inherits = FALSE))) {
        f <- instrument(f, paste0("user:", n))
        assign(n, f, envir = g)
        assign(n, f, envir = done)
      }
    }
    clean(before)
    .Internal(list2env(l, E))
    snap <- .Internal(ls(E, TRUE, TRUE))
    writeLines(snap, out)
    clean(snap)
    TRUE
  }

  first_sys <- get(".First.sys", base)
  wrapper <- function() {
    first_sys()
    # Every closure of base, then base's S3 methods table where it holds the
    # same function.
    for (n in ls(base, all.names = TRUE)) {
      f <- get(n, envir = base)
      if (typeof(f) != "closure" || n == ".First.sys") next
      g <- instrument(f, paste0("closure:", n))
      tbl <- base[[".__S3MethodsTable__."]]
      if (exists(n, envir = tbl, inherits = FALSE) &&
          identical(tryCatch(get(n, envir = tbl), error = function(e) NULL), f)) {
        assign(n, g, envir = tbl)
      }
      locked <- bindingIsLocked(n, base)
      if (locked) unlockBinding(n, base)
      assign(n, g, envir = base)
      if (locked) lockBinding(n, base)
    }
    addTaskCallback(callback, name = "robigo-base-subset")
    clean(character())
    invisible()
  }
  unlockBinding(".First.sys", base)
  assign(".First.sys", wrapper, envir = base)
  lockBinding(".First.sys", base)
  invisible()
})
