# reference classes: initialize, callSuper, inheritance, copy semantics, $methods(), active fields
library(methods)
Person <- suppressMessages(setRefClass("Person", fields = list(name = "character", age = "numeric"),
  methods = list(
    initialize = function(...) {
      initFields(...)
      if (length(age) == 0) age <<- 0
      invisible(.self)
    },
    birthday = function() { age <<- age + 1; invisible(.self) },
    show = function() cat("Person:", name, age, "\n"),
    greet = function() paste("Hi,", name))))
p <- Person$new(name = "Ann")
p
p$birthday()$birthday()
p$age
Student <- suppressMessages(setRefClass("Student", contains = "Person", fields = list(school = "character"),
  methods = list(greet = function() paste(callSuper(), "from", school))))
s <- Student$new(name = "Bob", age = 20, school = "MIT")
s$greet()
is(s, "Person")
class(s)
q <- p
q$age <- 99
p$age
r <- p$copy(); r$age <- 1; p$age
Person$methods(older = function(n) age + n)
Person$new(name = "C", age = 5)$older(10)
Person$fields()
Acc <- suppressMessages(setRefClass("Acc", fields = list(bal = "numeric", double = function(v) { if (missing(v)) bal * 2 else stop("read-only") })))
a <- Acc$new(bal = 4)
a$double
r <- tryCatch(a$double <- 1, error = function(e) conditionMessage(e)); r
r <- tryCatch(Person$new(name = 1), error = function(e) conditionMessage(e)); r
