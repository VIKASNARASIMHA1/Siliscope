# Entry point: runs in M-mode at 0x80000000 with a 4 KiB boot stack.
.section .text.entry
.globl _entry
_entry:
  csrr a0, mhartid
  bnez a0, park
  la sp, stack0
  li a0, 16384
  add sp, sp, a0
  call start
park:
  wfi
  j park

.section .bss
.align 4
.globl stack0
stack0:
  .space 16384
