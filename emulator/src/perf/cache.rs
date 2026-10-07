//! Set-associative, write-back, write-allocate cache with true LRU.

use super::PerfConfig;

pub struct Cache {
    sets: usize,
    ways: usize,
    line_shift: u32,
    tags: Vec<u64>,   // tag+1 (0 = invalid)
    stamp: Vec<u64>,  // LRU timestamp
    dirty: Vec<bool>,
    clock: u64,
    pub accesses: u64,
    pub misses: u64,
    pub writebacks: u64,
}

pub struct Outcome {
    pub hit: bool,
    pub writeback: Option<u64>,
}

impl Cache {
    pub fn new(size: usize, ways: usize, line: usize) -> Self {
        let lines = (size / line).max(ways);
        let sets = (lines / ways).max(1);
        Cache {
            sets, ways, line_shift: line.trailing_zeros(),
            tags: vec![0; sets * ways], stamp: vec![0; sets * ways], dirty: vec![false; sets * ways],
            clock: 0, accesses: 0, misses: 0, writebacks: 0,
        }
    }

    pub fn miss_rate(&self) -> f64 {
        if self.accesses == 0 { 0.0 } else { self.misses as f64 / self.accesses as f64 }
    }

    pub fn access(&mut self, addr: u64, write: bool) -> Outcome {
        self.accesses += 1;
        self.clock += 1;
        let line = addr >> self.line_shift;
        let set = (line as usize) % self.sets;
        let tag = line / self.sets as u64 + 1;
        let base = set * self.ways;
        let mut victim = base;
        let mut oldest = u64::MAX;
        for w in 0..self.ways {
            let i = base + w;
            if self.tags[i] == tag {
                self.stamp[i] = self.clock;
                if write {
                    self.dirty[i] = true;
                }
                return Outcome { hit: true, writeback: None };
            }
            let s = if self.tags[i] == 0 { 0 } else { self.stamp[i] };
            if s < oldest {
                oldest = s;
                victim = i;
            }
        }
        self.misses += 1;
        let mut wb = None;
        if self.tags[victim] != 0 && self.dirty[victim] {
            self.writebacks += 1;
            let old_line = (self.tags[victim] - 1) * self.sets as u64 + set as u64;
            wb = Some(old_line << self.line_shift);
        }
        self.tags[victim] = tag;
        self.stamp[victim] = self.clock;
        self.dirty[victim] = write;
        Outcome { hit: false, writeback: wb }
    }
}

pub struct Hierarchy {
    pub l1i: Cache,
    pub l1d: Cache,
    pub l2: Cache,
    l2_lat: u64,
    mem_lat: u64,
}

impl Hierarchy {
    pub fn new(c: &PerfConfig) -> Self {
        Hierarchy {
            l1i: Cache::new(c.l1_kb * 1024, c.l1_ways, c.line),
            l1d: Cache::new(c.l1_kb * 1024, c.l1_ways, c.line),
            l2: Cache::new(c.l2_kb * 1024, c.l2_ways, c.line),
            l2_lat: c.l2_lat,
            mem_lat: c.mem_lat,
        }
    }

    fn below(&mut self, addr: u64, write: bool) -> u64 {
        let o = self.l2.access(addr, write);
        if o.hit { self.l2_lat } else { self.l2_lat + self.mem_lat }
    }

    /// Returns extra stall cycles (an L1 hit is hidden by the pipeline).
    pub fn fetch(&mut self, pa: u64) -> u64 {
        let o = self.l1i.access(pa, false);
        if o.hit { 0 } else { self.below(pa, false) }
    }

    pub fn data(&mut self, pa: u64, write: bool) -> u64 {
        let o = self.l1d.access(pa, write);
        if let Some(wb) = o.writeback {
            self.l2.access(wb, true);
        }
        if o.hit { 0 } else { self.below(pa, false) }
    }

    /// Average memory access time = L1 hit + L1 miss rate * (L2 hit + L2 miss rate * memory)
    pub fn amat(&self, instr: bool) -> f64 {
        let l1 = if instr { &self.l1i } else { &self.l1d };
        1.0 + l1.miss_rate() * (self.l2_lat as f64 + self.l2.miss_rate() * self.mem_lat as f64)
    }
}
