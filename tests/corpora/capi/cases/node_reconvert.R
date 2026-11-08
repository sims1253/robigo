dyn.load("node_reconvert.so")
new_node <- function(car) .Call("capi_new_node", car)
node_cdr <- function(x) .Call("capi_node_cdr", x)
node_poke_cdr <- function(x, y) invisible(.Call("capi_node_poke_cdr", x, y))

# A node C handed back and R dropped (rlang's node_append walks the
# spine with `while (!is.null(node_cdr(cur)))`): when C hands the same
# node back later, it must still be the call's own node, so that
# poking its CDR extends the call (rlang's call_modify()).
d <- quote(get())
node_poke_cdr(d, new_node("foo"))
is.null(node_cdr(d))
cur <- node_cdr(d)
node_poke_cdr(cur, new_node("bar"))
print(d)

# the same with the dropped node passed to a builtin
d <- quote(get())
node_poke_cdr(d, new_node("foo"))
length(node_cdr(d))
cur <- node_cdr(d)
node_poke_cdr(cur, new_node("bar"))
print(d)

# a pairlist rather than a call
p <- pairlist(1)
is.null(node_cdr(p))
node_poke_cdr(p, new_node(2))
cur <- node_cdr(p)
is.null(node_cdr(cur))
node_poke_cdr(cur, new_node(3))
print(unlist(as.list(p)))

# rlang's node_append inside a function, appended three times
node_append <- function(x, y) {
  cur <- x
  while (!is.null(node_cdr(cur))) cur <- node_cdr(cur)
  node_poke_cdr(cur, y)
  invisible(x)
}
e <- quote(f())
node_append(e, new_node(1L))
node_append(e, new_node(2L))
node_append(e, new_node(3L))
print(e)
