# Long calls break into lines past the cutoff (60 bytes): after an
# argument's ", " (args2buff) and after a binary operator (linebreak),
# indented four spaces.
quote(averyveryverylongfunctionname(argument_number_one = 1, argument_number_two = 2, argument_number_three = 3, four = 4))
quote(aaaaaaaaaaaaaaa + bbbbbbbbbbbbbbbbbbbbb + cccccccccccccccccccc + dddddddddddddddddddd + eeeeeeeeeeeeeeeee + ffff)
quote(f(g(aaaaaaaaaaaaaaaaaaaa, bbbbbbbbbbbbbbbbbbbbbbbbbb), h(cccccccccccccccccccccc, dddddddddddddddddddddddddd, eeeeeeeeeeee)))
quote(function(aaaaaaaaaaaa = 1, bbbbbbbbbbbbbbb = 2, cccccccccccccccccc = 3, dddddddddddddddddddd = 4) NULL)
quote({
    result <- some_function_name(first_argument, second_argument, third_argument, fourth_argument)
    if (condition_one && condition_two && condition_three && condition_four && condition_five) result
})
