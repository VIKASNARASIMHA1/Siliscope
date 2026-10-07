//! Offline page-replacement simulators, including Belady's optimal (OPT) algorithm.
//!
//! Every function takes the reference string (page numbers) and the number of frames and
//! returns the total number of page faults (compulsory first-touch faults included).
//! `fifo`, `clock`, `aging` and `random` mirror the kernel's implementation in
//! kernel/swap.c step for step, so the numbers can be compared with what the kernel measured.

use std::collections::VecDeque;

pub fn distinct(refs: &[u32]) -> usize {
    let mut v: Vec<u32> = refs.to_vec();
    v.sort_unstable();
    v.dedup();
    v.len()
}

/// Belady's OPT: on a fault with all frames full, evict the page whose next use is farthest in
/// the future (or never). Needs the whole future, so it is only a yardstick, not implementable online.
pub fn opt(refs: &[u32], cap: usize) -> usize {
    let n = refs.len();
    // next[i] = index of the next occurrence of refs[i] after i (n = never)
    let mut next = vec![n; n];
    let mut last: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    for i in (0..n).rev() {
        next[i] = *last.get(&refs[i]).unwrap_or(&n);
        last.insert(refs[i], i);
    }
    let mut res: Vec<(u32, usize)> = Vec::new(); // (page, index of its next use)
    let mut faults = 0;
    for i in 0..n {
        if let Some(e) = res.iter_mut().find(|e| e.0 == refs[i]) {
            e.1 = next[i];
            continue;
        }
        faults += 1;
        if res.len() >= cap {
            let v = (0..res.len()).max_by_key(|&k| res[k].1).unwrap();
            res.swap_remove(v);
        }
        res.push((refs[i], next[i]));
    }
    faults
}

pub fn fifo(refs: &[u32], cap: usize) -> usize {
    let mut q: VecDeque<u32> = VecDeque::new();
    let mut faults = 0;
    for &p in refs {
        if q.contains(&p) {
            continue;
        }
        faults += 1;
        if q.len() >= cap {
            q.pop_front();
        }
        q.push_back(p);
    }
    faults
}

/// Exact LRU (the ideal that the kernel's "aging" policy approximates).
pub fn lru(refs: &[u32], cap: usize) -> usize {
    let mut res: Vec<(u32, usize)> = Vec::new(); // (page, last use)
    let mut faults = 0;
    for (t, &p) in refs.iter().enumerate() {
        if let Some(e) = res.iter_mut().find(|e| e.0 == p) {
            e.1 = t;
            continue;
        }
        faults += 1;
        if res.len() >= cap {
            let v = (0..res.len()).min_by_key(|&k| res[k].1).unwrap();
            res.remove(v);
        }
        res.push((p, t));
    }
    faults
}

/// Second-chance / Clock exactly as kernel/swap.c implements it: resident pages kept in arrival
/// order, a rotating hand, reference bit set on every access (and on load).
pub fn clock(refs: &[u32], cap: usize) -> usize {
    let mut res: Vec<(u32, bool)> = Vec::new();
    let mut hand = 0usize;
    let mut faults = 0;
    for &p in refs {
        if let Some(e) = res.iter_mut().find(|e| e.0 == p) {
            e.1 = true;
            continue;
        }
        faults += 1;
        if res.len() >= cap {
            loop {
                if hand >= res.len() {
                    hand = 0;
                }
                if res[hand].1 {
                    res[hand].1 = false;
                    hand = (hand + 1) % res.len();
                } else {
                    break;
                }
            }
            let victim = hand;
            hand = (hand + 1) % res.len();
            res.remove(victim);
            if hand > victim {
                hand -= 1;
            }
            if hand >= res.len() {
                hand = 0;
            }
        }
        res.push((p, true));
    }
    faults
}

