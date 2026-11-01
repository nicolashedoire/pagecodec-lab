// EL0 file-read sample: all addresses refer to the task's private 4 KiB page.
// SVC #0x140: x0/name x1/name_len x2/destination x3/capacity; x0=result.
.text
.globl _generic_read_demo
_generic_read_demo:
    mov x20, #0x50000000
    add x20, x20, #0x280       // arbitrary in-page name buffer
    add x21, x20, #0xa0        // arbitrary in-page destination
    mov w4, #0x6f6e            // "note" little-endian
    movk w4, #0x6574, lsl #16
    str w4, [x20]
    mov w4, #0xa5
    sturb w4, [x21, #-1]       // outside destination: must not change
    strb w4, [x21, #5]         // outside destination: must not change

    mov x0, x20
    mov x1, #4
    mov x2, x21
    mov x3, #5
    bl read_retry
    cmp x0, #5
    b.ne failure
    ldr w4, [x21]
    mov w5, #0x6568            // "hell" little-endian
    movk w5, #0x6c6c, lsl #16
    cmp w4, w5
    b.ne failure
    ldrb w4, [x21, #4]
    cmp w4, #0x6f               // "o"
    b.ne failure
    ldurb w4, [x21, #-1]
    cmp w4, #0xa5
    b.ne failure
    ldrb w4, [x21, #5]
    cmp w4, #0xa5
    b.ne failure

    mov w4, #0x6f6e            // "none" (missing file)
    movk w4, #0x656e, lsl #16
    str w4, [x20]
    mov x0, x20
    mov x1, #4
    mov x2, x21
    mov x3, #5
    bl read_retry
    cmn x0, #2                 // -ENOENT
    b.ne failure

    mov w4, #0x6f6e            // "note" again
    movk w4, #0x6574, lsl #16
    str w4, [x20]
    mov w4, #0x5a5a
    movk w4, #0x5a5a, lsl #16
    str w4, [x21]              // target remains unchanged on too-small read
    strb w4, [x21, #4]
    mov x0, x20
    mov x1, #4
    mov x2, x21
    mov x3, #4                 // "hello" needs 5 bytes
    bl read_retry
    cmn x0, #34                // -ERANGE, no partial write
    b.ne failure
    ldr w4, [x21]
    mov w5, #0x5a5a
    movk w5, #0x5a5a, lsl #16
    cmp w4, w5
    b.ne failure
    ldrb w4, [x21, #4]
    cmp w4, #0x5a
    b.ne failure
    ldurb w4, [x21, #-1]
    cmp w4, #0xa5
    b.ne failure
    ldrb w4, [x21, #5]
    cmp w4, #0xa5
    b.ne failure

    mov x22, #0x50000000
    add x22, x22, #0xfff       // last byte of the private page
    mov w4, #0xa5
    strb w4, [x22]
    mov x0, x20
    mov x1, #4
    mov x2, x22
    mov x3, #2                 // crosses the page boundary
    bl read_retry
    cmn x0, #14                // -EFAULT
    b.ne failure
    ldrb w4, [x22]
    cmp w4, #0xa5
    b.ne failure
    mov x0, #0
    svc #0x123
failure:
    mov x0, #42
    svc #0x123

// File service runs between slices; retry EAGAIN with the original name.
// The explicit counter and kernel interrupt budget bound the waiting path.
read_retry:
    mov x24, x0
    mov x25, #0x100000
retry:
    mov x0, x24
    svc #0x140
    cmn x0, #11                // -EAGAIN
    b.ne read_done
    subs x25, x25, #1
    b.ne retry
    b failure
read_done:
    ret
