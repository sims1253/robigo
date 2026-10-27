library(methods)
### Name: RMethodUtils
### Title: Method Utilities
### Aliases: substituteFunctionArgs makeGeneric makeStandardGeneric
###   generic.skeleton defaultDumpName doPrimitiveMethod conformMethod
###   getGeneric getGroup getGroupMembers getMethodsMetaData
###   assignMethodsMetaData matchSignature findUnique MethodAddCoerce
###   .saveImage cacheMetaData cacheGenericsMetaData setPrimitiveMethods
###   missingArg balanceMethodsList sigToEnv rematchDefinition isRematched
###   unRematchDefinition asMethodDefinition
###   addNextMethod,MethodDefinition-method
###   addNextMethod,MethodWithNext-method addNextMethod .valueClassTest
###   insertClassMethods .ShortPrimitiveSkeletons .EmptyPrimitiveSkeletons
### Keywords: internal

### ** Examples

getGroup("exp")
getGroup("==", recursive = TRUE)

getGroupMembers("Arith")
getGroupMembers("Math")
getGroupMembers("Ops") # -> its sub groups



