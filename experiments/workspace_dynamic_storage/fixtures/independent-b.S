// Independent task b: wait on its VFS gate until the service releases it.
.text
.globl _independent_b
_independent_b:
    mov x20, x0
    mov x21, x1
    mov x2, #0xb2b2
    cmp x21, x2
    b.ne failure
    str x21, [x20]
    add x24, x20, #0x200
    add x25, x20, #0x240
    mov w4, #0x2d62
    movk w4, #0x6167, lsl #16
    str w4, [x24]
    mov w4, #0x6574
    strh w4, [x24, #4]
    mov x1, #6
    mov x2, x25
    mov x3, #1
read_again:
    mov x0, x24
    svc #0x140
    cmn x0, #11
    b.eq read_again
    cmp x0, #1
    b.ne failure
    ldrb w4, [x25]
    cmp w4, #0x30
    b.eq read_again
    cmp w4, #0x31
    b.ne failure
    ldr x4, [x20]
    cmp x4, x21
    b.ne failure
    mov x0, #0
    svc #0x123
failure:
    mov x0, #42
    svc #0x123
