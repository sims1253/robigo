RNGkind("default", "default", "default"); set.seed(1); options(warn = 1)
### Name: col2rgb
### Title: Color to RGB Conversion
### Aliases: col2rgb
### Keywords: color dplot

### ** Examples

col2rgb("peachpuff")
col2rgb(c(blu = "royalblue", reddish = "tomato"))  # note: colnames

col2rgb(1:8)  # the ones from the palette() (if the default)

col2rgb(paste0("gold", 1:4))

col2rgb("#08a0ff")
## all three kinds of color specifications:
col2rgb(c(red = "red", hex = "#abcdef"))
col2rgb(c(palette = 1:3))

# long and short form of hexadecimal notation
col2rgb(c(long = "#559955", short = "#595"))
# with alpha
col2rgb(c(long = "#559955BB", short = "#595B"), alpha = TRUE)

##-- NON-INTRODUCTORY examples --

grC <- col2rgb(paste0("gray", 0:100))
table(print(diff(grC["red",])))  # '2' or '3': almost equidistant
## The 'named' grays are in between {"slate gray" is not gray, strictly}
col2rgb(c(g66 = "gray66", darkg =  "dark gray", g67 = "gray67",
          g74 = "gray74", gray  =       "gray", g75 = "gray75",
          g82 = "gray82", light = "light gray", g83 = "gray83"))

crgb <- col2rgb(cc <- colors())
colnames(crgb) <- cc
t(crgb)  # The whole table

## How many names are 'aliases' of each other?
ccodes <- c(256^(2:0) %*% crgb)
cl <- split(cc, ccodes)
length(cl) # 502 distinct colors
table(tcc <- lengths(cl))
## All the multiply named colors:
clmult <- cl[tcc >= 2]
names(clmult) <- sapply(clmult, function(x) paste(crgb[,x[1]], collapse = ","))
utils::str(clmult)

## Look at the color cube:
tc <- t(crgb[, !duplicated(ccodes)])
cNms <- rownames(tc)
if(requireNamespace("lattice", quietly = TRUE))
    lattice::cloud(blue ~ red + green, data = as.data.frame(tc), col = cNms)
## The 8 corners of the color cube:
isC <- rowSums(tc == 0 | tc == 255) == 3
cNms[isC] # "white" "black" "blue" "cyan" "green" "magenta" "red" "yellow"
## Don't show: 
stopifnot(setequal(cNms[isC],
     c("white","black","blue","cyan","green","magenta","red","yellow")))
## End(Don't show)
table(is.gray <- tc[,1] == tc[,2] & tc[,2] == tc[,3])  # (397, 105)

## Not run: 
##D ## Look at the color cube dynamically:##D 
##D  if(require("rgl")) {
##D    open3d(windowRect = c(50,50, 950, 950)) # large, so we see details
##D    plot3d (tc, col = cNms, size = 11) # --> rotate w/ mouse; enlarged corners:
##D    points3d(tc[isC,], col = cNms[isC], size=22)
##D    bg3d("darkgray") # (to "see more"); rotate around gray-axis:
##D    play3d(spin3d(axis = c(1, 1, 1), rpm = 2), duration = 30)
##D    if(FALSE) # add all names {zoom in with 2nd mouse button!}
##D      text3d(tc[!is.gray,], texts = cNms[!is.gray],
##D                              col = cNms[!is.gray], adj=-1/4, cex = 1/2)
##D    if(FALSE) { ## next version of {rgl}
##D      hover3d(tc, labels = cNms)
##D      message("Move mouse over plot to identify points.")
##D    } else { ##  click on blob to see colors()' name:
##D      identify3d(tc, labels=cNms)
##D    }
##D  }
## End(Not run)



