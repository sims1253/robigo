### Name: getInitial
### Title: Get Initial Parameter Estimates
### Aliases: getInitial getInitial.default getInitial.formula
###   getInitial.selfStart
### Keywords: models nonlinear manip

### ** Examples

PurTrt <- Puromycin[ Puromycin$state == "treated", ]
print(getInitial( rate ~ SSmicmen( conc, Vm, K ), PurTrt ), digits = 3)



