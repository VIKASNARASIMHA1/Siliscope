OUTPUT_ARCH("riscv")
ENTRY(_start)
SECTIONS
{
  . = 0x80000000;
  .text : { *(.text.init) *(.text .text.*) }
  .rodata : { *(.srodata .srodata.*) *(.rodata .rodata.*) }
  .data : { *(.sdata .sdata.*) *(.data .data.*) }
  .bss : { *(.sbss .sbss.*) *(.bss .bss.*) *(COMMON) }
  . = ALIGN(16);
  . += 0x40000;
  _stack_top = .;
}
