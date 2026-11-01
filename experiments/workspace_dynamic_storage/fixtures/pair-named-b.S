// ELF loaded through an explicit VFS name; marker differs from the default pair.
.text
.globl _named_b
_named_b:
    mov x20, x0
    mov x21, #0xd2d2
    str x21, [x20]
barrier:
    svc #0x150
    cmn x0, #11
    b.eq barrier
    cbnz x0, failure
    ldr x2, [x20]
    cmp x2, x21
    b.ne failure
    mov x0, #0
    svc #0x123
failure:
    mov x0, #42
    svc #0x123
