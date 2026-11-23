# With an error option set a parse error does not end the script: the
# option runs and the next line is read. At end of input inside an open
# statement the error option runs again and the script ends normally.
options(error = function() cat("error option ran\n"))
cat("before\n")
x y
cat("after\n")
x <- (
