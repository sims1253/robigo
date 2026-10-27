RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: Syntax
### Title: Operator Syntax and Precedence
### Aliases: Syntax
### Keywords: documentation programming

### ** Examples

## Logical AND ("&&") has higher precedence than OR ("||"):
TRUE || TRUE && FALSE   # is the same as
TRUE || (TRUE && FALSE) # and different from
(TRUE || TRUE) && FALSE

## Special operators have higher precedence than "!" (logical NOT).
## You can use this for %in% :
! 1:10 %in% c(2, 3, 5, 7) # same as !(1:10 %in% c(2, 3, 5, 7))
## but we strongly advise to use the "!( ... )" form in this case!


## '=' has lower precedence than '<-' ... so you should not mix them
##     (and '<-' is considered better style anyway):
## Not run: 
##D ## Consequently, this gives a ("non-catchable") error
##D  x <- y = 5  #->  Error in (x <- y) = 5 : ....
## End(Not run)



