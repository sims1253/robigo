# rlang::as_closure() of a binary operator (#401): the operator's body is
# a shallow duplicate of check nodes that R code holds, extended in place
# by a SETCDR from a callback closure, so the appended statement must
# reach `nodes` and the original check nodes must stay as they were.
dyn.load("node_append_closure.so")
duplicate <- function(x, shallow = FALSE) .Call("capi_duplicate", x, shallow)
node_cdr <- function(x) .Call("capi_node_cdr", x)
node_poke_cdr <- function(x, y) invisible(.Call("capi_node_poke_cdr", x, y))
new_call <- function(car, cdr) .Call("capi_new_call", car, cdr)

node_walk_last <- function(.x, .f, ...) {
  cur <- .x
  while (!is.null(node_cdr(cur))) {
    cur <- node_cdr(cur)
  }
  .f(cur, ...)
}
node_append <- function(.x, .y) {
  node_walk_last(.x, function(l) node_poke_cdr(l, .y))
  .x
}

binary_check_nodes <- pairlist(
  quote(
    if (missing(.x)) {
      if (missing(e1)) {
        stop("Must supply `e1` or `.x` to binary operator.")
      }
      .x <- e1
    } else if (!missing(e1)) {
      stop("Can't supply both `e1` and `.x` to binary operator.")
    }
  ),
  quote(
    if (missing(.y)) {
      if (missing(e2)) {
        stop("Must supply `e2` or `.y` to binary operator.")
      }
      .y <- e2
    } else if (!missing(e2)) {
      stop("Can't supply both `e2` and `.y` to binary operator.")
    }
  )
)
versatile_check_nodes <- as.pairlist(c(
  binary_check_nodes[[1]],
  quote(
    if (missing(.y) && !missing(e2)) {
      .y <- e2
    } else if (!missing(e2)) {
      stop("Can't supply both `e2` and `.y` to binary operator.")
    }
  )
))

new_binary_closure <- function(fn, versatile = FALSE) {
  nodes <- if (versatile) versatile_check_nodes else binary_check_nodes
  nodes <- duplicate(nodes, shallow = TRUE)
  nodes <- node_append(nodes, pairlist(body(fn)))
  body <- new_call(quote(`{`), nodes)
  formals(fn) <- alist(e1 = , e2 = , .x = e1, .y = e2)
  body(fn) <- body
  fn
}

ops <- list(
  `-` = new_binary_closure(function(.x, .y) if (missing(.y)) -.x else .x - .y,
                           versatile = TRUE),
  `&&` = new_binary_closure(function(.x, .y) .x && .y),
  `*` = new_binary_closure(function(.x, .y) .x * .y)
)
minus <- ops[["-"]]
print(body(minus))
minus(3)
minus(.y = 10, .x = 5)
ops[["&&"]](FALSE, TRUE)
ops[["*"]](6, 7)
length(binary_check_nodes)
length(versatile_check_nodes)
length(body(ops[["*"]]))
