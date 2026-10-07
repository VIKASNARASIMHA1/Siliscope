//! Cross-platform build / run / test / bench driver.
//!
//!   cargo run --release -p xtask -- <command>
//!
//! Commands: build | run | test | bench | cowbench | vmbench | tlbbench | pipebench | selftest | clean
//! (On Windows use rvsim.ps1, on Linux/macOS use ./rvsim.sh -- thin wrappers.)

mod mkfs;
mod paging;

use std::path::{Path, PathBuf};
use std::process::{exit, Command, Stdio};

const TARGET_FLAGS: &[&str] = &[
    "--target=riscv64-unknown-elf", "-march=rv64imac_zicsr_zifencei", "-mabi=lp64",
    "-mcmodel=medany", "-mno-relax", "-O2", "-ffreestanding", "-fno-common", "-fno-stack-protector", "-Wall",
    "-Wno-unused-function",
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn fatal(msg: &str) -> ! {
    eprintln!("\nerror: {}", msg);
    exit(1);
}

fn exe(name: &str) -> String {
    format!("{}{}", name, std::env::consts::EXE_SUFFIX)
}

fn rvsim_path() -> PathBuf {
    root().join("target").join("release").join(exe("rvsim"))
}

fn run_checked(cmd: &mut Command, what: &str) {
    match cmd.status() {
        Ok(s) if s.success() => {}
        Ok(_) => fatal(&format!("{} failed", what)),
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                fatal(&format!(
                    "could not run `{}` (not found on PATH).\n  Windows: winget install LLVM.LLVM  (then reopen the terminal)\n  Linux:   sudo apt install clang lld\n  macOS:   brew install llvm lld",
                    cmd.get_program().to_string_lossy()
                ));
            }
            fatal(&format!("{}: {}", what, e));
        }
    }
}

fn clang() -> Command {
    let mut c = Command::new("clang");
    c.args(TARGET_FLAGS).current_dir(root());
    c
}

fn compile(src: &Path, obj: &Path) {
    compile_with(src, obj, &[]);
}

fn compile_with(src: &Path, obj: &Path, extra: &[&str]) {
    std::fs::create_dir_all(obj.parent().unwrap()).unwrap();
    let mut c = clang();
    c.args(extra).arg("-c").arg(src).arg("-o").arg(obj);
    run_checked(&mut c, &format!("compiling {}", src.display()));
}

fn link(objs: &[PathBuf], script: &str, out: &Path) {
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    let mut c = clang();
    c.args(["-nostdlib", "-fuse-ld=lld", "-Wl,-z,max-page-size=4096"]).arg(format!("-Wl,-T,{}", script));
    c.args(objs).arg("-o").arg(out);
    run_checked(&mut c, &format!("linking {}", out.display()));
}

fn sources(dir: &Path, skip: &[&str]) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|_| fatal(&format!("missing directory {}", dir.display())))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("c") | Some("S")))
        .filter(|p| !skip.contains(&p.file_name().unwrap().to_str().unwrap()))
        .collect();
    v.sort();
    v
}

fn build_emulator() {
    println!("==> building emulator (cargo, release)");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    run_checked(
        Command::new(cargo).args(["build", "--release", "-p", "rvsim"]).current_dir(root()),
        "cargo build",
    );
}

fn build_kernel() {
    println!("==> building kernel");
    build_kernel_variant(&[], "kernel", "kernel.elf");
}

/// Build the kernel with extra compiler flags (e.g. -DCOW_FORK=0) into its own object dir.
fn build_kernel_variant(extra: &[&str], objdir: &str, elf: &str) {
    let r = root();
    let objs: Vec<PathBuf> = sources(&r.join("kernel"), &[])
        .iter()
        .map(|s| {
            let o = r.join("build").join(objdir).join(format!("{}.o", s.file_name().unwrap().to_str().unwrap()));
            compile_with(s, &o, extra);
            o
        })
        .collect();
    link(&objs, "kernel/kernel.ld", &r.join("build").join(elf));
}