/// LRU-by-aging exactly as in the kernel: at each eviction every page's 8-bit age is shifted right
/// with its reference bit entering at the top, the bits are cleared, and the smallest age loses.
pub fn aging(refs: &[u32], cap: usize) -> usize {
    struct E { page: u32, seq: u64, age: u8, r: bool }
    let mut res: Vec<E> = Vec::new();
    let mut seq = 0u64;
    let mut faults = 0;
    for &p in refs {
        if let Some(e) = res.iter_mut().find(|e| e.page == p) {
            e.r = true;
            continue;
        }
        faults += 1;
        if res.len() >= cap {
            for e in res.iter_mut() {
                e.age = (e.age >> 1) | if e.r { 0x80 } else { 0 };
                e.r = false;
            }
            let v = (0..res.len()).min_by_key(|&k| (res[k].age, res[k].seq)).unwrap();
            res.remove(v);
        }
        seq += 1;
        res.push(E { page: p, seq, age: 0, r: true });
    }
    faults
}

/// Random replacement with the kernel's xorshift32 generator. `state` carries over between runs,
/// just as the kernel's generator does.
pub fn random(refs: &[u32], cap: usize, state: &mut u32) -> usize {
    let mut res: Vec<u32> = Vec::new();
    let mut faults = 0;
    for &p in refs {
        if res.contains(&p) {
            continue;
        }
        faults += 1;
        if res.len() >= cap {
            *state ^= *state << 13;
            *state ^= *state >> 17;
            *state ^= *state << 5;
            let v = (*state as usize) % res.len();
            res.remove(v);
        }
        res.push(p);
    }
    faults
}

pub const KERNEL_RNG_SEED: u32 = 2463534242;

#[cfg(test)]
mod tests {
    use super::*;

    // The textbook string from Belady's 1969 anomaly paper.
    const BELADY: [u32; 12] = [1, 2, 3, 4, 1, 2, 5, 1, 2, 3, 4, 5];

    #[test]
    fn textbook_fifo_numbers_and_beladys_anomaly() {
        assert_eq!(fifo(&BELADY, 3), 9);
        assert_eq!(fifo(&BELADY, 4), 10); // more frames, MORE faults
    }

    #[test]
    fn textbook_lru_numbers() {
        assert_eq!(lru(&BELADY, 3), 10);
        assert_eq!(lru(&BELADY, 4), 8);
    }

    #[test]
    fn textbook_optimal_numbers() {
        assert_eq!(opt(&BELADY, 3), 7);
        assert_eq!(opt(&BELADY, 4), 6);
    }

    #[test]
    fn opt_is_a_lower_bound_for_every_policy() {
        let mut s = 12345u32;
        let refs: Vec<u32> = (0..3000)
            .map(|_| { s = s.wrapping_mul(1103515245).wrapping_add(12345); (s >> 16) % 40 })
            .collect();
        for cap in [4, 8, 16, 24, 32] {
            let o = opt(&refs, cap);
            let mut rs = KERNEL_RNG_SEED;
            assert!(o <= fifo(&refs, cap) && o <= lru(&refs, cap) && o <= clock(&refs, cap));
            assert!(o <= aging(&refs, cap) && o <= random(&refs, cap, &mut rs));
        }
    }

    #[test]
    fn everything_fits_means_only_compulsory_faults() {
        let refs: Vec<u32> = (0..500).map(|i| i % 10).collect();
        let mut rs = KERNEL_RNG_SEED;
        for f in [opt(&refs, 10), fifo(&refs, 10), lru(&refs, 10), clock(&refs, 10), aging(&refs, 10), random(&refs, 10, &mut rs)] {
            assert_eq!(f, distinct(&refs));
        }
    }

    #[test]
    fn cyclic_scan_larger_than_memory_is_lrus_worst_case() {
        let refs: Vec<u32> = (0..300).map(|i| i % 5).collect();
        assert_eq!(lru(&refs, 4), 300);            // every access misses
        assert!(opt(&refs, 4) < 120);              // OPT keeps most of the loop resident
    }
}
