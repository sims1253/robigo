# The options an Rscript starts with: InitOptions' list, then the system
# profile's (Common.R, then Rprofile.unix over Renviron's variables).
o <- options()
print(names(o))
# the values that do not depend on the installation or the environment
print(o[c("useFancyQuotes", "papersize", "dvipscmd", "defaultPackages", "timeout", "encoding")])
# the installation's commands and paths are strings
print(vapply(o[c("browser", "pager", "pdfviewer", "printcmd", "texi2dvi")], is.character, NA))
print(getOption("useFancyQuotes"))