fn build_user() -> Vec<(String, PathBuf)> {
    println!("==> building user programs");
    let r = root();
    let lib: Vec<PathBuf> = ["ulib.c", "usys.S"]
        .iter()
        .map(|f| {
            let o = r.join("build/user_obj").join(format!("{}.o", f));
            compile(&r.join("user").join(f), &o);
            o
        })
        .collect();
    let mut progs = Vec::new();
    for s in sources(&r.join("user"), &["ulib.c", "usys.S"]) {
        let name = s.file_stem().unwrap().to_str().unwrap().to_string();
        let o = r.join("build/user_obj").join(format!("{}.o", name));
        compile(&s, &o);
        let out = r.join("build/user").join(&name);
        let mut objs = vec![o];
        objs.extend(lib.iter().cloned());
        link(&objs, "user/user.ld", &out);
        progs.push((name, out));
    }
    progs
}

fn build_fs(progs: &[(String, PathBuf)]) {
    println!("==> building file system image (build/fs.img)");
    let r = root();
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for (n, p) in progs {
        files.push((n.clone(), std::fs::read(p).unwrap()));
    }
    if let Ok(rd) = std::fs::read_dir(r.join("rootfs")) {
        let mut v: Vec<_> = rd.filter_map(|e| e.ok()).collect();
        v.sort_by_key(|e| e.file_name());
        for e in v {
            if e.path().is_file() {
                files.push((e.file_name().to_string_lossy().to_string(), std::fs::read(e.path()).unwrap()));
            }
        }
    }
    let img = mkfs::build(&files).unwrap_or_else(|e| fatal(&format!("mkfs: {}", e)));
    std::fs::create_dir_all(r.join("build")).unwrap();
    std::fs::write(r.join("build/fs.img"), img).unwrap();
}

fn build_bench() {
    println!("==> building benchmarks");
    let r = root();
    let crt = r.join("build/bench_obj/crt0.o");
    let common = r.join("build/bench_obj/common.o");
    compile(&r.join("bench/crt0.S"), &crt);
    compile(&r.join("bench/common.c"), &common);
    for s in sources(&r.join("bench"), &["crt0.S", "common.c"]) {
        let name = s.file_stem().unwrap().to_str().unwrap().to_string();
        let o = r.join("build/bench_obj").join(format!("{}.o", name));
        compile(&s, &o);
        link(&[crt.clone(), o, common.clone()], "bench/bench.ld", &r.join("build/bench").join(format!("{}.elf", name)));
    }
}

fn build_all() {
    build_emulator();
    build_kernel();
    let progs = build_user();
    build_fs(&progs);
    build_bench();
    println!("\nBuild complete. Try:  run");
}

fn kernel_args() -> Vec<String> {
    let r = root();
    vec![
        r.join("build/kernel.elf").to_string_lossy().into(),
        "--disk".into(),
        r.join("build/fs.img").to_string_lossy().into(),
    ]
}

fn cmd_run(extra: &[String]) -> i32 {
    build_all();
    println!("\n--- starting guest (type commands; `halt` powers off; Ctrl-C quits the emulator) ---\n");
    let st = Command::new(rvsim_path()).args(kernel_args()).args(extra).status().unwrap_or_else(|e| fatal(&e.to_string()));
    st.code().unwrap_or(1)
}

fn run_capture(args: &[String]) -> (i32, String) {
    let out = Command::new(rvsim_path())
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap_or_else(|e| fatal(&format!("cannot run rvsim: {}", e)));
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned())
}

fn run_full(args: &[String]) -> (i32, String, String) {
    let out = Command::new(rvsim_path())
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap_or_else(|e| fatal(&format!("cannot run rvsim: {}", e)));
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Pull `RESULT <kind> key=value ...` lines (printed by the guest benchmark programs) out of the console output.
fn parse_results(out: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut v = Vec::new();
    for line in out.lines() {
        if let Some(i) = line.find("RESULT ") {
            let mut parts = line[i + 7..].split_whitespace();
            let kind = parts.next().unwrap_or("").to_string();
            let kv: Vec<(String, String)> = parts
                .filter_map(|p| p.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
                .collect();
            v.push((kind, kv));
        }
    }
    v
}

fn get<'a>(r: &'a [(String, String)], k: &str) -> &'a str {
    r.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.as_str()).unwrap_or("")
}

fn geti(r: &[(String, String)], k: &str) -> i64 {
    get(r, k).parse().unwrap_or(0)
}

fn write_csv(path: &Path, rows: &[&Vec<(String, String)>]) {
    if rows.is_empty() {
        return;
    }
    let mut s = rows[0].iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(",") + "\n";
    for r in rows {
        s += &(r.iter().map(|(_, v)| v.as_str()).collect::<Vec<_>>().join(",") + "\n");
    }
    std::fs::write(path, s).unwrap();
}

