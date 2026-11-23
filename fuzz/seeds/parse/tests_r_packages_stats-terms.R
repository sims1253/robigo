library(stats)
show <- function(f, ...) {
  t <- terms(f, ...)
  print(attr(t, "term.labels")); print(attr(t, "order")); print(attr(t, "intercept")); print(attr(t, "response"))
  print(attr(t, "factors")); print(attr(t, "variables"))
}
show(y ~ x)
show(y ~ a * b)
show(~ a + b:c - 1)
show(y ~ (a + b + c)^2)
show(y ~ a / b)
show(y ~ b %in% a)
show(y ~ a * b - a)
show(log(y) ~ poly(x, 2) + offset(z))
show(y ~ 0 + x)
show(y ~ x + I(x^2) + x:z, keep.order = TRUE)
show(y ~ s(x) + s(z) + w, specials = "s")
d <- data.frame(y = 1, a = 2, b = 3)
t <- terms(y ~ ., data = d); print(t)
print(update(y ~ x, ~ . + z))
print(update(y ~ x + w, log(.) ~ . - w))
f <- y ~ x; print(terms(f))

# R: model.c:InstallVar — R's isLanguage(R_NilValue) is true, so a NULL
# response is a legitimate term: it is installed as a variable (deparsed
# "NULL"), response stays 1, and update.formula's simplify drops the lhs
# again (`res[[2]] <- NULL` removes the slot).
show(NULL ~ y)
show(NULL ~ y + z)
print(update.formula(x ~ y, NULL ~ .))
print(update.formula(z ~ x + y, NULL ~ .))
print(update.formula(. ~ u + v, NULL ~ .))
print(terms.formula(NULL ~ y, simplify = TRUE))
print(terms.formula(NULL ~ y)$factors)
show(1 ~ y)
show(TRUE ~ y)
cat(tryCatch(terms.formula(2 ~ y), error = conditionMessage), "\n")
cat(tryCatch(terms.formula("a" ~ y), error = conditionMessage), "\n")
