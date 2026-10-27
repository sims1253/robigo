library(methods)
### Name: S3Part
### Title: S4 Classes that Contain S3 Classes
### Aliases: S3Part S3Part<- S3Class S3Class<- isXS3Class slotsFromS3 S4 S3
###   coerce,ANY,S3-method coerce,oldClass,S3-method coerce,ANY,S4-method
###   S3-class
### Keywords: programming classes

### ** Examples


## an "mlm" object, regressing two variables on two others

sepal <- as.matrix(datasets::iris[,c("Sepal.Width", "Sepal.Length")])
fit <- lm(sepal ~ Petal.Length + Petal.Width + Species, data = datasets::iris)
class(fit) # S3 class: "mlm", "lm"

## a class that contains "mlm"
myReg <- setClass("myReg", slots = c(title = "character"), contains = "mlm")

fit2 <- myReg(fit, title = "Sepal Regression for iris data")

fit2 # shows the inherited "mlm" object and the title

identical(S3Part(fit2), as(fit2, "mlm"))

class(as(fit2, "mlm")) # the S4 class, "mlm"

class(as(fit2, "S3")) # the S3 class, c("mlm", "lm")

## An object may contain an S3 class from a subclass of that declared:
xlm <- setClass("xlm", slots = c(eps = "numeric"), contains = "lm")

xfit <- xlm(fit, eps = .Machine$double.eps)

xfit@.S3Class # c("mlm", lm")

## Don't show: 
    removeClass("myReg")
## End(Don't show)



