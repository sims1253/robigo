RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: news
### Title: Build and Query R or Package News Information
### Aliases: news print.news_db

### ** Examples

## Build a db of all R news entries.
db <- news()
## Don't show: 
  vv <- capture.output(print(db, doBrowse=FALSE))  # without an error
  stopifnot(is.character(vv), length(vv) >= 3) # was wrong (for weeks during devel.)
  html <- tools::toHTML(subset(db, is.na(Version))) # emulate doBrowse with query
## End(Don't show)
## Bug fixes with PR number in 4.0.0.
db4 <- news(Version == "4.0.0" & grepl("^BUG", Category) & grepl("PR#", Text),
            db = db)
nrow(db4)
## Don't show: 
stopifnot(is.null(attr(db4, "bad")) && nrow(db4) == 25 )
## End(Don't show)
## print db4 to show in an HTML browser.

## News from a date range ('Matrix' is there in a regular R installation):
if(length(iM <- find.package("Matrix", quiet = TRUE)) && nzchar(iM)) {
   dM <- news(package="Matrix")
   stopifnot(identical(dM, news(db=dM)))
   dM2014 <- news("2014-01-01" <= Date & Date <= "2014-12-31", db = dM)
   stopifnot(paste0("1.1-", 2:4) %in% dM2014[,"Version"])
}

## Which categories have been in use? % R-core maybe should standardize a bit more
sort(table(db[, "Category"]), decreasing = TRUE)
## Entries with version >= 4.0.0
table(news(Version >= "4.0.0", db = db)$Version)

## No test: 
##D ## do the same for R 3.x.y, more slowly
##D db3 <- news(package = "R-3")
##D sort(table(db3[, "Category"]), decreasing = TRUE)
##D ## Entries with version >= 3.6.0
##D table(news(Version >= "3.6.0", db = db3)$Version)
## End(No test)



