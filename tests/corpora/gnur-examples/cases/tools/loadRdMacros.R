library(tools)
### Name: loadRdMacros
### Title: Load User-defined Rd Help System Macros
### Aliases: loadRdMacros loadPkgRdMacros
### Keywords: utilities documentation

### ** Examples

f <- tempfile()
writeLines(r"(
\newcommand{\Rlogo}{
  \if{html}{\figure{Rlogo.svg}{options: width=100 alt="R logo"}}
  \if{latex}{\figure{Rlogo.pdf}{options: width=0.5in}}
}
)", f)
m <- loadRdMacros(f)
ls(m)
## No test: 
##D ls(parent.env(m))
## End(No test)
ls(parent.env(parent.env(m)))
parse_Rd(textConnection(r"(\Rlogo)"), fragment = TRUE, macros = m)



