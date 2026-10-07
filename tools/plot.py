#!/usr/bin/env python3
"""Turn rvsim --perf-out CSV files into charts.   usage: python tools/plot.py results"""
import csv, glob, os, sys
try:
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
except ImportError:
    sys.exit("matplotlib not installed: pip install matplotlib")

d = sys.argv[1] if len(sys.argv) > 1 else "results"
names = sorted(os.path.basename(p)[:-len("_summary.csv")] for p in glob.glob(os.path.join(d, "*_summary.csv")))
if not names:
    print("(no *_summary.csv files: skipping the CPU benchmark charts)")

def rows(path):
    with open(path) as f:
        return list(csv.DictReader(f))

def summary(n):
    return {r["metric"]: r["value"] for r in rows(os.path.join(d, n + "_summary.csv"))}

S = {n: summary(n) for n in names}
preds = [k[len("accuracy_"):] for k in S[names[0]] if k.startswith("accuracy_")] if names else []

if names:
    # 1. predictor accuracy and CPI per workload
    fig, ax = plt.subplots(1, 2, figsize=(13, 4.5))
    w = 0.8 / len(preds)
    for i, p in enumerate(preds):
        ax[0].bar([x + i * w for x in range(len(names))], [100 * float(S[n]["accuracy_" + p]) for n in names], w, label=p)
        ax[1].bar([x + i * w for x in range(len(names))], [float(S[n]["cpi_" + p]) for n in names], w, label=p)
    for a, t, y in ((ax[0], "Branch prediction accuracy", "% correct"), (ax[1], "Modelled 5-stage CPI", "cycles / instruction")):
        a.set_xticks([x + 0.4 - w / 2 for x in range(len(names))]); a.set_xticklabels(names, rotation=30); a.set_title(t); a.set_ylabel(y)
    ax[0].set_ylim(40, 100.5); ax[0].legend(fontsize=7)
    plt.tight_layout(); plt.savefig(os.path.join(d, "predictors.png"), dpi=130); plt.close()

    # 2. cache miss rate vs size, one line per associativity (64B lines)
    for kind, title in (("dcache", "L1 D-cache"), ("icache", "L1 I-cache")):
        fig, axes = plt.subplots(2, 4, figsize=(16, 7), sharex=True)
        for ax_, n in zip(axes.flat, names):
            p = os.path.join(d, n + "_" + kind + ".csv")
            if not os.path.exists(p): continue
            data = [r for r in rows(p) if r["line"] == "64"]
            for ways in sorted({int(r["ways"]) for r in data}):
                pts = sorted((int(r["size_kb"]), 100 * float(r["miss_rate"])) for r in data if int(r["ways"]) == ways)
                ax_.plot([x for x, _ in pts], [y for _, y in pts], marker="o", label=f"{ways}-way")
            ax_.set_xscale("log", base=2); ax_.set_title(n); ax_.set_xlabel("size (KiB)"); ax_.set_ylabel("miss rate (%)")
        axes.flat[0].legend(fontsize=8)
        for ax_ in list(axes.flat)[len(names):]: ax_.axis("off")
        fig.suptitle(title + ": miss rate vs capacity and associativity (64 B lines)")
        plt.tight_layout(); plt.savefig(os.path.join(d, kind + "_size_assoc.png"), dpi=120); plt.close()

    # 3. line size sweep (16 KiB, 4-way D-cache)
    fig, ax = plt.subplots(figsize=(7, 4.5))
    for n in names:
        p = os.path.join(d, n + "_dcache.csv")
        if not os.path.exists(p): continue
        pts = sorted((int(r["line"]), 100 * float(r["miss_rate"])) for r in rows(p) if r["size_kb"] == "16" and r["ways"] == "4")
        ax.plot([x for x, _ in pts], [y for _, y in pts], marker="o", label=n)
    ax.set_xscale("log", base=2); ax.set_xlabel("line size (bytes)"); ax.set_ylabel("miss rate (%)"); ax.legend(fontsize=7)
    ax.set_title("D-cache miss rate vs line size (16 KiB, 4-way)")
    plt.tight_layout(); plt.savefig(os.path.join(d, "dcache_linesize.png"), dpi=130); plt.close()

    # 4. predictor table size sweep
    fig, axes = plt.subplots(1, 2, figsize=(12, 4.5), sharey=False)
    for ax_, kind in zip(axes, ("bimodal", "gshare")):
        for n in names:
            p = os.path.join(d, n + "_pred.csv")
            if not os.path.exists(p): continue
            pts = sorted((int(r["bits"]), 100 * float(r["accuracy"])) for r in rows(p) if r["predictor"] == kind)
            ax_.plot([x for x, _ in pts], [y for _, y in pts], marker=".", label=n)
        ax_.set_title(kind + ": accuracy vs index bits"); ax_.set_xlabel("index bits (2^n counters)"); ax_.set_ylabel("accuracy (%)")
    axes[0].legend(fontsize=7)
    plt.tight_layout(); plt.savefig(os.path.join(d, "predictor_size.png"), dpi=130); plt.close()

    print("wrote predictors.png, dcache_size_assoc.png, icache_size_assoc.png, dcache_linesize.png, predictor_size.png in", d)


