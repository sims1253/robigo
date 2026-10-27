### Name: capture.output
### Title: Send Output to a Character String or File
### Aliases: capture.output
### Keywords: utilities

### ** Examples

require(stats)
glmout <- capture.output(summary(glm(case ~ spontaneous+induced,
                                     data = infert, family = binomial())))
glmout[1:5]
capture.output(1+1, 2+2)
capture.output({1+1; 2+2})

## Not run: 
##D ## on Unix-alike with a2ps available##D 
##D op <- options(useFancyQuotes=FALSE)
##D pdf <- pipe("a2ps -o - | ps2pdf - tempout.pdf", "w")
##D capture.output(example(glm), file = pdf)
##D close(pdf); options(op) ; system("evince tempout.pdf &")
## End(Not run)



