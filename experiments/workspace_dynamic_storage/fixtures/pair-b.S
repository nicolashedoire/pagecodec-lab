// Second external ELF: same virtual address, independent private backing.
.text
.globl _pair_b
_pair_b:
    mov x20, x0
    mov x21, x1
    mov x2, #0xb2b2
    cmp x21, x2
    b.ne failure
    str x21, [x20]
    mov x22, #0x100000
wait:
    svc #0x150
    cbz x0, ready
    cmn x0, #11
    b.ne failure
    subs x22, x22, #1
    b.ne wait
    b failure
ready:
    ldr x2, [x20]
    cmp x2, x21
    b.ne failure
    mov x0, #0
    svc #0x123
failure:
    mov x0, #42
    svc #0x123
