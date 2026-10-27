library(methods)
### Name: slot
### Title: The Slots in an Object from a Formal Class
### Aliases: slot .hasSlot slot<- slotNames .slotNames getSlots
### Keywords: programming classes

### ** Examples

## Don't show: 
if(isClass("track")) removeClass("track")
## End(Don't show)

setClass("track", slots = c(x="numeric", y="numeric"))
myTrack <- new("track", x = -4:4, y = exp(-4:4))
slot(myTrack, "x")
slot(myTrack, "y") <- log(slot(myTrack, "y"))
utils::str(myTrack)

getSlots("track") # or
getSlots(getClass("track"))
slotNames(class(myTrack)) # is the same as
slotNames(myTrack)

## Transform such an S4 object to a list, e.g. to "export" it:
S4toList <- function(obj) {
   sn <- slotNames(obj)
   structure(lapply(sn, slot, object = obj), names = sn)
}
S4toList(myTrack)

## Don't show: 
removeClass("track")##  should not be needed... see ./setClass.Rd
## End(Don't show)



