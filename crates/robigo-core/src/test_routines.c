/* Package-routine stand-ins for robigo-core's own tests (tests/core.rs).
 * Nothing else references this object, so the linker leaves it out of
 * binaries. */
#include <stdint.h>

typedef void *SX;

/* (most of the 65 parameters only take a slot) */
#pragma GCC diagnostic ignored "-Wunused-parameter"

static SX (*t_alloc)(void);
static void (*t_collect)(void);
/* the objects, inverted: no word anywhere points at them but the
 * routine's own parameter slots */
uintptr_t robigo_test_hidden[2];

void robigo_test_hooks(SX (*alloc)(void), void (*collect)(void))
{
    t_alloc = alloc;
    t_collect = collect;
}

__attribute__((noinline)) static void clobber(void)
{
    uintptr_t a[8192] = {0};
    __asm__ volatile("" : : "r"(a) : "memory");
}

/* A .Call routine of 65 arguments that keeps a fresh object only in its
 * 7th and 65th parameters, which x86-64 and AArch64 pass on the stack,
 * across a collection: C locals the collection must see (docs/core.md
 * §3). */
SX robigo_test_stack_params(SX a0, SX a1, SX a2, SX a3, SX a4, SX a5, SX volatile a6, SX a7, SX a8, SX a9, SX a10, SX a11, SX a12, SX a13, SX a14, SX a15, SX a16, SX a17, SX a18, SX a19, SX a20, SX a21, SX a22, SX a23, SX a24, SX a25, SX a26, SX a27, SX a28, SX a29, SX a30, SX a31, SX a32, SX a33, SX a34, SX a35, SX a36, SX a37, SX a38, SX a39, SX a40, SX a41, SX a42, SX a43, SX a44, SX a45, SX a46, SX a47, SX a48, SX a49, SX a50, SX a51, SX a52, SX a53, SX a54, SX a55, SX a56, SX a57, SX a58, SX a59, SX a60, SX a61, SX a62, SX a63, SX volatile a64)
{
    a6 = t_alloc();
    a64 = t_alloc();
    robigo_test_hidden[0] = ~(uintptr_t) a6;
    robigo_test_hidden[1] = ~(uintptr_t) a64;
    clobber();
    t_collect();
    return a6 == a64 ? a0 : a1;
}