fn kernel_selftest(show: bool) -> bool {
    let r = root();
    let mut args = kernel_args();
    args.extend(["--snapshot", "--quiet", "--max-insns", "3000000000", "--uart-input"].map(String::from));
    args.push(r.join("tests/kernel_test.in").to_string_lossy().into());
    let (code, out) = run_capture(&args);
    if show {
        println!("{}", out);
    }
    let ok = code == 0
        && out.contains("ktest: ALL PASSED")
        && out.contains("halt: powering off")
        && out.contains("\n1 2 12\n")                       // `echo hello world | wc` through a pipe
        && out.contains("forkbench: 20 forks done");
    if !ok {
        println!("{}", out);
    }
    ok
}

fn cmd_selftest() -> i32 {
    build_all();
    println!("\n--- running the scripted guest self-test (output below) ---\n");
    if kernel_selftest(true) { 0 } else { 1 }
}

fn cmd_test() -> i32 {
    build_all();
    let r = root();
    println!("\n=== riscv-tests (official ISA conformance suite) ===");
    let mut files: Vec<PathBuf> = std::fs::read_dir(r.join("tests/riscv-tests"))
        .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.file_name().unwrap().to_str().unwrap().starts_with("rv64")).collect())
        .unwrap_or_default();
    files.sort();
    let (mut pass, mut fail) = (0, 0);
    for f in &files {
        let (code, _) = run_capture(&[f.to_string_lossy().into(), "--quiet".into(), "--max-insns".into(), "5000000".into()]);
        if code == 0 { pass += 1 } else { fail += 1; println!("  FAIL {} (exit {})", f.file_name().unwrap().to_string_lossy(), code) }
    }
    println!("  passed {} / {}", pass, pass + fail);

    println!("\n=== guest OS self-test (boot, shell, fork/exec, VM, fs, scheduler) ===");
    let kok = kernel_selftest(false);
    println!("  {}", if kok { "PASS: kernel booted and `ktest` reported ALL PASSED" } else { "FAIL" });

    println!("\n=== demand paging / swapping / scheduler checks ===");
    let (raw_out, res) = run_vmbench();
    let swaps: Vec<_> = res.iter().filter(|(k, _)| k == "swap").map(|(_, v)| v).collect();
    let corrupt: i64 = swaps.iter().map(|v| geti(v, "corrupt")).sum();
    let leaks = res.iter().filter(|(k, _)| k == "swap-leak").count();
    let lazy_ok = {
        let e = res.iter().find(|(k, v)| k == "lazy" && get(v, "mode") == "eager").map(|(_, v)| geti(v, "frames_used")).unwrap_or(0);
        let l = res.iter().find(|(k, v)| k == "lazy" && get(v, "mode") == "lazy").map(|(_, v)| geti(v, "frames_used")).unwrap_or(i64::MAX);
        e > 0 && l * 10 < e
    };
    let sched_ok = {
        let s = |p: &str| res.iter().find(|(k, v)| k == "sched" && get(v, "policy") == p).map(|(_, v)| v.clone());
        match (s("RR"), s("MLFQ")) {
            (Some(rr), Some(ml)) => geti(&ml, "interactive_wait_x100") < geti(&rr, "interactive_wait_x100") && geti(&ml, "short_turnaround_x10") < geti(&rr, "short_turnaround_x10"),
            _ => false,
        }
    };
    let orows = opt_comparison(&raw_out, &res);
    let opt_ok = orows.len() == 10
        && orows.iter().all(|o| o.opt <= o.sim && (o.policy == "LRU-exact" || (o.kernel >= o.opt)))   // nothing beats OPT
        && orows.iter().filter(|o| o.policy == "FIFO").all(|o| o.kernel == o.sim);                      // kernel FIFO == textbook FIFO
    let vm_ok = swaps.len() == 8 && corrupt == 0 && leaks == 0 && lazy_ok && sched_ok && opt_ok;
    println!("  swap runs: {} (corrupt pages: {}, leaks: {}),  lazy sbrk saves >10x frames: {},  MLFQ beats RR on wait + short turnaround: {},  OPT is a lower bound and kernel FIFO matches the simulator: {}", swaps.len(), corrupt, leaks, lazy_ok, sched_ok, opt_ok);
    println!("  {}", if vm_ok { "PASS" } else { "FAIL" });

    println!("\n=== cycle-by-cycle pipeline model vs formula model ===");
    let pv = run_pipeline_validation(Some(&["fib", "matmul", "qsort"]));
    let worst = pv.iter().map(|(_, _, f, c, _)| 100.0 * (f - c).abs() / c.max(1e-9)).fold(0.0, f64::max);
    let pipe_ok = pv.len() == 18 && worst < 3.0;
    println!("  fib/matmul/qsort x 6 predictors: largest CPI difference {:.2}% (limit 3%)  {}", worst, if pipe_ok { "PASS" } else { "FAIL" });

    println!("\n=== benchmark self-checks ===");
    let mut bfail = 0;
    let mut benches: Vec<PathBuf> = std::fs::read_dir(r.join("build/bench")).unwrap().filter_map(|e| e.ok().map(|e| e.path())).collect();
    benches.sort();
    for b in &benches {
        let (code, out) = run_capture(&[b.to_string_lossy().into(), "--quiet".into(), "--max-insns".into(), "500000000".into()]);
        let ok = code == 0;
        if !ok { bfail += 1 }
        println!("  {} {}: {}", if ok { "ok  " } else { "FAIL" }, b.file_stem().unwrap().to_string_lossy(), out.lines().next().unwrap_or(""));
    }
    if fail == 0 && kok && vm_ok && pipe_ok && bfail == 0 {
        println!("\nALL TESTS PASSED");
        0
    } else {
        println!("\nSOME TESTS FAILED");
        1
    }
}

