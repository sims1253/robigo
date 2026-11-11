# xxpipe's placeholder checks are GenerateCode-gated: the trial parse
# accepts the statement, the real parse lexes it again and fails at the
# pipe's reduction. Two warnings.
1 |> f("\uD800", _)
