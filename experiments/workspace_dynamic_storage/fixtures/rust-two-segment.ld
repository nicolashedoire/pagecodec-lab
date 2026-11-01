ENTRY(_start)
PHDRS {
  text PT_LOAD FLAGS(5);
  data PT_LOAD FLAGS(6);
}
SECTIONS {
  . = 0x50001000;
  .text : ALIGN(4) { *(.text._start) *(.text*) *(.rodata*) } :text
  . = 0x50000000;
  .data : ALIGN(8) { *(.data*) } :data
  .bss (NOLOAD) : ALIGN(8) { *(.bss*) *(COMMON) } :data
  /DISCARD/ : { *(.eh_frame*) *(.comment*) *(.note*) }
}