fn cmd_bench() -> i32 {
    build_all();
    let r = root();
    let res = r.join("results");
    std::fs::create_dir_all(&res).unwrap();
    println!("\n=== running benchmarks with the analyzer + design-space sweeps (this takes a few minutes) ===");
    let mut benches: Vec<PathBuf> = std::fs::read_dir(r.join("build/bench")).unwrap().filter_map(|e| e.ok().map(|e| e.path())).collect();
    benches.sort();
    let mut jobs: Vec<(String, Vec<String>)> = Vec::new();
    for b in &benches {
        let name = b.file_stem().unwrap().to_string_lossy().to_string();
        jobs.push((name, vec![b.to_string_lossy().into()]));
    }
    let mut kargs = kernel_args();
    kargs.extend(["--snapshot", "--uart-input"].map(String::from));
    kargs.push(r.join("tests/kernel_bench.in").to_string_lossy().into());
    jobs.push(("kernel_ktest".into(), kargs));
    for (name, mut args) in jobs {
        println!("  -> {}", name);
        args.extend(["--sweep", "--quiet", "--max-insns", "2000000000", "--perf-out"].map(String::from));
        args.push(res.join(&name).to_string_lossy().into());
        let (_, out) = run_capture(&args);
        std::fs::write(res.join(format!("{}_report.txt", name)), out).unwrap();
    }
    println!("\nCSV + text reports written to results/. Generating charts (needs Python + matplotlib)...");
    for py in ["python3", "python"] {
        if Command::new(py).arg(r.join("tools/plot.py")).arg(&res).current_dir(&r).status().map(|s| s.success()).unwrap_or(false) {
            println!("Charts written to results/*.png");
            return 0;
        }
    }
    println!("(skipped charts: install Python and `pip install matplotlib`, then run `python tools/plot.py results`)");
    0
}

/// Run lazybench + swapbench + schedbench in one guest boot; returns the parsed RESULT lines.
fn run_vmbench() -> (String, Vec<(String, Vec<(String, String)>)>) {
    let r = root();
    let mut args = kernel_args();
    args.extend(["--snapshot", "--quiet", "--max-insns", "6000000000", "--uart-input"].map(String::from));
    args.push(r.join("tests/vmbench.in").to_string_lossy().into());
    let (code, out, _) = run_full(&args);
    if code != 0 {
        println!("{}", out);
        fatal("vmbench guest run failed");
    }
    std::fs::create_dir_all(r.join("results")).unwrap();
    let raw: String = out.lines().filter_map(|l| l.find("RESULT ").map(|i| format!("{}\n", &l[i..]))).collect();
    std::fs::write(r.join("results/vm_bench.txt"), raw).unwrap();
    let parsed = parse_results(&out);
    (out, parsed)
}

