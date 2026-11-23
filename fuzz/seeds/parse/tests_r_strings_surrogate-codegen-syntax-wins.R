# A syntax error later in the statement fails the trial parse, so
# the pipe's code-generation error never happens: one warning.
1 |> f("\uD800", _) )
