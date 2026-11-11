# A default is reduced before CheckFormalArgs runs, so the real
# parse lexes the string in it too: two warnings.
function(x, x = "\uD800") 1
