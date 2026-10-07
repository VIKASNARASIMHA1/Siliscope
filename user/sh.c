// A small shell: commands with args, pipelines (a | b | c), '<' and '>' redirection,
// ';' sequencing, '&' background jobs, and built-ins cd/exit.  "sh script" runs a script file.
#include "user.h"

#define MAXLINE 256
#define MAXSTAGES 8

static int script_mode;

// Split a command into words (in place); fills in redirection targets.
static int tokenize(char *cmd, char **argv, char **infile, char **outfile) {
  int argc = 0;
  char *p = cmd;
  *infile = *outfile = 0;
  while (*p) {
    while (*p == ' ' || *p == '\t') *p++ = 0;
    if (!*p) break;
    if (*p == '<' || *p == '>') {
      char op = *p;
      *p++ = 0;
      while (*p == ' ') p++;
      char *name = p;
      while (*p && *p != ' ' && *p != '\t' && *p != '<' && *p != '>') p++;
      if (*p) *p++ = 0;
      if (op == '<') *infile = name; else *outfile = name;
      continue;
    }
    if (argc >= 15) { fprintf(2, "sh: too many arguments\n"); return -1; }
    argv[argc++] = p;
    while (*p && *p != ' ' && *p != '\t' && *p != '<' && *p != '>') p++;
  }
  argv[argc] = 0;
  return argc;
}

// Runs in a forked child: apply redirections, then exec. Never returns.
static void child_exec(char *cmd) {
  char *argv[16], *infile, *outfile;
  int argc = tokenize(cmd, argv, &infile, &outfile);
  if (argc <= 0) exit(argc < 0 ? 1 : 0);
  if (infile) { close(0); if (open(infile, O_RDONLY) < 0) { fprintf(2, "sh: cannot open %s\n", infile); exit(1); } }
  if (outfile) { close(1); if (open(outfile, O_WRONLY | O_CREATE) < 0) { fprintf(2, "sh: cannot create %s\n", outfile); exit(1); } }
  exec(argv[0], argv);
  fprintf(2, "sh: %s: command not found\n", argv[0]);
  exit(127);
}

static int run_command(char *line, int background) {
  char *stages[MAXSTAGES];
  int n = 0;
  stages[n++] = line;
  for (char *p = line; *p; p++) {
    if (*p == '|') {
      if (n >= MAXSTAGES) { fprintf(2, "sh: pipeline too long\n"); return -1; }
      *p = 0;
      stages[n++] = p + 1;
    }
  }
  if (n == 1) {                       // built-ins only make sense without a pipeline
    char tmp[MAXLINE], *argv[16], *in, *out;
    strcpy(tmp, line);
    int argc = tokenize(tmp, argv, &in, &out);
    if (argc <= 0) return 0;
    if (!strcmp(argv[0], "cd")) {
      if (argc < 2 || chdir(argv[1]) < 0) printf("cd: cannot cd to %s\n", argc < 2 ? "(none)" : argv[1]);
      return 0;
    }
    if (!strcmp(argv[0], "exit")) exit(0);
  }
  int fds[MAXSTAGES][2];
  for (int i = 0; i < n - 1; i++)
    if (pipe(fds[i]) < 0) { fprintf(2, "sh: pipe failed\n"); return -1; }
  int pids[MAXSTAGES], started = 0;
  for (int i = 0; i < n; i++) {
    int pid = fork();
    if (pid < 0) { fprintf(2, "sh: fork failed\n"); break; }
    if (pid == 0) {
      if (i > 0) { close(0); dup(fds[i - 1][0]); }       // stdin  <- previous stage
      if (i < n - 1) { close(1); dup(fds[i][1]); }       // stdout -> next stage
      for (int j = 0; j < n - 1; j++) { close(fds[j][0]); close(fds[j][1]); }
      child_exec(stages[i]);
    }
    pids[started++] = pid;
  }
  for (int j = 0; j < n - 1; j++) { close(fds[j][0]); close(fds[j][1]); }   // parent keeps no pipe ends
  if (background) { printf("[%d] started in background\n", pids[started - 1]); return 0; }
  int st = 0;
  for (int i = 0; i < started; i++) wait(&st);
  return st;
}

static void run_line(char *line) {
  char *start = line;
  for (char *p = line;; p++) {
    if (*p == ';' || *p == '\n' || *p == 0) {
      char end = *p;
      *p = 0;
      int bg = 0;
      int len = strlen(start);
      while (len > 0 && (start[len - 1] == ' ' || start[len - 1] == '\r')) len--;
      start[len] = 0;
      if (len > 0 && start[len - 1] == '&') { bg = 1; start[len - 1] = 0; }
      run_command(start, bg);
      if (end == 0 || end == '\n') break;
      start = p + 1;
    }
  }
}

int main(int argc, char **argv) {
  static char buf[MAXLINE];
  int fd = 0;
  if (argc > 1) {
    fd = open(argv[1], O_RDONLY);
    if (fd < 0) { printf("sh: cannot open %s\n", argv[1]); return 1; }
    script_mode = 1;
  }
  for (;;) {
    if (!script_mode) printf("$ ");
    int i = 0;
    for (;;) {                       // read one line
      char c;
      if (read(fd, &c, 1) < 1) { if (i == 0) return 0; break; }
      if (c == '\n') break;
      if (i < MAXLINE - 1) buf[i++] = c;
    }
    buf[i] = 0;
    if (script_mode) {
      if (i == 0 || buf[0] == '#') continue;
      printf("+ %s\n", buf);
    }
    run_line(buf);
  }
}
