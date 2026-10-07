//! rvsim -- RISC-V RV64IMAC full-system emulator with a microarchitecture analyzer.

mod bus;
mod cpu;
mod csr;
mod devices;
mod elf;
mod mmu;
mod perf;
mod rvc;
mod trap;

use bus::Bus;
use cpu::Cpu;
use devices::uart::Uart;
use perf::{Perf, PerfConfig};
use std::process::exit;
use std::time::Instant;

const USAGE: &str = "\
rvsim - RISC-V RV64IMAC full-system emulator

USAGE: rvsim <program.elf> [options]

  --disk <img>         attach a virtio-blk disk image
  --snapshot           do not write disk changes back to the image file
  --uart-input <file>  feed the UART receiver from a file (deterministic, no stdin)
  --max-insns <n>      stop after n instructions (exit code 124)
  --ram-mb <n>         DRAM size in MiB (default 128)
  --perf               enable the microarchitecture analyzer and print a report
  --sweep              also sweep cache sizes/ways/lines and predictor sizes (implies --perf)
  --cycle-model        also run a cycle-by-cycle 5-stage pipeline simulation and compare it with the formula model (implies --perf)
  --perf-out <prefix>  write CSV files <prefix>_summary.csv (and sweep CSVs)
  --l1-kb <n> --l1-ways <n> --line <bytes> --l2-kb <n> --l2-ways <n>   analyzer cache geometry
  --trace <file>       write a text instruction trace (pc inst [L/S paddr])
  --tlb-entries <n>    4 KiB-page TLB entries, direct-mapped (default 256)
  --stlb-entries <n>   superpage (2 MiB/1 GiB) TLB entries, fully associative (default 16)
  --tlb-stats          print one machine-readable TLB statistics line to stderr at exit
  --quiet              suppress the end-of-run summary line
";

fn die(msg: &str) -> ! {
    eprintln!("rvsim: {}", msg);
    exit(2);
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut prog: Option<String> = None;
    let (mut disk, mut uart_in, mut perf_out, mut trace) = (None, None, None, None);
    let mut max_insns = 0u64;
    let mut ram_mb = 128usize;
    let (mut do_perf, mut snapshot, mut quiet, mut tlb_stats) = (false, false, false, false);
    let (mut tlb_entries, mut stlb_entries) = (mmu::DEFAULT_TLB_ENTRIES, mmu::DEFAULT_STLB_ENTRIES);
    let mut pc = PerfConfig::default();
    while let Some(a) = args.next() {
        let mut val = |name: &str| args.next().unwrap_or_else(|| die(&format!("{} needs a value", name)));
        let num = |s: String, name: &str| -> usize { s.parse().unwrap_or_else(|_| die(&format!("bad number for {}", name))) };
        match a.as_str() {
            "-h" | "--help" => {
                print!("{}", USAGE);
                return;
            }
            "--disk" => disk = Some(val("--disk")),
            "--snapshot" => snapshot = true,
            "--uart-input" => uart_in = Some(val("--uart-input")),
            "--max-insns" => max_insns = num(val("--max-insns"), "--max-insns") as u64,
            "--ram-mb" => ram_mb = num(val("--ram-mb"), "--ram-mb"),
            "--perf" => do_perf = true,
            "--sweep" => {
                do_perf = true;
                pc.sweep = true;
            }
            "--cycle-model" => {
                do_perf = true;
                pc.cycle = true;
            }
            "--perf-out" => perf_out = Some(val("--perf-out")),
            "--l1-kb" => pc.l1_kb = num(val("--l1-kb"), "--l1-kb"),
            "--l1-ways" => pc.l1_ways = num(val("--l1-ways"), "--l1-ways"),
            "--line" => pc.line = num(val("--line"), "--line"),
            "--l2-kb" => pc.l2_kb = num(val("--l2-kb"), "--l2-kb"),
            "--l2-ways" => pc.l2_ways = num(val("--l2-ways"), "--l2-ways"),
            "--trace" => trace = Some(val("--trace")),
            "--tlb-entries" => tlb_entries = num(val("--tlb-entries"), "--tlb-entries"),
            "--stlb-entries" => stlb_entries = num(val("--stlb-entries"), "--stlb-entries"),
            "--tlb-stats" => tlb_stats = true,
            "--quiet" => quiet = true,
            s if s.starts_with('-') => die(&format!("unknown option {}\n\n{}", s, USAGE)),
            _ => prog = Some(a),
        }
    }
    let prog = prog.unwrap_or_else(|| die(&format!("no program given\n\n{}", USAGE)));
    if !pc.line.is_power_of_two() {
        die("--line must be a power of two");
    }
    let image = std::fs::read(&prog).unwrap_or_else(|e| die(&format!("cannot read {}: {}", prog, e)));

    let uart = match &uart_in {
        Some(f) => Uart::with_input(std::fs::read(f).unwrap_or_else(|e| die(&format!("cannot read {}: {}", f, e)))),
        None => Uart::with_stdin(),
    };
    let mut bus = Bus::new(ram_mb << 20, uart);
    let loaded = elf::load(&image, &mut bus).unwrap_or_else(|e| die(&format!("{}: {}", prog, e)));
    bus.tohost = loaded.tohost.unwrap_or(0);
    if let Some(d) = &disk {
        bus.virtio.attach(d, snapshot).unwrap_or_else(|e| die(&format!("cannot read disk {}: {}", d, e)));
    }
    let mut cpu = Cpu::new(bus, loaded.entry);
    cpu.tlb = mmu::Tlb::new(tlb_entries, stlb_entries);
    if do_perf {
        cpu.perf = Some(Box::new(Perf::new(pc)));
    }
    if let Some(t) = &trace {
        let f = std::fs::File::create(t).unwrap_or_else(|e| die(&format!("cannot create {}: {}", t, e)));
        cpu.trace = Some(std::io::BufWriter::new(f));
    }

    let start = Instant::now();
    let mut timed_out = false;
    while cpu.step() {
        if max_insns != 0 && cpu.csr.minstret >= max_insns {
            timed_out = true;
            break;
        }
    }
    let elapsed = start.elapsed().as_secs_f64();

    cpu.bus.virtio.save();
    if let Some(mut t) = cpu.trace.take() {
        use std::io::Write;
        let _ = t.flush();
    }
    if let Some(p) = cpu.perf.as_mut() {
        p.finish();
    }
    let tlb = Some((cpu.tlb.hits + cpu.tlb.sup_hits, cpu.tlb.misses));
    if tlb_stats {
        eprintln!("[rvsim-tlb] entries={} sup_entries={} hits={} sup_hits={} walks={} flushes={}", cpu.tlb.entries, cpu.tlb.sup_entries, cpu.tlb.hits, cpu.tlb.sup_hits, cpu.tlb.misses, cpu.tlb.flushes);
    }
    if let Some(p) = cpu.perf.as_ref() {
        print!("{}", p.report(tlb));
        if let Some(prefix) = &perf_out {
            if let Some(dir) = std::path::Path::new(prefix).parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = p.write_csv(prefix, tlb) {
                eprintln!("rvsim: cannot write CSV: {}", e);
            }
        }
    }
    if !quiet {
        let n = cpu.csr.minstret;
        eprintln!(
            "[rvsim] {} instructions in {:.2}s ({:.1} MIPS){}",
            n, elapsed, n as f64 / elapsed.max(1e-9) / 1e6,
            if timed_out { " -- stopped at --max-insns" } else { "" }
        );
    }
    if timed_out {
        exit(124);
    }
    exit(cpu.bus.exit.unwrap_or(if cpu.idle_exit { 3 } else { 0 }));
}
