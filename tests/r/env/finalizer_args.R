msg <- function(expr) tryCatch({
    expr
    "ok"
}, error = function(e) conditionMessage(e))
print(msg(reg.finalizer(1, function(e) NULL)))
print(msg(reg.finalizer(new.env(), sum)))
print(msg(reg.finalizer(new.env(), function(e) NULL, NA)))
print(msg(reg.finalizer(new.env(), function(e) NULL, "x")))
print(msg(reg.finalizer(new.env(), function(e) NULL, c(TRUE, FALSE))))
print(msg(reg.finalizer(new.env(), function(e) NULL, 1)))