# ---- OS-level experiments (written by `rvsim vmbench` / `rvsim tlbbench`) ----------------
def maybe(name):
    path = os.path.join(d, name)
    return rows(path) if os.path.exists(path) else None

swap = maybe("swap_bench.csv")
if swap:
    pols = []
    for r in swap:
        if r["policy"] not in pols: pols.append(r["policy"])
    pats = []
    for r in swap:
        if r["pattern"] not in pats: pats.append(r["pattern"])
    fig, axes = plt.subplots(1, len(pats), figsize=(5.5 * len(pats), 4.2), sharey=False)
    if len(pats) == 1: axes = [axes]
    for ax_, pat in zip(axes, pats):
        vals = [int(next(r for r in swap if r["policy"] == p_ and r["pattern"] == pat)["major_faults"]) for p_ in pols]
        bars = ax_.bar(pols, vals, color=["#4c72b0", "#55a868", "#c44e52", "#8172b2"][:len(pols)])
        for b_, v_ in zip(bars, vals): ax_.text(b_.get_x() + b_.get_width() / 2, v_, str(v_), ha="center", va="bottom", fontsize=9)
        ax_.set_title(f"access pattern: {pat}"); ax_.set_ylabel("major page faults (swap-ins)  -  lower is better")
    fig.suptitle("Page replacement in the kernel: 64-page heap, 24 resident frames")
    plt.tight_layout(); plt.savefig(os.path.join(d, "swap_policies.png"), dpi=130); plt.close()
    print("wrote swap_policies.png")

sched = maybe("sched_bench.csv")
if sched:
    metrics = [("short_turnaround_x10", 10, "short jobs (arrive late)\nturnaround, ticks"),
               ("interactive_wait_x100", 100, "interactive jobs\nwait for CPU, ticks"),
               ("long_turnaround_x10", 10, "long CPU jobs\nturnaround, ticks")]
    fig, axes = plt.subplots(1, 3, figsize=(12, 3.8))
    for ax_, (key, div, title) in zip(axes, metrics):
        names_ = [r["policy"] for r in sched]
        vals = [int(r[key]) / div for r in sched]
        bars = ax_.bar(names_, vals, color=["#c44e52", "#4c72b0"][:len(names_)])
        for b_, v_ in zip(bars, vals): ax_.text(b_.get_x() + b_.get_width() / 2, v_, f"{v_:.2f}".rstrip("0").rstrip("."), ha="center", va="bottom")
        ax_.set_title(title, fontsize=10)
    fig.suptitle("Round-robin vs MLFQ (lower is better)")
    plt.tight_layout(); plt.savefig(os.path.join(d, "scheduler_rr_vs_mlfq.png"), dpi=130); plt.close()
    print("wrote scheduler_rr_vs_mlfq.png")

lazy = maybe("lazy_bench.csv")
if lazy:
    fig, ax_ = plt.subplots(figsize=(5.5, 4))
    names_ = [r["mode"] for r in lazy]; vals = [int(r["frames_used"]) for r in lazy]
    bars = ax_.bar(names_, vals, color=["#c44e52", "#55a868"][:len(names_)])
    for b_, v_ in zip(bars, vals): ax_.text(b_.get_x() + b_.get_width() / 2, v_, str(v_), ha="center", va="bottom")
    ax_.set_ylabel("4 KiB frames consumed"); ax_.set_title("sbrk(32 MiB), touch 1/16 of the pages")
    plt.tight_layout(); plt.savefig(os.path.join(d, "lazy_sbrk.png"), dpi=130); plt.close()
    print("wrote lazy_sbrk.png")

