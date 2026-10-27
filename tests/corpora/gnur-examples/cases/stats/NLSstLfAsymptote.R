RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: NLSstLfAsymptote
### Title: Horizontal Asymptote on the Left Side
### Aliases: NLSstLfAsymptote NLSstLfAsymptote.sortedXyData
### Keywords: manip

### ** Examples

DNase.2 <- DNase[ DNase$Run == "2", ]
DN.srt <- sortedXyData( expression(log(conc)), expression(density), DNase.2 )
NLSstLfAsymptote( DN.srt )



