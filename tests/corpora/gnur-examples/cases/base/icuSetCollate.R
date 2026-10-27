RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: icuSetCollate
### Title: Setup Collation by ICU
### Aliases: icuSetCollate icuGetCollate R_ICU_LOCALE
### Keywords: utilities

### ** Examples
## No test: 
##D ## These examples depend on having ICU available, and on the locale.
##D ## As we don't know the current settings, we can only reset to the default.
##D if(capabilities("ICU")) withAutoprint({
##D     icuGetCollate()
##D     icuGetCollate("valid")
##D     x <- c("Aarhus", "aarhus", "safe", "test", "Zoo")
##D     sort(x)
##D     icuSetCollate(case_first = "upper"); sort(x)
##D     icuSetCollate(case_first = "lower"); sort(x)
##D 
##D     ## Danish collates upper-case-first and with 'aa' as a single letter
##D     icuSetCollate(locale = "da_DK", case_first = "default"); sort(x) 
##D     ## Estonian collates Z between S and T
##D     icuSetCollate(locale = "et_EE"); sort(x)
##D     icuSetCollate(locale = "default"); icuGetCollate("valid")
##D })
## End(No test)


