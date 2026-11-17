# R_LookupMethod searches the S3 methods table of the generic's defining
# environment (base's .__S3MethodsTable__., bound lazily as GNU R binds
# it). The method-lookup memo sees a method put there after calls that
# missed it, and its removal. (Closures stay inside local(): the
# base-subset tracer rewrites closures bound in the global environment.)
local({
  tbl <- get(".__S3MethodsTable__.", envir = baseenv())
  print(typeof(tbl))
  x <- structure(1, class = "zz")
  r <- function() for (i in 1:3) print(x)
  r()
  assign("print.zz", function(x, ...) cat("a zz\n"), envir = tbl)
  r()
  rm("print.zz", envir = tbl)
  r()
})
