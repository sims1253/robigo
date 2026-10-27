### Name: citation
### Title: Citing R and R Packages in Publications
### Aliases: CITATION citation readCitationFile citHeader citFooter
### Keywords: misc

### ** Examples

## the basic R reference
citation()

## extract the BibTeX entry from the return value
x <- citation()
toBibtex(x)
## No test: ##D 
##D ## references for a package
##D citation("lattice")
##D citation("lattice", auto = TRUE)  # request the Manual-type reference
##D citation("foreign")
##D 
##D ## a CITATION file with more than one bibentry:
##D file.show(system.file("CITATION", package="mgcv"))
##D cm <- citation("mgcv")
##D cm # header, text references, plus "reminder" about getting BibTeX
##D print(cm, bibtex = TRUE) # each showing its bibtex code
##D 
##D ## a CITATION file including citation(auto = meta)
##D file.show(system.file("CITATION", package="nlme"))
##D citation("nlme")
## End(No test)



