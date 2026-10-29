# GNU R's src/main: the graphics engine and what it needs

GNU R 4.5.0's graphics engine, compiled into robigo's libR by `build.rs`
(GPL-2 | GPL-3, like `rlib/`), so that grDevices, graphics and grid build
from GNU R's sources (`scripts/build-natives.sh`) and run on it. The files
are copied from `src/main` unmodified:

- the engine: `engine.c` (includes `xspline.c`), `devices.c`,
  `plotmath.c`, `patterns.c`, `clippath.c`, `mask.c`, `colors.c`,
  `gevents.c`, `plot3d.c` (with `contour-common.h`), `graphics.c`,
  `plot.c`, and the Hershey fonts (`g_alab_her.c`, `g_cntrlify.c`,
  `g_fontdb.c`, `g_her_glyph.c`, `g_*.h`);
- what they and the graphics packages call: `format.c` (`formatReal`,
  ...), `rlocale.c` (`Ri18n_wcwidth`; `rlocale_*.h`), `localecharset.c`
  (`locale2charset`), `gzio.h` (`R_gzopen`, the reader of grDevices' font
  metric files).

`include/` holds GNU R's internal headers from `src/include`
(`Defn.h`, `Graphics.h`, `GraphicsBase.h`, `Internal.h`, `Print.h`,
`Rgraphics.h`, `rlocale.h`, ...), unmodified, and the `config.h` that
`configure` writes for x86_64 Linux. Everything compiles with
`-DTESTING_WRITE_BARRIER`, so object fields go through the API functions
robigo exports.

The `robigo_*.c` files are ports of GNU R C code that lives in files too
large to vendor whole (each function marked with its origin):

- `robigo_glue.c`: util.c's UTF-8 and Adobe Symbol helpers, and the
  display-list recording of `.External.graphics`/`.Call.graphics`
  (dotcode.c `do_Externalgr`/`do_dotcallgr`) around robigo's foreign
  calls; the devices' GC roots (memory.c).
- `robigo_print.c`: print.c's `R_print`/`PrintDefaults`, printutils.c's
  `Encode*`, options.c's `OutDec` and `max_contour_segments` (options()
  sets them through `option_set`).
- `robigo_sys.c`: sysutils.c's `R_fopen`, `R_popen`, `R_system`,
  `R_tmpnam*`; unix/X11.c without `HAVE_X11` (robigo has no X11 module).

The Rust side (`src/graphics.rs`): `R_FunTab`/`PRIMOFFSET` (display-list
replay calls primitives through the function table), `.Devices` kept by
C, `InitGraphics`, `recordGraphics`.

The utils support also compiles GNU R's unmodified `src/main/dounzip.c`
and `src/modules/internet/{Rsock.c,sock.c,sock.h}`, with `unzip.h` and
`RBufferUtils.h`. The additional ports are:

- `robigo_hashtab.c`: unique.c's hash tables and csduplicated.
- `robigo_util.c`: string buffers, numeric parsing, sorting and wide strings.
- `robigo_internet.c`: internet.c's socket entry points and file downloads.
- `robigo_sys.c`: console history and input handlers.

The natives build uses accessor functions for utils' size.c, with GNU R's
layout sizes from `scripts/natives-layout.h` for object.size reporting.
