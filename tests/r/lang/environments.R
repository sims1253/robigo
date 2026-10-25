e <- new.env()
assign("a", 1, envir = e)
get("a", envir = e)
exists("a", envir = e)
exists("zzz_not_here")
e$b <- 2
sort(ls(e))
mget(c("a", "b"), envir = e)
local({
  x <- 5
  x * 2
})
f <- function() {
  x <- "inner"
  g <- function() parent.frame()
  identical(g(), environment())
}
f()
counter_env <- new.env()
counter_env$n <- 0
bump <- function(env) env$n <- env$n + 1
for (i in 1:5) bump(counter_env)
counter_env$n
environmentName(globalenv())
environmentName(baseenv())
environmentName(environment(sum))
is.environment(e)
rm("a", envir = e)
ls(e)
fn <- function() sys.call()
fn()
outer_fn <- function(x) inner_fn(x)
inner_fn <- function(y) sys.function()
class(outer_fn(1))
