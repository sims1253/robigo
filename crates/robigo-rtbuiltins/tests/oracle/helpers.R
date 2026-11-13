## The oracle cases' encoding (read by tests/oracle.rs). Each case prints
##   TAG call : <the builtin's call>
##   TAG in NAME : <an input>
##   TAG W : <message, hex> <call|ctx|NULL>   (each warning, in order)
##   TAG E : <message, hex> <call|ctx|NULL>   (an error), or
##   TAG = <the value>
## "call" is the builtin's own call (errorcall/warningcall), "ctx" any other
## (error()/warning(): the context's call).
## Values: NULL, lgl[T,F,NA], int[1,NA], dbl[<IEEE bits, hex>], cplx[re/im],
## chr[x<bytes hex>.<n|u|l|b>,NA], raw[0a], list[a;b], sym(<hex>),
## lang(a;b), then {<name hex>=<value>;...} for attributes (names first, as
## attributes() reports them).
.h <- function(r) paste(as.character(r), collapse = "")
.bits <- function(d) {
  if (!length(d)) return(character())
  b <- writeBin(as.double(d), raw(), size = 8L, endian = "big")
  apply(matrix(as.character(b), nrow = 8L), 2L, paste, collapse = "")
}
.e <- function(s) c(unknown = "n", "UTF-8" = "u", latin1 = "l", bytes = "b")[[Encoding(s)]]
.chr <- function(x) vapply(seq_along(x), function(i) {
  s <- x[[i]]
  if (is.na(s)) "NA" else paste0("x", .h(charToRaw(s)), ".", .e(s))
}, "")
enc <- function(x) {
  core <- switch(typeof(x),
    "NULL" = "NULL",
    logical = paste0("lgl[", paste(ifelse(is.na(x), "NA", ifelse(x, "T", "F")), collapse = ","), "]"),
    integer = paste0("int[", paste(ifelse(is.na(x), "NA", as.character(as.vector(unclass(x)))), collapse = ","), "]"),
    double = paste0("dbl[", paste(.bits(x), collapse = ","), "]"),
    complex = paste0("cplx[", if (length(x)) paste(paste0(.bits(Re(x)), "/", .bits(Im(x))), collapse = ","), "]"),
    character = paste0("chr[", paste(.chr(x), collapse = ","), "]"),
    raw = paste0("raw[", paste(as.character(as.vector(x)), collapse = ","), "]"),
    list = paste0("list[", paste(vapply(seq_along(x), function(i) enc(x[[i]]), ""), collapse = ";"), "]"),
    symbol = paste0("sym(", .h(charToRaw(as.character(x))), ")"),
    language = paste0("lang(", paste(vapply(as.list(x), enc, ""), collapse = ";"), ")"),
    paste0("<", typeof(x), ">"))
  a <- if (typeof(x) %in% c("symbol", "language", "NULL")) NULL else attributes(x)
  if (length(a))
    core <- paste0(core, "{", paste(paste0(vapply(names(a), function(n) .h(charToRaw(n)), ""), "=",
                                            vapply(a, enc, "")), collapse = ";"), "}")
  core
}
case <- function(tag, call, inputs = list(), internal = FALSE) {
  bcall <- if (internal) call[[2L]] else call
  cat(tag, " call : ", enc(bcall), "\n", sep = "")
  for (n in names(inputs)) cat(tag, " in ", n, " : ", enc(inputs[[n]]), "\n", sep = "")
  env <- list2env(inputs, parent = baseenv())
  who <- function(c) if (is.null(c)) "NULL" else if (identical(c, bcall)) "call" else "ctx"
  res <- tryCatch(withCallingHandlers(list(value = eval(call, env)),
    warning = function(w) {
      cat(tag, " W : ", .h(charToRaw(conditionMessage(w))), " ", who(conditionCall(w)), "\n", sep = "")
      invokeRestart("muffleWarning")
    }),
    error = function(e) {
      cat(tag, " E : ", .h(charToRaw(conditionMessage(e))), " ", who(conditionCall(e)), "\n", sep = "")
      NULL
    })
  if (!is.null(res)) cat(tag, " = ", enc(res$value), "\n", sep = "")
  invisible()
}
## a binary operator on x and y, every pairing of the inputs in `xs`
binary <- function(prefix, op, xs, ys = xs) {
  for (i in names(xs)) for (j in names(ys))
    case(paste0(prefix, ".", i, ".", j), call(op, quote(x), quote(y)), list(x = xs[[i]], y = ys[[j]]))
}
