### Name: read.ftable
### Title: Manipulate Flat Contingency Tables
### Aliases: read.ftable write.ftable format.ftable print.ftable
### Keywords: category

### ** Examples

## Agresti (1990), page 157, Table 5.8.
## Not in ftable standard format, but o.k.
file <- tempfile()
cat("             Intercourse\n",
    "Race  Gender     Yes  No\n",
    "White Male        43 134\n",
    "      Female      26 149\n",
    "Black Male        29  23\n",
    "      Female      22  36\n",
    file = file)
## No test: 
##D file.show(file)
## End(No test)
ft1 <- read.ftable(file)
ft1
unlink(file)

## Agresti (1990), page 297, Table 8.16.
## Almost o.k., but misses the name of the row variable.
file <- tempfile()
cat("                      \"Tonsil Size\"\n",
    "            \"Not Enl.\" \"Enl.\" \"Greatly Enl.\"\n",
    "Noncarriers       497     560           269\n",
    "Carriers           19      29            24\n",
    file = file)
## No test: 
##D file.show(file)
## End(No test)
ft <- read.ftable(file, skip = 2,
                  row.var.names = "Status",
                  col.vars = list("Tonsil Size" =
                      c("Not Enl.", "Enl.", "Greatly Enl.")))
ft
unlink(file)

ft22 <- ftable(Titanic, row.vars = 2:1, col.vars = 4:3)
write.ftable(ft22, quote = FALSE) # is the same as
print(ft22)#method="non.compact" is default
print(ft22, method="row.compact")
print(ft22, method="col.compact")
print(ft22, method="compact")

## using 'justify' and 'quote' :
format(ftable(wool + tension ~ breaks, warpbreaks),
       justify = "none", quote = FALSE)
## Don't show: 
 op <- options(warn = 2) # no warnings allowed
 stopifnot(dim(format(ft)) == 4:5,
	   dim(format(ftable(UCBAdmissions))) == c(6,9))
 meths <- c("non.compact", "row.compact", "col.compact", "compact")
 dimform <-
    function(ft) sapply(meths, function(M) dim(format(ft, method = M)))
 m.eq    <- function(M,m) all.equal(unname(M), m, tolerance = 0)
 ## All format(..) w/o warnings:
 stopifnot(m.eq(print(dimform(ft22)),
		rbind(11:10, rep(7:6, each = 2))),
	   m.eq(print(dimform(ftable(Titanic, row.vars = integer()))),
		rbind(rep(6:5,2), 33)))
 options(op)
## End(Don't show)



