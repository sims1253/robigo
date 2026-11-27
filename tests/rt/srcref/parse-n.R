# parse(n =, keep.source = TRUE) ends its wholeSrcref where R_Parse stopped,
# and a #line past that point is unread (R_Parse reads no further). A call
# of `{` written as a funcall (xxfuncall) keeps no srcref list.
p <- parse(text = c("1", "2"), n = 1, keep.source = TRUE)
print(as.integer(unclass(attr(p, "wholeSrcref")))); print(length(attr(p, "srcref")))
p <- parse(text = c("f <- function() 1", "#line 9 \"z.R\"", "2"), n = 1, keep.source = TRUE)
print(as.integer(unclass(attr(p, "wholeSrcref")))); print(attr(attr(p, "wholeSrcref"), "srcfile")$filename); print(class(attr(p, "srcfile")))
print(as.integer(unclass(p[[1]][[3]][[4]])))
tf <- tempfile(); writeLines(c("a <- 1; b <- 2", "{ c <- 3 }", "d"), tf); con <- file(tf, "r")
p1 <- parse(con, n = 1, keep.source = TRUE); print(as.integer(unclass(attr(p1, "wholeSrcref"))))
p2 <- parse(con, n = 1, keep.source = TRUE); print(as.integer(unclass(attr(p2, "wholeSrcref")))); print(lapply(attr(p2[[1]], "srcref"), function(s) as.integer(unclass(s)))); close(con)
q <- parse(text = "`{`(1)", keep.source = TRUE); print(attributes(q[[1]]))
q <- parse(text = "{1}; `{`(2, 3)", keep.source = TRUE); print(names(attributes(q[[1]]))); print(attributes(q[[2]]))
