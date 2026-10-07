OUTPUT_ARCH("riscv")
ENTRY(_start)
SECTIONS
{
  . = 0x40000000;
  .text : { *(.text .text.*) }
  .rodata : { *(.srodata .srodata.*) *(.rodata .rodata.*) }
  . = ALIGN(0x1000);
  .data : { *(.sdata .sdata.*) *(.data .data.*) }
  .bss : { *(.sbss .sbss.*) *(.bss .bss.*) *(COMMON) }
  . = ALIGN(0x1000);
  end = .;
}
