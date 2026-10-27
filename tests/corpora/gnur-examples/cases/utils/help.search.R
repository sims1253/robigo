RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: help.search
### Title: Search the Help System
### Aliases: help.search ?? print.hsearch
### Keywords: documentation

### ** Examples

## Don't show: 
help.search("linear models", lib.loc = .Library) # (limit to "base + Rec." packages)
## End(Don't show)
## Not run: 
##D  
##D help.search("linear models")  # In case you forgot how to fit linear models
## End(Not run)
help.search("non-existent topic")

??utils::help  # All the topics matching "help" in the utils package

## No test: 
##D ## Documentation with topic/concept/title matching 'print'
##D ## (disabling fuzzy matching to not also match 'point')
##D help.search("print", agrep = FALSE)
##D help.search(apropos = "print", agrep = FALSE)  # ignores concepts
##D 
##D ## Help pages with documented topics starting with 'try':
##D help.search("^try", fields = "alias")
##D alias??"^try"  # the same
##D 
##D ## Help pages documenting high-level plots:
##D help.search(keyword = "hplot")
## End(No test)
RShowDoc("KEYWORDS")  # show all keywords



