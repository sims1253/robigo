### Name: asOneSidedFormula
### Title: Convert to One-Sided Formula
### Aliases: asOneSidedFormula
### Keywords: models

### ** Examples

(form <- asOneSidedFormula("age"))
stopifnot(exprs = {
    identical(form, asOneSidedFormula(form))
    identical(form, asOneSidedFormula(as.name("age")))
    identical(form, asOneSidedFormula(expression(age)))
})
asOneSidedFormula(quote(log(age)))
asOneSidedFormula(1)



