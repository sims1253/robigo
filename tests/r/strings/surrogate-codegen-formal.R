# CheckFormalArgs fails in the real parse when the second x is
# reduced, before the body's string is lexed again: the trial parse's
# warning alone.
function(x, x) "\uD800"
