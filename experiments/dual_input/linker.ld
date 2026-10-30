ENTRY(_start)
SECTIONS
{
  . = 0x40080000;
  .text : ALIGN(16) { KEEP(*(.text.boot)) *(.text .text.*) }
  .rodata : ALIGN(16) { *(.rodata .rodata.*) }
  .data : ALIGN(16) { *(.data .data.*) }
  .bss (NOLOAD) : ALIGN(16) {
    __bss_start = .;
    *(.bss .bss.* COMMON)
    . = ALIGN(16);
    __bss_end = .;
    . += 0x10000;
    __stack_top = .;
  }
  /DISCARD/ : { *(.eh_frame*) *(.comment*) }
}
