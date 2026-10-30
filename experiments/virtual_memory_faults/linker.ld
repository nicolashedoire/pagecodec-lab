ENTRY(_start)
SECTIONS
{
  . = 0x40080000;
  __image_start = .;
  .text : ALIGN(4096) {
    __text_start = .;
    KEEP(*(.text.boot)) *(.text .text.*)
    . = ALIGN(4096);
    __text_end = .;
  }
  .rodata : ALIGN(4096) {
    __rodata_start = .;
    *(.rodata .rodata.*)
    /* A mutable Rust test page is deliberately mapped read-only by the guest MMU. */
    KEEP(*(.roprobe))
    . = ALIGN(4096);
    __rodata_end = .;
  }
  .data : ALIGN(4096) {
    __data_start = .;
    *(.data .data.*)
    . = ALIGN(4096);
  }
  .bss (NOLOAD) : ALIGN(4096) {
    __bss_start = .;
    *(.bss .bss.* COMMON)
    . = ALIGN(16);
    __bss_end = .;
    . += 0x10000;
    __stack_top = .;
    . = ALIGN(4096);
    __image_end = .;
  }
  /DISCARD/ : { *(.eh_frame*) *(.comment*) }
}
