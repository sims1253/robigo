RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: ns-internals
### Title: Namespace Internals
### Aliases: asNamespace getNamespaceInfo .getNamespaceInfo importIntoEnv
###   isBaseNamespace isNamespace namespaceExport namespaceImport
###   namespaceImportClasses namespaceImportFrom namespaceImportMethods
###   packageHasNamespace parseNamespaceFile registerS3method
###   registerS3methods setNamespaceInfo .__S3MethodsTable__.
###   .mergeExportMethods .mergeImportMethods .knownS3Generics
###   loadingNamespaceInfo .getNamespace ..getNamespace .S3_methods_table
### Keywords: internal

### ** Examples

nsName <- "stats"
(ns <- asNamespace(nsName)) # <environment: namespace:stats>

## Inverse function of asNamespace() :
environmentName(asNamespace("stats")) # "stats"
environmentName(asNamespace("base"))  # "base"
getNamespaceInfo(ns, "spec")[["name"]] ## -> "stats"
## Don't show: 
stopifnot(identical(nsName,
                      getNamespaceInfo(ns, "spec")[["name"]]))
## End(Don't show)

## Only for the daring ones, getting into the bowels (non-API!):
.nsInfEnv <- function(ns)   asNamespace(ns, base.OK = FALSE)[[".__NAMESPACE__."]]
lsNSInfo  <- function(ns, ...) ls(..., envir = .nsInfEnv(ns))
allinfoNS <- function(ns)      as.list(.nsInfEnv(ns), sorted=TRUE)

utils::str(allinfoNS("stats"))
utils::str(allinfoNS("stats4"))