/// Reference strings printed by swapbench: (pattern, frames, refs).
fn parse_refstrings(out: &str, frames: usize) -> Vec<(String, usize, Vec<u32>)> {
    let mut v = Vec::new();
    for line in out.lines() {
        if let Some(i) = line.find("REFSTRING pattern=") {
            let rest = &line[i + "REFSTRING pattern=".len()..];
            if let Some((name, nums)) = rest.split_once(" : ") {
                let refs: Vec<u32> = nums.split_whitespace().filter_map(|t| t.parse().ok()).collect();
                v.push((name.to_string(), frames, refs));
            }
        }
    }
    v
}

/// One row of the kernel-vs-simulation-vs-OPT comparison.
struct OptRow { pattern: String, policy: String, kernel: i64, sim: i64, opt: i64 }

/// Replay each reference string through the ideal simulators and Belady OPT and line the results up
/// with the major faults the kernel measured. Returns rows plus the per-pattern compulsory/OPT summary.
fn opt_comparison(out: &str, res: &[(String, Vec<(String, String)>)]) -> Vec<OptRow> {
    const FRAMES: usize = 24;
    let strings = parse_refstrings(out, FRAMES);
    let mut rng = paging::KERNEL_RNG_SEED;     // the kernel's RNG state carries over between runs
    let mut rows = Vec::new();
    for (pat, frames, refs) in &strings {
        let comp = paging::distinct(refs) as i64;
        let o = paging::opt(refs, *frames) as i64 - comp;
        let sims: [(&str, i64); 5] = [
            ("FIFO", paging::fifo(refs, *frames) as i64 - comp),
            ("CLOCK", paging::clock(refs, *frames) as i64 - comp),
            ("LRU-aging", paging::aging(refs, *frames) as i64 - comp),
            ("RANDOM", paging::random(refs, *frames, &mut rng) as i64 - comp),
            ("LRU-exact", paging::lru(refs, *frames) as i64 - comp),
        ];
        for (pol, sim) in sims {
            let kernel = res.iter().find(|(k, v)| k == "swap" && get(v, "policy") == pol && get(v, "pattern") == pat)
                .map(|(_, v)| geti(v, "major_faults")).unwrap_or(-1);
            rows.push(OptRow { pattern: pat.clone(), policy: pol.to_string(), kernel, sim, opt: o });
        }
    }
    rows
}

fn print_opt_table(rows: &[OptRow]) {
    let mut pats: Vec<&str> = Vec::new();
    for r in rows { if !pats.contains(&r.pattern.as_str()) { pats.push(&r.pattern); } }
    for pat in pats {
        let opt = rows.iter().find(|r| r.pattern == pat).map(|r| r.opt).unwrap_or(0);
        println!("\n  pattern: {}   (Belady OPT = {} major faults, the theoretical minimum)", pat, opt);
        println!("  {:<10} {:>14} {:>16} {:>14}", "policy", "kernel measured", "ideal simulation", "kernel / OPT");
        for r in rows.iter().filter(|r| r.pattern == pat) {
            let k = if r.kernel >= 0 { r.kernel.to_string() } else { "-".into() };
            let ratio = if r.kernel >= 0 && opt > 0 { format!("{:.2}x", r.kernel as f64 / opt as f64) } else { "-".into() };
            println!("  {:<10} {:>14} {:>16} {:>14}", r.policy, k, r.sim, ratio);
        }
    }
}