tlb = maybe("tlb_sweep.csv")
if tlb:
    fig, ax_ = plt.subplots(figsize=(7, 4.5))
    for kern in sorted({r["kernel"] for r in tlb}):
        pts = sorted((int(r["entries"]), 100 * float(r["miss_rate"])) for r in tlb if r["kernel"] == kern)
        ax_.plot([x for x, _ in pts], [y for _, y in pts], marker="o", label=kern)
    ax_.set_xscale("log", base=2); ax_.set_yscale("log")
    ax_.set_xlabel("4 KiB-page TLB entries (direct-mapped)"); ax_.set_ylabel("TLB miss rate (%) - page walks")
    ax_.set_title("TLB miss rate: kernel direct map with 4 KiB pages vs 2 MiB superpages"); ax_.legend()
    plt.tight_layout(); plt.savefig(os.path.join(d, "tlb_sweep.png"), dpi=130); plt.close()
    print("wrote tlb_sweep.png")

pv = maybe("pipeline_validation.csv")
if pv:
    wl = []
    for r in pv:
        if r["workload"] not in wl: wl.append(r["workload"])
    fig, ax_ = plt.subplots(figsize=(9, 4.3))
    g = [r for r in pv if r["predictor"] == "gshare-14b"]
    xs = range(len(wl)); w = 0.38
    f = [float(next(r for r in g if r["workload"] == k)["formula_cpi"]) for k in wl]
    c = [float(next(r for r in g if r["workload"] == k)["cycle_cpi"]) for k in wl]
    ax_.bar([i - w / 2 for i in xs], f, w, label="formula model")
    ax_.bar([i + w / 2 for i in xs], c, w, label="cycle-by-cycle simulation")
    for i, (a, b) in enumerate(zip(f, c)):
        ax_.text(i, max(a, b), f"{100 * (a - b) / b:+.1f}%", ha="center", va="bottom", fontsize=8)
    ax_.set_xticks(list(xs)); ax_.set_xticklabels(wl, rotation=20); ax_.set_ylabel("CPI (gshare-14b)")
    ax_.set_title("Does the formula match a real cycle-by-cycle pipeline?"); ax_.legend()
    plt.tight_layout(); plt.savefig(os.path.join(d, "pipeline_validation.png"), dpi=130); plt.close()
    print("wrote pipeline_validation.png")

oc = maybe("opt_compare.csv")
if oc:
    pats = []
    for r in oc:
        if r["pattern"] not in pats: pats.append(r["pattern"])
    pols = ["FIFO", "CLOCK", "LRU-aging", "RANDOM"]
    fig, axes = plt.subplots(1, len(pats), figsize=(5.8 * len(pats), 4.4))
    if len(pats) == 1: axes = [axes]
    for ax_, pat in zip(axes, pats):
        rows_ = [r for r in oc if r["pattern"] == pat]
        opt_v = int(rows_[0]["opt_major"])
        labels = ["OPT\n(Belady)"] + pols
        vals = [opt_v] + [int(next(r for r in rows_ if r["policy"] == p_)["kernel_major"]) for p_ in pols]
        cols = ["#2ca02c"] + ["#4c72b0", "#55a868", "#c44e52", "#8172b2"]
        bars = ax_.bar(labels, vals, color=cols)
        for b_, v_ in zip(bars, vals):
            lab = str(v_) if v_ == opt_v else f"{v_}\n({v_ / max(opt_v, 1):.2f}x)"
            ax_.text(b_.get_x() + b_.get_width() / 2, v_, lab, ha="center", va="bottom", fontsize=8)
        ax_.set_title(f"access pattern: {pat}"); ax_.set_ylabel("major page faults (lower is better)")
        ax_.set_ylim(0, max(vals) * 1.22)
    fig.suptitle("Kernel replacement policies vs Belady's optimal (24 frames, same reference string)")
    plt.tight_layout(); plt.savefig(os.path.join(d, "opt_compare.png"), dpi=130); plt.close()
    print("wrote opt_compare.png")
