RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: NLSstRtAsymptote
### Title: Horizontal Asymptote on the Right Side
### Aliases: NLSstRtAsymptote NLSstRtAsymptote.sortedXyData
### Keywords: manip

### ** Examples

DNase.2 <- DNase[ DNase$Run == "2", ]
DN.srt <- sortedXyData( expression(log(conc)), expression(density), DNase.2 )
NLSstRtAsymptote( DN.srt )



