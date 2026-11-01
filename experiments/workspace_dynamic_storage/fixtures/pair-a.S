// Pair a: own file operations and refused access to the other namespace.
.text
.globl _pair_a
_pair_a:
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
    ldr x2, [x20]
    cmp x2, x21
    b.ne failure
    add x24, x20, #0x200
    add x25, x20, #0x240
    mov w4, #0x2d61
    movk w4, #0x6f64, lsl #16
    str w4, [x24]
    mov w4, #0x63
    strb w4, [x24, #4]
    mov x1, #5
    mov x2, x25
    mov x3, #4
    mov x27, #0
    bl io
    cmp x0, #4
    b.ne failure
    ldrb w4, [x25]
    cmp w4, #0x61
    b.ne failure
    mov w4, #0x4141
    movk w4, #0x4141, lsl #16
    str w4, [x25]
    mov x27, #1
    bl io
    cmp x0, #4
    b.ne failure
    mov w4, #0x62
    strb w4, [x24]
    mov w4, #0x5a
    strb w4, [x25]
    mov x27, #0
    bl io
    cmn x0, #13
    b.ne failure
    ldrb w4, [x25]
    cmp w4, #0x5a
    b.ne failure
    mov x27, #1
    bl io
    cmn x0, #13
    b.ne failure
    mov x0, #0
    svc #0x123
failure:
    mov x0, #42
    svc #0x123
io:
    mov x0, x24
    cbnz x27, put
    svc #0x140
    b check
put:
    svc #0x141
check:
    cmn x0, #11
    b.eq io
    ret
