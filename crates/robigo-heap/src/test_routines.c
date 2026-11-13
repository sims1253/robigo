/* Package-routine stand-ins for robigo-heap's own tests (tests/heap.rs).
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
uintptr_t robigo_heap_test_hidden[2];

void robigo_heap_test_hooks(SX (*alloc)(void), void (*collect)(void))
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
 * 7th and 65th parameters across a collection: C locals the collection
 * must see (docs/core.md §3). x86-64 passes the 7th on the stack; AArch64
 * passes it in x6, where the volatile qualifier is what forces it into a
 * stack slot of this frame. */
SX robigo_heap_test_stack_params(SX a0, SX a1, SX a2, SX a3, SX a4, SX a5, SX volatile a6, SX a7, SX a8, SX a9, SX a10, SX a11, SX a12, SX a13, SX a14, SX a15, SX a16, SX a17, SX a18, SX a19, SX a20, SX a21, SX a22, SX a23, SX a24, SX a25, SX a26, SX a27, SX a28, SX a29, SX a30, SX a31, SX a32, SX a33, SX a34, SX a35, SX a36, SX a37, SX a38, SX a39, SX a40, SX a41, SX a42, SX a43, SX a44, SX a45, SX a46, SX a47, SX a48, SX a49, SX a50, SX a51, SX a52, SX a53, SX a54, SX a55, SX a56, SX a57, SX a58, SX a59, SX a60, SX a61, SX a62, SX a63, SX volatile a64)
{
    a6 = t_alloc();
    a64 = t_alloc();
    robigo_heap_test_hidden[0] = ~(uintptr_t) a6;
    robigo_heap_test_hidden[1] = ~(uintptr_t) a64;
    clobber();
    t_collect();
    return a6 == a64 ? a0 : a1;
}

/* A .Call routine of 65 arguments that collects (through the test's
 * export stand-in, which closes the C segment) while its stack arguments
 * hold whatever words the caller passed: the conservative scan's only view
 * of them. a64 is on the stack on x86-64 and AArch64 alike. */
SX robigo_heap_test_collect65(SX a0, SX a1, SX a2, SX a3, SX a4, SX a5, SX a6, SX a7, SX a8, SX a9, SX a10, SX a11, SX a12, SX a13, SX a14, SX a15, SX a16, SX a17, SX a18, SX a19, SX a20, SX a21, SX a22, SX a23, SX a24, SX a25, SX a26, SX a27, SX a28, SX a29, SX a30, SX a31, SX a32, SX a33, SX a34, SX a35, SX a36, SX a37, SX a38, SX a39, SX a40, SX a41, SX a42, SX a43, SX a44, SX a45, SX a46, SX a47, SX a48, SX a49, SX a50, SX a51, SX a52, SX a53, SX a54, SX a55, SX a56, SX a57, SX a58, SX a59, SX a60, SX a61, SX a62, SX a63, SX a64)
{
    t_collect();
    return a0;
}

/* A six-argument native entry (an ALTREP method, a cend) that keeps a fresh
 * object only in a local of its frame across a collection: a C local the
 * collection must see. The volatile qualifier forces it into a stack slot. */
SX robigo_heap_test_local6(SX a0)
{
    SX volatile x = t_alloc();
    robigo_heap_test_hidden[0] = ~(uintptr_t) x;
    x = x;
    clobber();
    t_collect();
    return x == a0 ? a0 : 0;
}

#if defined(__x86_64__)
/* robigo_heap_try_call65(jb, f, args, out, scan_top, 0) entered with rbx and
 * r12..r15 holding v, as a Rust caller's callee-saved registers may (the
 * caller's values are restored after). Arguments: rdi = v, rsi = jb,
 * rdx = f, rcx = args, r8 = out, r9 = scan_top. */
int robigo_heap_try_call65(void *, void *, SX *, SX *, uintptr_t *, int);
__attribute__((naked)) int robigo_heap_test_call65_with_regs(__attribute__((unused)) uintptr_t v, __attribute__((unused)) void *jb,
                                                             __attribute__((unused)) void *f, __attribute__((unused)) SX *args,
                                                             __attribute__((unused)) SX *out, __attribute__((unused)) uintptr_t *scan_top)
{
    __asm__ volatile(
        "push %rbp\n\t"
        "mov %rsp, %rbp\n\t"
        "push %rbx\n\t"
        "push %r12\n\t"
        "push %r13\n\t"
        "push %r14\n\t"
        "push %r15\n\t"
        "sub $8, %rsp\n\t"
        "mov %rdi, %rbx\n\t"
        "mov %rdi, %r12\n\t"
        "mov %rdi, %r13\n\t"
        "mov %rdi, %r14\n\t"
        "mov %rdi, %r15\n\t"
        "mov %rsi, %rdi\n\t"
        "mov %rdx, %rsi\n\t"
        "mov %rcx, %rdx\n\t"
        "mov %r8, %rcx\n\t"
        "mov %r9, %r8\n\t"
        "xor %r9d, %r9d\n\t"
        "call robigo_heap_try_call65\n\t"
        "lea -40(%rbp), %rsp\n\t"
        "pop %r15\n\t"
        "pop %r14\n\t"
        "pop %r13\n\t"
        "pop %r12\n\t"
        "pop %rbx\n\t"
        "pop %rbp\n\t"
        "ret\n\t");
}
#endif
