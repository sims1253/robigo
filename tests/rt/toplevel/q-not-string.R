# q() reports do_quit's errors against its own call too: error(), not
# errorcall() (main.c:1595), the context being the closure q.
q(save = 1)
