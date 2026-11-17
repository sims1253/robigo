## 2026-11-17: S3x-d, messages are native-encoded bytes

- **The question** (the foreman, from B's math work, #802). rtcore's
  diagnostics were UTF-8 text (`&str`), but GNU R's messages are `char *`
  in the native encoding. Should rtcore carry a byte-string message
  through the condition path, or refuse such messages and document it?
- **What GNU R does.** A message is whatever bytes its `char *` holds.
  `stop(x)` and `warning(x)` pass `translateChar(x)` on (errors.c:do_stop,
  do_warning). For a string with invalid UTF-8 bytes in a UTF-8 locale, a
  native string such as `"a\xffb"` or a line read from a Latin-1 file,
  that is the raw byte. So:
  - the condition's message is `identical` to `x`;
  - the report writes the byte as it is.

  Text GNU R passes through `EncodeChar` or `EncodeString` (a symbol in
  "object '%s' not found", a deparsed call) is escaped, so it is valid
  UTF-8 whatever the input.
- **What robigo-rt did.** `do_stop` and `do_warning` turned the message
  into a Rust string with `String::from_utf8_lossy`. So `stop("a\xffb")`'s
  condition message was U+FFFD where GNU R has the byte, and was not
  `identical` to `x`. The deferred warning printed `a\357\277\275b`, where
  GNU R prints `a\377b`. The conditions code was already bytes inside;
  the losses were at the `&str` boundary.
- **Decision: a byte-string message, not a refusal.** A refusal would
  document a difference R code can see (`identical`, the bytes of a
  report), and robigo-rt's goal is GNU R's output exactly.
  - rtcore's error and warning hooks (`ErrorFn`, `WarningFn`),
    `ErrorCond::msg`, `record_error_call` and `collect_warning` take the
    message as native bytes (`&[u8]`).
  - `Cx::error_bytes`, `error_call_bytes` and `warning_call_bytes` take
    them from callers. `Cx::error`, `error_call` and `warning_call` stay
    the `&str` conveniences over them, for the messages Rust builds from
    literals and escaped text, so no call site changes.
  - A site that passes GNU R's `translateChar` text into a message uses
    the byte variants. `do_stop` and `do_warning` do now.
  - Any message that is not valid UTF-8 is stored as a native string
    (the error record's value, the deferred warning). A UTF-8 one is
    stored as before.
- **What stays to do.** Rust-built messages that embed an R string
  through `String::from_utf8_lossy` (about 90 sites across rt and
  rtbuiltins) are now the only lossy ones. Each is right only where GNU R
  escapes the text there (`EncodeChar`, `EncodeString`). Where GNU R uses
  `translateChar`, the site's owner switches it to the byte variants when
  a case shows it, the way S3x-b's classed errors went site by site. No
  case shows one today.
- Case: `tests/rt/conditions/native-bytes`, recorded on 4.6.2 and
  ratcheted. It checks:
  - `stop`'s and `warning`'s condition messages, by `identical`, through
    exiting and calling handlers;
  - the deferred warning with and without a call;
  - the uncaught error.

  It passes byte for byte, and failed before the change.
