// Executed in EL0 after loading from next.bin. Calls the bounded write ABI.
.text
.globl _generic_write_demo
_generic_write_demo:
    mov x20, #0x50000000
    add x20, x20, #0x280
    add x21, x20, #0xa0
    // "actor.txt" and "world".
    mov w4, #0x6361
    movk w4, #0x6f74, lsl #16
    str w4, [x20]
    mov w4, #0x2e72
    movk w4, #0x7874, lsl #16
    str w4, [x20, #4]
    mov w4, #0x74
    strb w4, [x20, #8]
    mov w4, #0x6f77
    movk w4, #0x6c72, lsl #16
    str w4, [x21]
    mov w4, #0x64
    strb w4, [x21, #4]
    mov x0, x20
    mov x1, #9
    mov x2, x21
    mov x3, #5
    svc #0x141                 // enqueue a private copy of "world"
    cmn x0, #11
    b.ne failure
    mov w4, #0x57              // change source to "World" before polling
    strb w4, [x21]
    mov x0, x20
    bl write_retry
    cmp x0, #5
    b.ne failure

    // Empty files are allowed; the source length is zero.
    mov w4, #0x6d65
    movk w4, #0x7470, lsl #16
    str w4, [x20]
    mov w4, #0x79
    strb w4, [x20, #4]
    mov x0, x20
    mov x1, #5
    mov x2, x21
    mov x3, #0
    bl write_retry
    cbnz x0, failure

    // A valid user range containing an invalid name must be refused.
    mov w4, #0x2e2e
    strh w4, [x20]
    mov x0, x20
    mov x1, #2
    mov x2, x21
    mov x3, #5
    bl write_retry
    cmn x0, #22
    b.ne failure

    // A source crossing the end of the private page is invalid.
    mov w4, #0x6c66
    movk w4, #0x6c75, lsl #16
    str w4, [x20]               // "full"
    mov x0, x20
    mov x1, #4
    mov x2, #0x50000000
    add x2, x2, #0xfff
    mov x3, #2
    bl write_retry
    cmn x0, #14
    b.ne failure

    // The runner fills the catalog; no room for "full".
    mov x0, x20
    mov x1, #4
    mov x2, x21
    mov x3, #5
    bl write_retry
    cmn x0, #28
    b.ne failure
    mov x0, #0
    svc #0x123
failure:
    mov x0, #42
    svc #0x123
write_retry:
    mov x24, x0
    mov x25, #0x100000
retry:
    mov x0, x24
    svc #0x141
    cmn x0, #11
    b.ne done
    subs x25, x25, #1
    b.ne retry
    b failure
done:
    ret
