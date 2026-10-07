.macro sys name, num
.globl \name
\name:
  li a7, \num
  ecall
  ret
.endm
sys fork, 1
sys exit, 2
sys wait, 3
sys read, 4
sys write, 5
sys open, 6
sys close, 7
sys kill, 8
sys exec, 9
sys fstat, 10
sys chdir, 11
sys dup, 12
sys getpid, 13
sys sbrk, 14
sys sleep, 15
sys uptime, 16
sys unlink, 17
sys mkdir, 18
sys halt, 19
sys ps, 20
sys pipe, 21
sys freepages, 22
sys vmctl, 23
