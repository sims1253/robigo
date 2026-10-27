RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: row.names
### Title: Get and Set Row Names for Data Frames
### Aliases: row.names row.names.data.frame row.names.default row.names<-
###   row.names<-.data.frame row.names<-.default .rowNamesDF<-
### Keywords: classes methods

### ** Examples

## To illustrate the note:
df <- data.frame(x = c(TRUE, FALSE, NA, NA), y = c(12, 34, 56, 78))
row.names(df) <- 1 : 4
attr(df, "row.names") #> 1:4
deparse(df) # or dput(df)
##--> c(NA, 4L) : Compact storage, *not* regarded as automatic.

row.names(df) <- NULL
attr(df, "row.names") #> 1:4
deparse(df) # or dput(df) -- shows
##--> c(NA, -4L) : Compact storage, regarded as automatic.



