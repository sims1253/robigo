# importIntoEnv preserves active bindings and does not force promises.
importIntoEnv <- function(impenv, impnames, expenv, expnames) invisible(.Internal(importIntoEnv(impenv, impnames, expenv, expnames)))
exp <- new.env()
n <- 0
makeActiveBinding("counter", function() { n <<- n + 1; n }, exp)
imp <- new.env()
importIntoEnv(imp, "copy", exp, "counter")
print(bindingIsActive("copy", imp))
print(imp$copy)
print(imp$copy)
print(exp$counter)
print(body(activeBindingFunction("copy", imp)))

# a value that changes under the importer
state <- "list"
makeActiveBinding("symbol", function() state, exp)
importIntoEnv(imp, "symbol", exp, "symbol")
print(imp$symbol)
state <- "changed"
print(get("symbol", imp))

# found through the export environment's enclosures
child <- new.env(parent = exp)
importIntoEnv(imp, "through", child, "symbol")
print(bindingIsActive("through", imp))

# replacing an active import rebinds it
importIntoEnv(imp, "copy", exp, "symbol")
print(imp$copy)

# promises stay unforced
delayedAssign("lazy", { cat("forced\n"); 42 }, assign.env = exp)
importIntoEnv(imp, "lazy", exp, "lazy")
cat("imported\n")
print(imp$lazy)
print(exp$lazy)

# plain values, and an imported function called through a namespace-like env
exp$plain <- 1:3
exp$f <- function(x) x + 1
importIntoEnv(imp, c("p", "g"), exp, c("plain", "f"))
print(imp$p)
print(imp$g(1))
print(bindingIsActive("p", imp))

# a locked import environment
lockBinding("p", imp)
print(tryCatch(importIntoEnv(imp, "p", exp, "plain"), error = conditionMessage))

# errors
print(tryCatch(importIntoEnv(imp, "x", exp, "no_such_export"), error = function(e) deparse(conditionCall(e))))
print(tryCatch(importIntoEnv(imp, "x", exp, "no_such_export"), error = conditionMessage))
print(tryCatch(importIntoEnv(imp, c("a", "b"), exp, "plain"), error = conditionMessage))
print(tryCatch(importIntoEnv(imp, 1, exp, "plain"), error = conditionMessage))
print(tryCatch(importIntoEnv(NULL, "a", exp, "plain"), error = conditionMessage))
print(tryCatch(importIntoEnv(1, "a", exp, "plain"), error = conditionMessage))
print(tryCatch(importIntoEnv(imp, "a", 1, "plain"), error = conditionMessage))
print(tryCatch(importIntoEnv(imp, "a", NULL, "plain"), error = conditionMessage))

# what base holds is found through the namespace
importIntoEnv(imp, "base_sum", baseenv(), "sum")
print(identical(imp$base_sum, sum))
importIntoEnv(imp, "ns_sum", asNamespace("base"), "sum")
print(identical(imp$ns_sum, sum))

# R_MakeActiveBinding's checks, through both entry points
makeActiveBinding("act", function() 1, exp)
imp$regular <- 1
print(tryCatch(importIntoEnv(imp, "regular", exp, "act"), error = conditionMessage))
print(tryCatch(makeActiveBinding("regular", function() 2, imp), error = conditionMessage))
print(imp$regular)
makeActiveBinding("act2", function() "old", imp)
lockBinding("act2", imp)
print(tryCatch(importIntoEnv(imp, "act2", exp, "act"), error = conditionMessage))
print(tryCatch(makeActiveBinding("act2", function() "new", imp), error = conditionMessage))
print(imp$act2)
unlockBinding("act2", imp)
makeActiveBinding("act2", function() "new", imp)
print(imp$act2)
lockEnvironment(imp)
print(tryCatch(importIntoEnv(imp, "fresh", exp, "act"), error = conditionMessage))
print(tryCatch(makeActiveBinding("fresh2", function() 1, imp), error = conditionMessage))
print(exists("fresh", imp))
