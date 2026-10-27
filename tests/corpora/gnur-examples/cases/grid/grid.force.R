library(grid)
### Name: grid.force
### Title: Force a grob into its components
### Aliases: grid.force grid.force.default grid.force.gPath grid.force.grob
###   forceGrob grid.revert grid.revert.gPath grid.revert.grob
### Keywords: dplot

### ** Examples

grid.newpage()
pushViewport(viewport(width=.5, height=.5))
# Draw xaxis
grid.xaxis(name="xax")
grid.ls()
# Force xaxis
grid.force()
grid.ls()
# Revert xaxis
grid.revert()
grid.ls()
# Draw and force yaxis
grid.force(yaxisGrob(), draw=TRUE)
grid.ls()
# Revert yaxis
grid.revert()
grid.ls()
# Force JUST xaxis
grid.force("xax")
grid.ls()
# Force ALL
grid.force()
grid.ls()
# Revert JUST xaxis
grid.revert("xax")
grid.ls()



