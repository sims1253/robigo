library(parallel)

# fork is not implemented; cleanup of no children validates and returns NULL
print(.Call(parallel:::C_mc_cleanup, TRUE, TRUE, FALSE))
print(.Call(parallel:::C_mc_cleanup, FALSE, FALSE, FALSE))
print(.Call(parallel:::C_mc_cleanup, 9L, TRUE, TRUE))
print(tryCatch(.Call(parallel:::C_mc_cleanup, NA, TRUE, FALSE),
               error = function(e) conditionMessage(e)))
print(tryCatch(.Call(parallel:::C_mc_cleanup, TRUE, NA, FALSE),
               error = function(e) conditionMessage(e)))
print(tryCatch(.Call(parallel:::C_mc_cleanup, TRUE, TRUE, NA),
               error = function(e) conditionMessage(e)))
print(detectCores() > 0)
