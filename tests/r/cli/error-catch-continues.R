# With an error option set the session continues at the next line; the
# rest of the error's line is dropped, as in R's console.
options(error = function(e) {})
1; stop("boom"); 2
3
