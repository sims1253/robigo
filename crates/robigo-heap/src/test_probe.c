/* A "package" for the pin report's tests (tests/heap.rs): built as a
 * shared library of its own (build.rs, Linux), so that the report names it
 * as dladdr names a package's .so. Nothing else uses it. */
#include <stdint.h>

typedef void *SX;

static void (*probe_collect)(void);
static SX probe_key;

/* The export the routine calls (the test's stand-in, which collects). */
void robigo_heap_probe_hooks(void (*collect)(void))
{
    probe_collect = collect;
}

/* Hand the routine its object through a global, which the conservative
 * scan does not read: afterwards only the routine's own local names it. */
void robigo_heap_probe_give(SX key)
{
    probe_key = key;
}

/* A .Call routine that keeps an object in an unprotected local across a
 * call into an export that collects, and uses it after: a missing
 * PROTECT, which the pin report names (docs/g3.md §3 item 3). */
SX robigo_heap_probe_unprotected(void)
{
    SX volatile k = probe_key;
    probe_key = 0;
    probe_collect();
    return k;
}
