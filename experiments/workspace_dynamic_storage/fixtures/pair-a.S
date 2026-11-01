// First external ELF: use the same virtual address as pair B, but a distinct
// marker, then wait until both live tasks have each suffered a timer IRQ.
.text
.globl _pair_a
_pair_a:
    mov x20, x0
    mov x21, x1
    mov x2, #0xa1a1
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
