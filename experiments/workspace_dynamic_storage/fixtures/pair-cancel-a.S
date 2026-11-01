// Submit one file write, then exit without polling the completion.
.text
.globl _pair_cancel_a
_pair_cancel_a:
    mov x20, x0
    mov x21, x1
    mov x2, #0xa1a1
    cmp x21, x2
    b.ne failure
    str x21, [x20]
barrier:
    svc #0x150
    cmn x0, #11
    b.eq barrier
    cbnz x0, failure
    add x0, x20, #0x200
    add x2, x20, #0x240
    mov w4, #0x2d61
    movk w4, #0x6f64, lsl #16
    str w4, [x0]
    mov w4, #0x63
    strb w4, [x0, #4]
    mov w4, #0x4343
    movk w4, #0x4343, lsl #16
    str w4, [x2]
    mov x1, #5
    mov x3, #4
    svc #0x141
    cmn x0, #11
    b.ne failure
    mov x0, #0
    svc #0x123
failure:
    mov x0, #42
    svc #0x123
