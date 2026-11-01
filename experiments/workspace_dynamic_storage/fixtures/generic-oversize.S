// EL0 file-read sample: a 33-byte file cannot fit in a 32-byte buffer.
.text
.globl _generic_oversize_demo
_generic_oversize_demo:
    mov x20, #0x50000000
    add x20, x20, #0x280
    add x21, x20, #0xa0
    mov w4, #0x6962             // "big" little-endian
    movk w4, #0x0067, lsl #16
    str w4, [x20]
    mov x4, #0x5a5a
    movk x4, #0x5a5a, lsl #16
    movk x4, #0x5a5a, lsl #32
    movk x4, #0x5a5a, lsl #48
    stp x4, x4, [x21]
    stp x4, x4, [x21, #16]
    mov w5, #0xa5
    sturb w5, [x21, #-1]
    strb w5, [x21, #32]
    mov x0, x20
    mov x1, #3
    mov x2, x21
    mov x3, #32
    bl read_retry
    cmn x0, #34                 // -ERANGE, file exists but is too large
    b.ne failure
    mov w5, #0x2e2e             // ".." is an invalid file name
    strh w5, [x20]
    mov x0, x20
    mov x1, #2
    mov x2, x21
    mov x3, #32
    bl read_retry
    cmn x0, #22                 // -EINVAL, not "file missing"
    b.ne failure
    ldp x5, x6, [x21]
    cmp x5, x4
    b.ne failure
    cmp x6, x4
    b.ne failure
    ldp x5, x6, [x21, #16]
    cmp x5, x4
    b.ne failure
    cmp x6, x4
    b.ne failure
    ldurb w5, [x21, #-1]
    cmp w5, #0xa5
    b.ne failure
    ldrb w5, [x21, #32]
    cmp w5, #0xa5
    b.ne failure
    mov x0, #0
    svc #0x123
failure:
    mov x0, #42
    svc #0x123
read_retry:
    mov x24, x0
    mov x25, #0x100000
retry:
    mov x0, x24
    svc #0x140
    cmn x0, #11
    b.ne read_done
    subs x25, x25, #1
    b.ne retry
    b failure
read_done:
    ret
