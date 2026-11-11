dyn.load("quo_squash.so")
duplicate <- function(x) .Call("capi_duplicate", x)
node_car <- function(x) .Call("capi_node_car", x)
node_cdr <- function(x) .Call("capi_node_cdr", x)
quo_get_expr <- function(q) .Call("capi_node_cadr", q)
node_poke_car <- function(x, v) invisible(.Call("capi_node_poke_car", x, v))

# A quosure is a `~` call with class "quosure".
quo <- function(expr) structure(call("~", expr), class = c("quosure", "formula"))
is_quosure <- function(x) inherits(x, "quosure")

# rlang 1.3.0's quo_squash() and quo_squash_impl() (R/quo.R), without warn
quo_squash_impl <- function(x, parent = NULL) {
  if (is.call(x) || is.pairlist(x)) {
    if (is_quosure(x)) {
      while (is_quosure(x)) x <- quo_get_expr(x)
      if (!is.null(parent)) node_poke_car(parent, x)
      quo_squash_impl(x, parent)
    } else {
      node <- x
      while (!is.null(node)) {
        quo_squash_impl(node_car(node), parent = node)
        node <- node_cdr(node)
      }
    }
  }
  x
}
quo_squash <- function(q) {
  if (is_quosure(q)) q <- quo_get_expr(q)
  quo_squash_impl(duplicate(q))
}
show <- function(q) {
  e <- quo_get_expr(q)
  s <- quo_squash(q)
  print(s)
  print(deparse(s))
  # the quosure itself is untouched: quo_squash() worked on a copy
  print(identical(quo_get_expr(q), e))
}

# rlang::quo(identity(!!rlang::quo(cyl))): ggplot2's default labels
show(quo(call("identity", quo(quote(cyl)))))

# deeper, and more than one quosure per call
show(quo(call("f", 1, call("g", quo(quote(a)), quo(call("h", quo(quote(b))))), quo(quote(c)))))

# a quosure of a quosure collapses all the way
show(quo(call("+", quo(quo(quote(x))), 1)))

# a pairlist argument holding a quosure (function(x = !!quo(y)) ...)
fn <- call("function", as.pairlist(list(x = quo(quote(y)))), quote(x))
show(quo(fn))