fn cmd_vmbench() -> i32 {
    build_all();
    println!("\n=== demand paging, swapping and scheduler benchmarks (about 30 s) ===");
    let (raw_out, res) = run_vmbench();
    let r = root();
    for kind in ["lazy", "swap", "sched"] {
        let rows: Vec<&Vec<(String, String)>> = res.iter().filter(|(k, _)| k == kind).map(|(_, v)| v).collect();
        write_csv(&r.join("results").join(format!("{}_bench.csv", kind)), &rows);
    }
    println!("\n--- lazy sbrk: reserve 32 MiB, touch 1/16 of it ---");
    for (k, v) in res.iter().filter(|(k, _)| k == "lazy") {
        let _ = k;
        println!("  {:<6} frames used: {:>5}   (data intact: {})", get(v, "mode"), get(v, "frames_used"), get(v, "ok"));
    }
    println!("\n--- page replacement: 64-page heap, 24-frame resident limit, {} references ---",
        res.iter().find(|(k, _)| k == "swap").map(|(_, v)| get(v, "refs").to_string()).unwrap_or_default());
    println!("  {:<10} {:<9} {:>13} {:>11} {:>9}", "policy", "pattern", "major faults", "swap-outs", "corrupt");
    for (_, v) in res.iter().filter(|(k, _)| k == "swap") {
        println!("  {:<10} {:<9} {:>13} {:>11} {:>9}", get(v, "policy"), get(v, "pattern"), get(v, "major_faults"), get(v, "swap_outs"), get(v, "corrupt"));
    }
    println!("\n--- page replacement vs Belady's optimal (OPT) on the same reference strings, 24 frames ---");
    let orows = opt_comparison(&raw_out, &res);
    print_opt_table(&orows);
    {
        let mut csv = String::from("pattern,policy,kernel_major,sim_major,opt_major\n");
        for o in &orows { csv += &format!("{},{},{},{},{}\n", o.pattern, o.policy, o.kernel, o.sim, o.opt); }
        std::fs::write(r.join("results/opt_compare.csv"), csv).unwrap();
    }
    println!("\n--- scheduler: 3 long jobs + 2 interactive jobs, 2 short jobs arriving 10 ticks later ---");
    println!("  {:<6} {:>22} {:>22} {:>24} {:>10}", "policy", "short turnaround (ticks)", "long turnaround (ticks)", "interactive wait (ticks)", "total");
    for (_, v) in res.iter().filter(|(k, _)| k == "sched") {
        println!("  {:<6} {:>22.1} {:>22.1} {:>24.2} {:>10}", get(v, "policy"),
            geti(v, "short_turnaround_x10") as f64 / 10.0, geti(v, "long_turnaround_x10") as f64 / 10.0,
            geti(v, "interactive_wait_x100") as f64 / 100.0, get(v, "total_ticks"));
    }
    println!("\nRaw lines: results/vm_bench.txt   CSVs: results/{{lazy,swap,sched}}_bench.csv");
    plot_results();
    0
}

fn plot_results() {
    let r = root();
    for py in ["python3", "python"] {
        if Command::new(py).arg(r.join("tools/plot.py")).arg(r.join("results")).current_dir(&r)
            .stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false) {
            println!("Charts updated in results/*.png");
            return;
        }
    }
    println!("(charts skipped: install Python + `pip install matplotlib`, then `python tools/plot.py results`)");
}

/// TLB size sweep, with the kernel direct map built from 4 KiB pages vs 2 MiB superpages.
fn cmd_tlbbench() -> i32 {
    build_all();
    println!("\n==> building kernel variant with 4 KiB-only mappings");
    build_kernel_variant(&["-DKERNEL_SUPERPAGES=0"], "kernel_4k", "kernel_4k.elf");
    let r = root();
    println!("\n=== TLB sweep: ktest + forkbench workload, direct-mapped 4 KiB TLB of N entries + 16-entry superpage TLB ===");
    let mut rows: Vec<Vec<(String, String)>> = Vec::new();
    println!("  {:<22} {:>8} {:>12} {:>12} {:>10} {:>10}", "kernel mapping", "entries", "TLB hits", "super hits", "walks", "miss rate");
    for (label, elf) in [("4 KiB pages", "kernel_4k.elf"), ("2 MiB superpages", "kernel.elf")] {
        for entries in [4usize, 8, 16, 32, 64, 128, 256] {
            let args: Vec<String> = vec![
                r.join("build").join(elf).to_string_lossy().into(), "--disk".into(), r.join("build/fs.img").to_string_lossy().into(),
                "--snapshot".into(), "--quiet".into(), "--tlb-stats".into(), "--tlb-entries".into(), entries.to_string(),
                "--stlb-entries".into(), "16".into(), "--max-insns".into(), "3000000000".into(),
                "--uart-input".into(), r.join("tests/tlb.in").to_string_lossy().into(),
            ];
            let (code, out, err) = run_full(&args);
            if code != 0 { println!("{}", out); fatal(&format!("{} run failed", label)); }
            let line = err.lines().find(|l| l.starts_with("[rvsim-tlb]")).unwrap_or("");
            let f = |k: &str| -> u64 { line.split_whitespace().find_map(|t| t.strip_prefix(&format!("{}=", k))).and_then(|v| v.parse().ok()).unwrap_or(0) };
            let (hits, sup, walks) = (f("hits"), f("sup_hits"), f("walks"));
            let miss = walks as f64 / (hits + sup + walks).max(1) as f64;
            println!("  {:<22} {:>8} {:>12} {:>12} {:>10} {:>9.3}%", label, entries, hits, sup, walks, miss * 100.0);
            rows.push(vec![
                ("kernel".into(), label.into()), ("entries".into(), entries.to_string()), ("hits".into(), hits.to_string()),
                ("sup_hits".into(), sup.to_string()), ("walks".into(), walks.to_string()), ("miss_rate".into(), format!("{:.6}", miss)),
            ]);
        }
    }
    std::fs::create_dir_all(r.join("results")).unwrap();
    let refs: Vec<&Vec<(String, String)>> = rows.iter().collect();
    write_csv(&r.join("results/tlb_sweep.csv"), &refs);
    println!("\nCSV: results/tlb_sweep.csv");
    plot_results();
    0
}

