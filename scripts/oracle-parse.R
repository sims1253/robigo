# Dump GNU R's parse trees in the canonical format of robigo_syntax::dump.
#
# Usage: Rscript --vanilla scripts/oracle-parse.R IN.R [OUT.dump]
# Writes one line per top-level expression. When the file does not parse,
# writes a single line
#   ERROR <line> <col> <message>
# with the 1-based character column (tabs advance to the next multiple of 8)
# and GNU R's message; errors raised by parser actions keep their
# "(<input>:<line>:<col>)" suffix, syntax errors from the grammar drop the
# "<file>:<line>:<col>: " prefix and the source echo.

dump_dbl <- function(x) {
  if (is.na(x) && !is.nan(x)) "NA"
  else if (is.nan(x)) "NaN"
  else if (x == Inf) "Inf"
  else if (x == -Inf) "-Inf"
  else sprintf("%.17g", x)
}

dump_const <- function(x) {
  if (is.null(x)) return("NULL")
  if (length(x) != 1L) stop("non-scalar constant")
  switch(typeof(x),
    logical = if (is.na(x)) "lgl:NA" else if (x) "lgl:TRUE" else "lgl:FALSE",
    integer = if (is.na(x)) "int:NA" else paste0("int:", x),
    double = paste0("dbl:", dump_dbl(x)),
    complex = if (is.na(x)) "cplx:NA" else paste0("cplx:", dump_dbl(Re(x)), ",", dump_dbl(Im(x))),
    character = if (is.na(x)) "str:NA" else paste0("str:", encodeString(x, quote = '"')),
    stop("unexpected constant type ", typeof(x))
  )
}

dump_arg <- function(name, value, missing) {
  # Tag and formal names can hold any byte (string tags like "\n"=1,
  # backtick symbols); escape controls so the one-line-per-expression
  # format stays unambiguous. Identity for clean names.
  prefix <- if (!is.null(name) && !is.na(name) && nzchar(name))
    paste0(encodeString(name, quote = ""), "=")
  else ""
  if (missing) paste0(prefix, "<missing>") else paste0(prefix, dump_expr(value))
}

is_missing_arg <- function(x) is.symbol(x) && identical(as.character(x), "")

dump_expr <- function(e) {
  if (is.symbol(e)) {
    return(paste0("`", encodeString(as.character(e), quote = ""), "`"))
  }
  if (is.call(e)) {
    if (identical(e[[1L]], as.name("function"))) {
      formals <- e[[2L]]
      nms <- names(formals)
      parts <- character()
      for (i in seq_along(formals)) {
        # Never bind a possibly-empty argument to a variable: reading it
        # back would signal "argument is missing".
        m <- is_missing_arg(formals[[i]])
        parts <- c(parts, if (m) dump_arg(nms[i], NULL, TRUE) else dump_arg(nms[i], formals[[i]], FALSE))
      }
      body <- dump_expr(e[[3L]])
      formals_str <- if (length(parts)) paste0(" ", parts, collapse = "") else ""
      return(paste0("(`function` (pairlist", formals_str, ") ", body, " NULL)"))
    }
    items <- as.list(e)
    nms <- names(e)
    out <- dump_expr(items[[1L]])
    for (i in seq_along(items)[-1L]) {
      m <- is_missing_arg(items[[i]])
      out <- paste(out, if (m) dump_arg(nms[i], NULL, TRUE) else dump_arg(nms[i], items[[i]], FALSE))
    }
    return(paste0("(", out, ")"))
  }
  dump_const(e)
}

# Canonical text for a parse failure: position + message, without the
# source echo the interactive printer adds.
dump_error <- function(e, infile) {
  if (!is.null(e$lineno)) {
    # Errors raised by parser actions (raiseParseError in gram.y): the
    # position is carried as condition fields and the message already
    # contains the "(<input>:<line>:<col>)" suffix.
    paste("ERROR", e$lineno, e$colno, conditionMessage(e))
  } else {
    # Grammar errors (yyerror): "<file>:<line>:<col>: <message>" plus a
    # source echo; keep the message only.
    first <- strsplit(conditionMessage(e), "\n", fixed = TRUE)[[1L]][[1L]]
    m <- regmatches(first, regexec("^(.*):([0-9]+):([0-9]+): (.*)$", first))[[1L]]
    if (length(m) == 0L) {
      # Should not happen; keep the raw text so the mismatch is visible.
      paste("ERROR", "0", "0", first)
    } else {
      paste("ERROR", m[[3L]], m[[4L]], m[[5L]])
    }
  }
}

main <- function(args) {
  infile <- args[[1L]]
  outfile <- if (length(args) >= 2L) args[[2L]] else ""
  lines <- tryCatch(
    {
      exprs <- withCallingHandlers(
        parse(file = infile, keep.source = FALSE, encoding = "UTF-8"),
        warning = function(w) invokeRestart("muffleWarning")
      )
      vapply(as.list(exprs), dump_expr, "")
    },
    error = function(e) dump_error(e, infile)
  )
  writeLines(lines, if (nzchar(outfile)) outfile else stdout(), useBytes = TRUE)
}

if (sys.nframe() == 0L && !exists("main_disabled")) main(commandArgs(trailingOnly = TRUE))
