### Name: cairoSymbolFont
### Title: Specify a Symbol Font
### Aliases: cairoSymbolFont
### Keywords: device

### ** Examples

## Not run: 
##D ## If a font uses PUA, we can just specify the font name ...
##D cairo_pdf(symbolfamily="OpenSymbol")
##D dev.off()
##D ## ... or equivalently ...
##D cairo_pdf(symbolfamily=cairoSymbolFont("OpenSymbol"))
##D dev.off()
##D 
##D ## If a font does not use PUA, we must indicate that ...
##D cairo_pdf(symbolfamily=cairoSymbolFont("Nimbus Sans", usePUA=FALSE))
##D dev.off()
## End(Not run)



