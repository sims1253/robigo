# Attribute assignment validates its target (R: attrib.c:installAttrib via
# the replacement wrappers): a symbol, unbound names in replacement
# targets, and reading attributes back. GNU R 4.5.0 recorded output.
e <- function(expr) {
  r <- tryCatch(expr, error = identity)
  if (inherits(r, "condition")) print(conditionMessage(r)) else print(r)
}
s <- quote(s)
e(class(s) <- "factor")
e(class(NULL) <- "factor")
# the target of an attribute assignment is evaluated first: unbound names
# fail there (R: applydefine), with a symbol or a value in its place.
e(attr(a_made_up_variable, "x") <- 1)
e(attr(quote(nm), "name") <- "y")
# simple objects keep attr/attributes/class round trips honest
x <- structure(1, name = "a")
cat(attr(x, "name"), "\n")
print(attributes(NULL))
print(attr(NULL, "anything"))
print(attr(s, "anything"))
print(attributes(structure(1:3, "val" = "x")))
