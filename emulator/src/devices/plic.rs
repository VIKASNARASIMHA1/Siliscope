//! PLIC with 31 sources and two contexts (0 = hart0 M-mode, 1 = hart0 S-mode).
//! Level-triggered: a source is pending while its line is high and it has not
//! been claimed.

pub struct Plic {
    prio: [u32; 32],
    enable: [u32; 2],
    thresh: [u32; 2],
    claimed: u32,
    lines: u32,
}

impl Plic {
    pub fn new() -> Self {
        Plic { prio: [0; 32], enable: [0; 2], thresh: [0; 2], claimed: 0, lines: 0 }
    }

    pub fn set_line(&mut self, id: u32, level: bool) {
        if level {
            self.lines |= 1 << id;
        } else {
            self.lines &= !(1 << id);
        }
    }

    fn best(&self, ctx: usize) -> u32 {
        let mut best = 0;
        let mut best_prio = 0;
        for id in 1..32u32 {
            let bit = 1u32 << id;
            if self.lines & bit != 0
                && self.claimed & bit == 0
                && self.enable[ctx] & bit != 0
                && self.prio[id as usize] > self.thresh[ctx]
                && self.prio[id as usize] > best_prio
            {
                best = id;
                best_prio = self.prio[id as usize];
            }
        }
        best
    }

    pub fn meip(&self) -> bool {
        self.best(0) != 0
    }
    pub fn seip(&self) -> bool {
        self.best(1) != 0
    }

    pub fn load(&mut self, off: u64, _size: usize) -> Option<u64> {
        match off {
            0x0..=0x7f => Some(self.prio[(off / 4) as usize & 31] as u64),
            0x1000 => Some((self.lines & !self.claimed) as u64),
            0x2000..=0x20ff => {
                let ctx = ((off - 0x2000) / 0x80) as usize;
                if ctx < 2 && (off - 0x2000) % 0x80 == 0 { Some(self.enable[ctx] as u64) } else { Some(0) }
            }
            0x200000..=0x201fff => {
                let ctx = ((off - 0x200000) / 0x1000) as usize;
                match (off - 0x200000) % 0x1000 {
                    0 => Some(self.thresh[ctx] as u64),
                    4 => {
                        let id = self.best(ctx);
                        if id != 0 {
                            self.claimed |= 1 << id;
                        }
                        Some(id as u64)
                    }
                    _ => Some(0),
                }
            }
            _ => Some(0),
        }
    }

    pub fn store(&mut self, off: u64, _size: usize, val: u64) -> bool {
        let v = val as u32;
        match off {
            0x0..=0x7f => self.prio[(off / 4) as usize & 31] = v & 7,
            0x2000..=0x20ff => {
                let ctx = ((off - 0x2000) / 0x80) as usize;
                if ctx < 2 && (off - 0x2000) % 0x80 == 0 {
                    self.enable[ctx] = v & !1;
                }
            }
            0x200000..=0x201fff => {
                let ctx = ((off - 0x200000) / 0x1000) as usize;
                match (off - 0x200000) % 0x1000 {
                    0 => self.thresh[ctx] = v & 7,
                    4 => {
                        if v < 32 {
                            self.claimed &= !(1u32 << v);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        true
    }
}
