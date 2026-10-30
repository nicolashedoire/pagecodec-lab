    .section .text.vectors, "ax"
    .balign 2048
    .global vector_table
vector_table:
    b fatal_svc
    .balign 128
    b fatal_svc
    .balign 128
    b fatal_svc
    .balign 128
    b fatal_svc
    .balign 128
    b fatal_svc
    .balign 128
    b fatal_svc
    .balign 128
    b fatal_svc
    .balign 128
    b fatal_svc
    .balign 128
    b svc_handler
    .balign 128
    b fatal_svc
    .balign 128
    b fatal_svc
    .balign 128
    b fatal_svc
    .balign 128
    b fatal_svc
    .balign 128
    b fatal_svc
    .balign 128
    b fatal_svc
    .balign 128
    b fatal_svc
    .balign 128

    .section .text.svc_handler, "ax"
svc_handler:
    sub sp, sp, #272
    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    str x30, [sp, #240]
    mrs x0, SP_EL0
    str x0, [sp, #248]
    mrs x0, ELR_EL1
    str x0, [sp, #256]
    mrs x0, SPSR_EL1
    str x0, [sp, #264]
    mrs x1, ESR_EL1
    mov x0, sp
    bl svc_dispatch
    cbz x0, return_same
    cmp x0, #1
    b.eq switch_to_b
    cmp x0, #2
    b.eq switch_to_a
    cmp x0, #3
    b.eq finish
    b fatal_svc
return_same:
    mov x0, sp
    add sp, sp, #272
    b restore_frame
switch_to_b:
    // Preserve A's response frame in kernel memory before switching roots.
    mov x1, sp
    adrp x2, A_FRAME
    add x2, x2, :lo12:A_FRAME
    mov x3, #34
1:  ldr x4, [x1], #8
    str x4, [x2], #8
    subs x3, x3, #1
    b.ne 1b
    adrp x1, CURRENT
    add x1, x1, :lo12:CURRENT
    mov x2, #1
    str x2, [x1]
    adrp x1, ROOTS
    add x1, x1, :lo12:ROOTS
    ldr x2, [x1, #8]
    dsb ishst
    msr TTBR0_EL1, x2
    isb
    tlbi vmalle1
    dsb ish
    isb
    add sp, sp, #272
    adrp x0, B_FRAME
    add x0, x0, :lo12:B_FRAME
    b restore_frame
switch_to_a:
    adrp x1, CURRENT
    add x1, x1, :lo12:CURRENT
    str xzr, [x1]
    adrp x1, ROOTS
    add x1, x1, :lo12:ROOTS
    ldr x2, [x1]
    dsb ishst
    msr TTBR0_EL1, x2
    isb
    tlbi vmalle1
    dsb ish
    isb
    add sp, sp, #272
    adrp x0, A_FRAME
    add x0, x0, :lo12:A_FRAME
    b restore_frame

    .global start_first
start_first:
restore_frame:
    mov x30, x0
    ldr x1, [x30, #248]
    msr SP_EL0, x1
    ldr x1, [x30, #256]
    msr ELR_EL1, x1
    ldr x1, [x30, #264]
    msr SPSR_EL1, x1
    ldp x0, x1, [x30, #0]
    ldp x2, x3, [x30, #16]
    ldp x4, x5, [x30, #32]
    ldp x6, x7, [x30, #48]
    ldp x8, x9, [x30, #64]
    ldp x10, x11, [x30, #80]
    ldp x12, x13, [x30, #96]
    ldp x14, x15, [x30, #112]
    ldp x16, x17, [x30, #128]
    ldp x18, x19, [x30, #144]
    ldp x20, x21, [x30, #160]
    ldp x22, x23, [x30, #176]
    ldp x24, x25, [x30, #192]
    ldp x26, x27, [x30, #208]
    ldr x28, [x30, #224]
    ldr x29, [x30, #232]
    ldr x30, [x30, #240]
    eret
fatal_svc:
    msr daifset, #0xf
    b guest_fatal

    .section .usertext, "ax"
    .global user_a
user_a:
    mov x0, #0x50000000
    add x0, x0, #1, lsl #12
    sub x0, x0, #2
    mov x1, #5
    svc #0x10
    .global a_after_badptr
a_after_badptr:
    cmp x0, #1
    b.ne user_fail
    mov x0, #-2
    mov x1, #5
    svc #0x10
    .global a_after_overflow
a_after_overflow:
    cmp x0, #1
    b.ne user_fail
    mov x0, #0x50000000
    mov x1, #17
    svc #0x10
    .global a_after_badlen
a_after_badlen:
    cmp x0, #2
    b.ne user_fail
    mov x0, #0x50000000
    mov w2, #72
    strb w2, [x0, #0]
    mov w2, #69
    strb w2, [x0, #1]
    mov w2, #76
    strb w2, [x0, #2]
    strb w2, [x0, #3]
    mov w2, #79
    strb w2, [x0, #4]
    mov x1, #5
    svc #0x10
    .global a_after_send
a_after_send:
    cbnz x0, user_fail
    mov x0, #0x50000000
    add x0, x0, #32
    mov x1, #16
    svc #0x13
    .global a_after_reply
a_after_reply:
    cbnz x0, user_fail
    cmp x1, #2
    b.ne user_fail
    mov x2, #0x50000000
    ldrb w3, [x2, #32]
    cmp w3, #79
    b.ne user_fail
    ldrb w3, [x2, #33]
    cmp w3, #75
    b.ne user_fail
    mov x0, #0xbeef
    svc #0x14
    .global a_after_finish
a_after_finish:
    b user_fail

    .global user_b
user_b:
    // A's page is not B's backing page, so B's corresponding first byte is zero.
    mov x2, #0x50000000
    ldrb w3, [x2]
    cbnz w3, user_fail
    mov x0, #0x50000000
    add x0, x0, #32
    mov x1, #16
    svc #0x11
    .global b_after_receive
b_after_receive:
    cbnz x0, user_fail
    cmp x1, #5
    b.ne user_fail
    mov x2, #0x50000000
    ldrb w3, [x2, #32]
    cmp w3, #72
    b.ne user_fail
    ldrb w3, [x2, #33]
    cmp w3, #69
    b.ne user_fail
    ldrb w3, [x2, #34]
    cmp w3, #76
    b.ne user_fail
    ldrb w3, [x2, #35]
    cmp w3, #76
    b.ne user_fail
    ldrb w3, [x2, #36]
    cmp w3, #79
    b.ne user_fail
    mov x0, #0x50000000
    add x0, x0, #64
    mov w2, #79
    strb w2, [x0]
    mov w2, #75
    strb w2, [x0, #1]
    mov x1, #2
    svc #0x12
    .global b_after_reply
b_after_reply:
user_fail:
    svc #0x7bad
    b user_fail
