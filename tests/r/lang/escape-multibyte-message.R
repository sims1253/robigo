# GNU R's message carries the lone lead byte of the escaped character.
parse(text = "\"\\é\"")
