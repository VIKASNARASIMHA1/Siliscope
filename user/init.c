// First process: attach the console as fds 0/1/2, then run the shell.
#include "user.h"

int main(void) {
  if (open("console", O_RDWR) < 0) { exit(1); }
  dup(0); dup(0);
  printf("init: starting sh\n");
  for (;;) {
    int pid = fork();
    if (pid < 0) { printf("init: fork failed\n"); halt(); }
    if (pid == 0) {
      char *argv[] = {"sh", 0};
      exec("sh", argv);
      printf("init: exec sh failed\n");
      exit(1);
    }
    int st;
    while (wait(&st) != pid) {}
    printf("init: shell exited, powering off\n");
    halt();
  }
}