/// Read a `metric,value` summary CSV written by `rvsim --perf-out`.
fn read_summary(path: &Path) -> std::collections::HashMap<String, String> {
    let mut m = std::collections::HashMap::new();
    if let Ok(s) = std::fs::read_to_string(path) {
        for l in s.lines().skip(1) {
            if let Some((k, v)) = l.split_once(',') {
                m.insert(k.to_string(), v.to_string());
            }
        }
    }
    m
}

const PRED_NAMES: [&str; 6] = ["static-not-taken", "static-btfn", "bimodal-1024", "bimodal-4096", "gshare-10b", "gshare-14b"];

/// Run every workload through the formula model AND the cycle-by-cycle pipeline simulator.
/// Returns (workload, predictor, formula CPI, cycle CPI, instructions).
fn run_pipeline_validation(only: Option<&[&str]>) -> Vec<(String, String, f64, f64, u64)> {
    let r = root();
    let dir = r.join("results");
    std::fs::create_dir_all(&dir).unwrap();
    let mut jobs: Vec<(String, Vec<String>)> = Vec::new();
    let mut benches: Vec<PathBuf> = std::fs::read_dir(r.join("build/bench")).unwrap().filter_map(|e| e.ok().map(|e| e.path())).collect();
    benches.sort();
    for b in &benches {
        jobs.push((b.file_stem().unwrap().to_string_lossy().to_string(), vec![b.to_string_lossy().into()]));
    }
    let mut kargs = kernel_args();
    kargs.extend(["--snapshot", "--uart-input"].map(String::from));
    kargs.push(r.join("tests/kernel_bench.in").to_string_lossy().into());
    jobs.push(("kernel_ktest".into(), kargs));
    let mut rows = Vec::new();
    for (name, mut args) in jobs {
        if let Some(o) = only {
            if !o.contains(&name.as_str()) { continue; }
        }
        let prefix = dir.join(format!("pipe_{}", name));
        args.extend(["--cycle-model", "--quiet", "--max-insns", "2000000000", "--perf-out"].map(String::from));
        args.push(prefix.to_string_lossy().into());
        let (code, out, _) = run_full(&args);
        if code != 0 && !name.starts_with("kernel") { println!("{}", out); fatal(&format!("{} failed", name)); }
        let s = read_summary(&dir.join(format!("pipe_{}_summary.csv", name)));
        let instrs: u64 = s.get("instructions").and_then(|v| v.parse().ok()).unwrap_or(0);
        for p in PRED_NAMES {
            let f: f64 = s.get(&format!("cpi_{}", p)).and_then(|v| v.parse().ok()).unwrap_or(0.0);
            let c: f64 = s.get(&format!("cyclecpi_{}", p)).and_then(|v| v.parse().ok()).unwrap_or(0.0);
            rows.push((name.clone(), p.to_string(), f, c, instrs));
        }
    }
    rows
}

fn cmd_pipebench() -> i32 {
    build_all();
    println!("\n=== formula (analytic) CPI vs cycle-by-cycle 5-stage pipeline simulation ===");
    let rows = run_pipeline_validation(None);
    println!("  {:<14} {:>12} {:>10} {:>16} {:>9}   (predictor: gshare-14b)", "workload", "instructions", "formula", "cycle-accurate", "diff");
    let mut maxdiff: f64 = 0.0;
    for (w, p, f, c, n) in &rows {
        let d = 100.0 * (f - c) / c.max(1e-9);
        maxdiff = maxdiff.max(d.abs());
        if p == "gshare-14b" {
            println!("  {:<14} {:>12} {:>10.4} {:>16.4} {:>+8.2}%", w, n, f, c, d);
        }
    }
    println!("\n  largest difference over all {} workload x predictor combinations: {:.2}%", rows.len(), maxdiff);
    let mut csv = String::from("workload,predictor,instructions,formula_cpi,cycle_cpi,diff_pct\n");
    for (w, p, f, c, n) in &rows {
        csv += &format!("{},{},{},{:.4},{:.4},{:.3}\n", w, p, n, f, c, 100.0 * (f - c) / c.max(1e-9));
    }
    std::fs::write(root().join("results/pipeline_validation.csv"), csv).unwrap();
    println!("\nCSV: results/pipeline_validation.csv");
    plot_results();
    0
}

