show <- function(tag, x) cat(tag, ":", paste(x, collapse = " "), "\n")
f <- function(a, b) {
  z <- 1; y <- 2
  show("F.ls", ls(sorted = FALSE))
  rm(z); z <- 3
  show("F.re", ls(sorted = FALSE))
  y <- 4
  show("F.upd", ls(sorted = FALSE))
  show("F.exists.w", exists("w", inherits = FALSE))
  show("F.missing.b", missing(b))
  show("F.aslist", names(as.list(environment(), all.names = TRUE)))
}
f(1)
g <- function(a) {
  assign("q", 1); x <- 1; assign("r", 2)
  show("G.ls", ls(sorted = FALSE))
  for (n in c("o1", "o2", "o3", "o4", "o5", "o6")) assign(n, 1)
  show("G.many", ls(sorted = FALSE))
  rm(q, o3); assign("q", 9); x <- 5
  show("G.re", ls(sorted = FALSE))
  show("G.mget", paste(names(mget(c("x", "q", "a"))), unlist(mget(c("x", "q", "a")))))
}
g(7)
h <- function(a, b = 2, ...) {
  show("H.ls", ls(sorted = FALSE, all.names = TRUE))
  b
  show("H.after", ls(sorted = FALSE, all.names = TRUE))
}
h(1)
h(1, c = 3)
k <- function() { delayedAssign("lazy", stop("never")); makeActiveBinding("act", function() 1, environment()); v <- 1; show("K.ls", ls(sorted = FALSE)) }
k()
e <- new.env(hash = FALSE); for (n in c("p1", "p2", "p3", "p4", "p5")) assign(n, 1, e); rm("p2", envir = e); assign("p2", 2, e)
show("E.ls", ls(e, sorted = FALSE))
show("E.aslist", names(as.list(e)))
