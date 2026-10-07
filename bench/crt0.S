# Bare-metal startup for benchmark programs (runs in M-mode, no MMU).
.section .text.init
.globl _start
_start:
  la sp, _stack_top
  call main
  li t0, 0x100000            # SiFive test finisher
  bnez a0, 1f
  li t1, 0x5555              # pass
  sw t1, 0(t0)
2: j 2b
1:
  slli a0, a0, 16
  li t1, 0x3333
  or a0, a0, t1              # fail(code)
  sw a0, 0(t0)
  j 2b
