x |> f()
x |> f(y)
x |> f(y = _)
x |> f(1, y = _)
x |> g() |> h(z)
x |> (\(y) y + 1)()
x |> _$a
x |> _$a[[1]]
mtcars |> subset(cyl == 4) |> nrow()
