# GNU R's JIT on a namespace's closures: cmp.R's frameTypes takes a
# namespace's imports frame (its name "imports:...") as one of its
# namespace frames; getInlineInfo finds a name's home namespace through it
# (findHomeNS) and stops on an imports frame that is not the top
# environment's. Seen through a constant argument's binding: a value in
# compiled code (PUSHCONSTARG), a promise in the AST's.
bt <- function(a) .Internal(getBindingType(quote(a), environment()))
mkns <- function(name, parent) {
  ns <- new.env(parent = parent)
  info <- new.env(parent = baseenv())
  assign("spec", c(name = name, version = "1.0"), envir = info)
  assign(".__NAMESPACE__.", info, envir = ns)
  ns
}
imp <- new.env(parent = .BaseNamespaceEnv)
attr(imp, "name") <- "imports:foo"
ns <- mkns("foo", imp)
# a namespace closure with a loop: MAYBEJIT at its first call, compiled at
# its second
f <- function() { for (i in 1) NULL; bt(1) }
environment(f) <- ns
print(f()); print(f()); print(f())
# a name the imports frame binds shadows base's: its home is no base, so
# compiled code calls it (no inlining of base's length)
assign("length", function(x) "imported length", envir = imp)
g <- function(x) { for (i in 1) NULL; c(length(x), bt(1)) }
environment(g) <- ns
print(g(1:3)); print(g(1:3)); print(g(1:3))
# no "imports:" name: "non-namespace found within namespace environments",
# the closure stays uncompiled
plain <- new.env(parent = .BaseNamespaceEnv)
ns2 <- mkns("foo2", plain)
h <- function() { for (i in 1) NULL; bt(1) }
environment(h) <- ns2
print(h()); print(h()); print(h())
# a name of two strings: frameTypes' test is an error, uncompiled
two <- new.env(parent = .BaseNamespaceEnv)
attr(two, "name") <- c("imports:a", "imports:b")
ns3 <- mkns("foo3", two)
h3 <- function() { for (i in 1) NULL; bt(1) }
environment(h3) <- ns3
print(h3()); print(h3()); print(h3())
# a name found in an imports frame below the top's: "bad namespace import
# frame", uncompiled
imp2 <- new.env(parent = .BaseNamespaceEnv)
imp1 <- new.env(parent = imp2)
attr(imp1, "name") <- "imports:a"
attr(imp2, "name") <- "imports:b"
assign("k", function() "k", envir = imp2)
ns4 <- mkns("foo4", imp1)
u <- function() { for (i in 1) NULL; c(k(), bt(1)) }
environment(u) <- ns4
print(u()); print(u()); print(u())
