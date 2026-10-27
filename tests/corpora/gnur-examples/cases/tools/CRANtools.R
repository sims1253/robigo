library(tools)
### Name: CRANtools
### Title: CRAN Package Repository Tools
### Aliases: CRAN_package_db CRAN_check_results CRAN_check_details
###   CRAN_check_issues summarize_CRAN_check_status R_CRAN_WEB R_CRAN_SRC
###   CRAN_current_db CRAN_aliases_db CRAN_rdxrefs_db CRAN_archive_db
###   CRAN_authors_db

### ** Examples
## No test: 
##D ## This can be rather slow  with a non-local CRAN mirror
##D ## and might fail (slowly) without Internet access in that case.
##D 
##D set.seed(11)  # but the packages chosen will change as soon as CRAN does.
##D pdb <- CRAN_package_db()
##D dim(pdb)
##D ## DESCRIPTION fields included:
##D colnames(pdb)
##D ## Summarize publication dates:
##D summary(as.Date(pdb$Published))
##D ## Summarize numbers of packages according to maintainer:
##D summary(lengths(split(pdb$Package, pdb$Maintainer)))
##D ## Packages with 'LASSO' in their Description:
##D pdb$Package[grepl("LASSO", pdb$Description)]
##D 
##D results <- CRAN_check_results()
##D ## Available variables:
##D names(results)
##D ## Tabulate overall check status according to flavor:
##D with(results, table(Flavor, Status))
##D 
##D details <- CRAN_check_details()
##D ## Available variables:
##D names(details)
##D ## Tabulate checks according to their status:
##D tab <- with(details, table(Check, Status))
##D ## Inspect some installation problems:
##D bad <- subset(details,
##D               ((Check == "whether package can be installed") &
##D                (Status != "OK")))
##D ## Show a random sample of up to 6
##D head(bad[sample(seq_len(NROW(bad)), NROW(bad)), ])
##D 
##D issues <- CRAN_check_issues()
##D head(issues)
##D ## Show counts of issues according to kind:
##D table(issues[, "kind"])
##D 
##D ## Summarize CRAN check status for 10 randomly-selected packages
##D ## (reusing the information already read in):
##D pos <- sample(seq_len(NROW(pdb)), 10L)
##D summarize_CRAN_check_status(pdb[pos, "Package"],
##D                             results, details, issues)
## End(No test)


