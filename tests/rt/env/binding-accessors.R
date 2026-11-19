# envir.c's binding accessors: bindingIsActive, activeBindingFunction,
# delayedBindingExpression, delayedBindingEnvironment,
# forcedBindingExpression (R_GetVarLocExpression, promiseUnwrap).
show <- function(expr) {
  r <- tryCatch(expr, error = function(e) e)
  if (inherits(r, "error")) {
    cat("error:", conditionMessage(r), "| call:", deparse(conditionCall(r)), "\n")
  } else print(r)
  invisible()
}
e <- new.env()
assign("v", 1, envir = e)
makeActiveBinding("a", function() 42, e)
delayedAssign("d", stop("not forced"), assign.env = e)
delayedAssign("f", 1 + 2, assign.env = e)
force(e$f)
# bindingIsActive / activeBindingFunction
show(bindingIsActive("a", e))
show(bindingIsActive(quote(v), e))
show(bindingIsActive("nope", e))
show(bindingIsActive(1, e))
show(bindingIsActive("a", NULL))
show(bindingIsActive("a", 1))
show(bindingIsActive("pi", baseenv()))
show(activeBindingFunction("a", e))
show(activeBindingFunction("v", e))
show(activeBindingFunction("nope", e))
show(activeBindingFunction("a", list()))
# the delayed and forced bindings
show(.Internal(delayedBindingExpression(quote(d), e)))
show(identical(.Internal(delayedBindingEnvironment(quote(d), e)), globalenv()))
show(.Internal(forcedBindingExpression(quote(f), e)))
show(.Internal(delayedBindingExpression(quote(f), e)))
show(.Internal(forcedBindingExpression(quote(d), e)))
show(.Internal(delayedBindingEnvironment(quote(f), e)))
show(.Internal(delayedBindingExpression(quote(v), e)))
show(.Internal(delayedBindingEnvironment(quote(v), e)))
show(.Internal(forcedBindingExpression(quote(v), e)))
show(.Internal(delayedBindingExpression(quote(nope), e)))
show(.Internal(delayedBindingEnvironment(quote(nope), e)))
show(.Internal(forcedBindingExpression(quote(nope), e)))
show(.Internal(delayedBindingExpression(quote(a), e)))
show(.Internal(delayedBindingEnvironment(quote(a), e)))
show(.Internal(delayedBindingExpression("d", e)))
show(.Internal(delayedBindingExpression(quote(d), 1)))
show(.Internal(delayedBindingExpression(quote(d), list())))
# a function's arguments: delayed until read, then forced; a default
g <- function(x, y = z * 2, ...) {
  z <- 5
  r <- list(.Internal(delayedBindingExpression(quote(x), environment())),
            identical(.Internal(delayedBindingEnvironment(quote(x), environment())), parent.frame()),
            .Internal(delayedBindingExpression(quote(y), environment())),
            identical(.Internal(delayedBindingEnvironment(quote(y), environment())), environment()))
  force(x)
  c(r, list(.Internal(forcedBindingExpression(quote(x), environment()))))
}
show(g(1 + 1))
h <- function(x) g(x)
show(h(3 * 3))
# a promise chain: the innermost promise's expression
k <- function(...) { p <- function(q) .Internal(delayedBindingExpression(quote(q), environment())); p(...) }
show(k(a + b))
# compiled callers
cg <- function(x) { for (i in 1) NULL; .Internal(delayedBindingExpression(quote(x), environment())) }
show(cg(u + w))
show(cg(u + w))
# arity
show(.Internal(delayedBindingExpression(quote(d))))
show(.Internal(bindingIsActive(quote(a))))
