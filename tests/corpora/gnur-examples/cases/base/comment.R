RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: comment
### Title: Query or Set a '"comment"' Attribute
### Aliases: comment comment<-
### Keywords: attribute

### ** Examples

x <- matrix(1:12, 3, 4)
comment(x) <- c("This is my very important data from experiment #0234",
                "Jun 5, 1998")
x
comment(x)



