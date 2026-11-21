# GNU R's JIT on a namespace's closures: cmp.R's frameTypes takes a
# namespace's imports frame (its name "imports:...") as one of its
# namespace frames; getInlineInfo finds a name's home namespace through it
# (findHomeNS) and stops on an imports frame that is not the top
# environment's. Seen through a constant argument's binding: a value in
# compiled code (PUSHCONSTARG), a promise in the AST's. Each closure has
# a body of its own: GNU R's JIT cache would give one whose body it has
# compiled for another top environment no code (NOJIT).
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
h <- function() { for (i in 1) NULL; bt(2) }
environment(h) <- ns2
print(h()); print(h()); print(h())
# a name of two strings: frameTypes' test is an error, uncompiled
two <- new.env(parent = .BaseNamespaceEnv)
attr(two, "name") <- c("imports:a", "imports:b")
ns3 <- mkns("foo3", two)
h3 <- function() { for (i in 1) NULL; bt(3) }
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
# isBaseVar takes the frame a name is found in, not its home: `pi` bound
# in an imports frame whose namespace imports base whole has base for its
# home, yet compiled code reads the binding (no constant folding)
imp5 <- new.env(parent = .BaseNamespaceEnv)
attr(imp5, "name") <- "imports:foo5"
assign("pi", 3, envir = imp5)
ns5 <- mkns("foo5", imp5)
assign("imports", list(base = TRUE), envir = get(".__NAMESPACE__.", ns5))
p <- function() { for (i in 1) NULL; c(pi, bt(1)) }
environment(p) <- ns5
print(p()); print(p()); print(p())
# isBaseVar's getInlineInfo stops on `pi` found in an imports frame below
# the top's ("bad namespace import frame"): uncompiled
assign("pi", 4, envir = imp2)
ns6 <- mkns("foo6", imp1)
q <- function() { for (i in 1) NULL; c(bt(1), pi) }
environment(q) <- ns6
print(q()); print(q()); print(q())
# findHomeNS's getNamespace() of an import that is not registered stops
# (GNU R's loadNamespace finds no such package): uncompiled
imp7 <- new.env(parent = .BaseNamespaceEnv)
attr(imp7, "name") <- "imports:foo7"
assign("k7", function() "k7", envir = imp7)
ns7 <- mkns("foo7", imp7)
assign("imports", list(nosuchpkg7 = TRUE), envir = get(".__NAMESPACE__.", ns7))
r7 <- function() { for (i in 1) NULL; c(k7(), bt(1)) }
environment(r7) <- ns7
print(r7()); print(r7()); print(r7())
# a whole import of a registered namespace without exports: exists() of
# NULL stops, uncompiled
nsE <- mkns("fooE", .BaseNamespaceEnv)
invisible(.Internal(registerNamespace("fooE", nsE)))
imp10 <- new.env(parent = .BaseNamespaceEnv)
attr(imp10, "name") <- "imports:foo10"
assign("k10", function() "k10", envir = imp10)
ns10 <- mkns("foo10", imp10)
assign("imports", list(fooE = TRUE), envir = get(".__NAMESPACE__.", ns10))
r10 <- function() { for (i in 1) NULL; c(k10(), bt(1)) }
environment(r10) <- ns10
print(r10()); print(r10()); print(r10())
# nsName: a namespace whose spec names it "base" has base's handlers, so
# compiled code calls base's length (GETBUILTIN), not the namespace's own
imp8 <- new.env(parent = .BaseNamespaceEnv)
attr(imp8, "name") <- "imports:base"
ns8 <- mkns("base", imp8)
assign("length", function(x) "own length", envir = ns8)
g8 <- function(x) { for (i in 1) NULL; c(length(x), bt(8)) }
environment(g8) <- ns8
print(g8(1:3)); print(g8(1:3)); print(g8(1:3))
# a spec without a name: nsName is NA, and getInlineHandler's test of it
# stops for a name with a handler (`length`): uncompiled; a name without
# one (`k9`) is called, compiled
imp9 <- new.env(parent = .BaseNamespaceEnv)
attr(imp9, "name") <- "imports:foo9"
ns9 <- new.env(parent = imp9)
info9 <- new.env(parent = baseenv())
assign("spec", c(version = "1.0"), envir = info9)
assign(".__NAMESPACE__.", info9, envir = ns9)
assign("length", function(x) "own length", envir = ns9)
assign("k9", function() "k9", envir = ns9)
h9 <- function(x) { for (i in 1) NULL; c(length(x), bt(9)) }
environment(h9) <- ns9
print(h9(1:3)); print(h9(1:3)); print(h9(1:3))
j9 <- function() { for (i in 1) NULL; c(k9(), bt(1)) }
environment(j9) <- ns9
print(j9()); print(j9()); print(j9())
