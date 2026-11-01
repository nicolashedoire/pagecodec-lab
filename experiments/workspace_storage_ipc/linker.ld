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
  .usertext : ALIGN(4096) {
    __usertext_start = .;
    KEEP(*(.usertext))
    . = ALIGN(4096);
    __usertext_end = .;
  }
  .rodata : ALIGN(4096) {
    __rodata_start = .;
    *(.rodata .rodata.*)
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
    . = ALIGN(4096);
    __stack_guard_start = .;
    . += 0x1000;
    __stack_bottom = .;
    . += 0x40000;
    __stack_top = .;
    . = ALIGN(4096);
  }
  .userstack (NOLOAD) : ALIGN(4096) {
    __userstack_start = .;
    KEEP(*(.userstack))
    . = ALIGN(4096);
    __userstack_end = .;
    __image_end = .;
  }
  /DISCARD/ : { *(.eh_frame*) *(.comment*) }
}
