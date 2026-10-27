### Name: RClassUtils
### Title: Utilities for Managing Class Definitions
### Aliases: completeSubclasses newClassRepresentation
###   print.classRepresentation setExtendsMetaData setSubclassMetaData
###   subclassesMetaName extendsMetaName classPrototypeDef-class .classEnv
###   classLabel testVirtual makePrototypeFromClassDef newEmptyObject
###   completeClassDefinition getAllSuperClasses superClassDepth
###   isVirtualClass assignClassDef newBasic makeExtends
###   reconcilePropertiesAndPrototype tryNew empty.dump showClass
###   showExtends possibleExtends completeExtends classMetaName
###   methodsPackageMetaName metaNameUndo requireMethods checkAtAssignment
###   checkSlotAssignment defaultPrototype isClassDef validSlotNames
###   getDataPart setDataPart .BasicClasses .BasicVectorClasses
###   .InitBasicClasses .InitMethodsListClass .setCoerceGeneric
###   conditionalExtension-class
### Keywords: internal

### ** Examples

typeof(defaultPrototype()) #-> "S4"

## .classEnv()
meth.ns <- asNamespace("methods")
if(get4 <- !any("package:stats4" == search()))
   require("stats4")
stopifnot(TRUE
 , identical(.classEnv("data.frame"), meth.ns)
 , identical(.classEnv(class(new("data.frame"))), meth.ns)
 , identical(.classEnv(     "mle"       ), meth.ns) # <- *not* 'stats4'
 , identical(.classEnv(class(new("mle"))), asNamespace("stats4"))
 , identical(.classEnv(getClass ("mle") ), asNamespace("stats4"))
 )
if(get4) detach("package:stats4")



