### Name: xtabs
### Title: Cross Tabulation
### Aliases: xtabs print.xtabs
### Keywords: category

### ** Examples

## 'esoph' has the frequencies of cases and controls for all levels of
## the variables 'agegp', 'alcgp', and 'tobgp'.
xtabs(cbind(ncases, ncontrols) ~ ., data = esoph)
## Output is not really helpful ... flat tables are better:
ftable(xtabs(cbind(ncases, ncontrols) ~ ., data = esoph))
## In particular if we have fewer factors ...
ftable(xtabs(cbind(ncases, ncontrols) ~ agegp, data = esoph))

## This is already a contingency table in array form.
DF <- as.data.frame(UCBAdmissions)
## Now 'DF' is a data frame with a grid of the factors and the counts
## in variable 'Freq'.
DF
## Nice for taking margins ...
xtabs(Freq ~ Gender + Admit, DF)
## And for testing independence ...
summary(xtabs(Freq ~ ., DF))

## with NA's
DN <- DF; DN[cbind(6:9, c(1:2,4,1))] <- NA
DN # 'Freq' is missing only for (Rejected, Female, B)
(xtNA <- xtabs(Freq ~ Gender + Admit, DN))     # NA prints 'invisibly'
print(xtNA, na.print = "NA")                   # show NA's better
xtabs(Freq ~ Gender + Admit, DN, na.rm = TRUE) # ignore missing Freq
## Use addNA = TRUE to tabulate missing factor levels:
xtabs(Freq ~ Gender + Admit, DN, addNA = TRUE)
xtabs(Freq ~ Gender + Admit, DN, addNA = TRUE, na.rm = TRUE)
## na.action = na.omit removes all rows with NAs right from the start:
xtabs(Freq ~ Gender + Admit, DN, na.action = na.omit)

## Create a nice display for the warp break data.
warpbreaks$replicate <- rep_len(1:9, 54)
ftable(xtabs(breaks ~ wool + tension + replicate, data = warpbreaks))

### ---- Sparse Examples ----

## No test: 
##D if(require("Matrix")) withAutoprint({
##D  ## similar to "nlme"s  'ergoStool' :
##D  d.ergo <- data.frame(Type = paste0("T", rep(1:4, 9*4)),
##D                       Subj = gl(9, 4, 36*4))
##D  xtabs(~ Type + Subj, data = d.ergo) # 4 replicates each
##D  set.seed(15) # a subset of cases:
##D  xtabs(~ Type + Subj, data = d.ergo[sample(36, 10), ], sparse = TRUE)
##D 
##D  ## Hypothetical two-level setup:
##D  inner <- factor(sample(letters[1:25], 100, replace = TRUE))
##D  inout <- factor(sample(LETTERS[1:5], 25, replace = TRUE))
##D  fr <- data.frame(inner = inner, outer = inout[as.integer(inner)])
##D  xtabs(~ inner + outer, fr, sparse = TRUE)
##D })
## End(No test)



