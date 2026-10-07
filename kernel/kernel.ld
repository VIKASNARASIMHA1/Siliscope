OUTPUT_ARCH("riscv")
ENTRY(_entry)
SECTIONS
{
  . = 0x80000000;
  .text : { *(.text.entry) *(.text .text.*) . = ALIGN(0x1000); etext = .; }
  .rodata : { . = ALIGN(16); *(.srodata .srodata.*) . = ALIGN(16); *(.rodata .rodata.*) }
  .data : { . = ALIGN(16); *(.sdata .sdata.*) *(.data .data.*) }
  .bss : { . = ALIGN(16); *(.sbss .sbss.*) *(.bss .bss.*) *(COMMON) . = ALIGN(0x1000); }
  end = .;
}
