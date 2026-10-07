# Run by "rvsim test" and "rvsim run-selftest": a scripted tour of the OS.
echo === shell, redirection and files ===
ls
echo hello-from-the-shell > /hello.out
cat /hello.out
mkdir /d ; echo in-dir > /d/file ; ls /d ; cat /d/file
rm /d/file ; rm /d
wc /README.txt
echo === in-guest regression suite ===
ktest
echo === scheduler demo ===
schedtest
echo === pipes and pipelines ===
echo hello world | wc
cat README.txt | wc
ls | grep sh
cat README.txt | grep rvsim | wc
echo === copy-on-write fork benchmark ===
forkbench