/// Compare copy-on-write fork against eager copying on the same workload.
fn cmd_cowbench() -> i32 {
    build_all();
    println!("\n==> building kernel variant with eager (copy-everything) fork");
    build_kernel_variant(&["-DCOW_FORK=0"], "kernel_eager", "kernel_eager.elf");
    let r = root();
    let mut rows = Vec::new();
    for (label, elf) in [("copy-on-write", "kernel.elf"), ("eager copy", "kernel_eager.elf")] {
        let args: Vec<String> = vec![
            r.join("build").join(elf).to_string_lossy().into(), "--disk".into(), r.join("build/fs.img").to_string_lossy().into(),
            "--snapshot".into(), "--perf".into(), "--max-insns".into(), "3000000000".into(),
            "--uart-input".into(), r.join("tests/forkbench.in").to_string_lossy().into(),
        ];
        let (code, out) = run_capture(&args);
        if code != 0 { println!("{}", out); fatal(&format!("{} run failed", label)); }
        let num = |key: &str| -> f64 {
            out.lines().find(|l| l.contains(key)).and_then(|l| l.split(':').nth(1)).map(|s| s.trim().split_whitespace().next().unwrap_or("0").parse().unwrap_or(0.0)).unwrap_or(0.0)
        };
        let instrs = num("Instructions retired");
        let cpi = out.lines().find(|l| l.starts_with("gshare-14b")).and_then(|l| l.split_whitespace().last()).and_then(|s| s.parse::<f64>().ok()).unwrap_or(1.0);
        let pages = out.lines().find(|l| l.contains("pages consumed")).and_then(|l| l.rsplit(':').next()).map(|s| s.trim().to_string()).unwrap_or("?".into());
        let ticks = out.lines().find(|l| l.contains("forks done in")).and_then(|l| l.split_whitespace().rev().nth(1)).map(|s| s.to_string()).unwrap_or("?".into());
        rows.push((label, instrs, instrs * cpi, pages, ticks));
    }
    let mut report = String::from("Fork benchmark: 20 forks of a ~2 MiB process; each child writes one page.\n\n");
    report += &format!("{:<15} {:>16} {:>18} {:>22} {:>12}\n", "fork strategy", "instructions", "modelled cycles", "pages used per fork", "ticks");
    for (l, i, c, p, t) in &rows {
        report += &format!("{:<15} {:>16.0} {:>18.0} {:>22} {:>12}\n", l, i, c, p, t);
    }
    let boot = rows[0].1.min(rows[1].1);
    let _ = boot;
    report += &format!("\nWhole-run speedup (instructions): {:.2}x   (modelled cycles): {:.2}x\n", rows[1].1 / rows[0].1, rows[1].2 / rows[0].2);
    report += "(Both runs include the same boot and shell start-up cost, so the fork-only speedup is larger.)\n";
    println!("\n{}", report);
    std::fs::create_dir_all(r.join("results")).unwrap();
    std::fs::write(r.join("results/cow_fork.txt"), &report).unwrap();
    0
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("help");
    let rest: Vec<String> = args.iter().skip(1).cloned().collect();
    let code = match cmd {
        "build" => { build_all(); 0 }
        "run" => cmd_run(&rest),
        "selftest" => cmd_selftest(),
        "test" => cmd_test(),
        "bench" => cmd_bench(),
        "cowbench" => cmd_cowbench(),
        "vmbench" => cmd_vmbench(),
        "pipebench" => cmd_pipebench(),
        "tlbbench" => cmd_tlbbench(),
        "clean" => { let _ = std::fs::remove_dir_all(root().join("build")); println!("removed build/"); 0 }
        _ => {
            println!("usage: xtask <build | run [rvsim flags] | selftest | test | bench | cowbench | vmbench | tlbbench | pipebench | clean>");
            if cmd == "help" { 0 } else { 2 }
        }
    };
    exit(code);
}
