//! virtio-blk over MMIO, *legacy* (version 1) interface -- the same one QEMU's
//! `virt` machine exposes and xv6 drives. Requests are completed synchronously
//! at QueueNotify time; the guest then sees the interrupt.

use crate::bus::DRAM_BASE;

const QUEUE_NUM_MAX: u32 = 8;
const SECTOR: usize = 512;

pub struct VirtioBlk {
    pub disk: Vec<u8>,
    path: Option<String>,
    snapshot: bool,
    dirty: bool,
    status: u32,
    guest_page_size: u32,
    queue_sel: u32,
    queue_num: u32,
    queue_align: u32,
    queue_pfn: u32,
    last_avail: u16,
    int_status: u32,
    pub reads: u64,
    pub writes: u64,
}

fn rd16(m: &[u8], a: u64) -> Option<u16> {
    let o = a.checked_sub(DRAM_BASE)? as usize;
    Some(u16::from_le_bytes(m.get(o..o + 2)?.try_into().ok()?))
}
fn rd32(m: &[u8], a: u64) -> Option<u32> {
    let o = a.checked_sub(DRAM_BASE)? as usize;
    Some(u32::from_le_bytes(m.get(o..o + 4)?.try_into().ok()?))
}
fn rd64(m: &[u8], a: u64) -> Option<u64> {
    let o = a.checked_sub(DRAM_BASE)? as usize;
    Some(u64::from_le_bytes(m.get(o..o + 8)?.try_into().ok()?))
}
fn wr16(m: &mut [u8], a: u64, v: u16) -> Option<()> {
    let o = a.checked_sub(DRAM_BASE)? as usize;
    m.get_mut(o..o + 2)?.copy_from_slice(&v.to_le_bytes());
    Some(())
}
fn wr32(m: &mut [u8], a: u64, v: u32) -> Option<()> {
    let o = a.checked_sub(DRAM_BASE)? as usize;
    m.get_mut(o..o + 4)?.copy_from_slice(&v.to_le_bytes());
    Some(())
}

impl VirtioBlk {
    pub fn new() -> Self {
        VirtioBlk {
            disk: Vec::new(), path: None, snapshot: false, dirty: false, status: 0,
            guest_page_size: 4096, queue_sel: 0, queue_num: 0, queue_align: 0,
            queue_pfn: 0, last_avail: 0, int_status: 0, reads: 0, writes: 0,
        }
    }

    pub fn attach(&mut self, path: &str, snapshot: bool) -> std::io::Result<()> {
        self.disk = std::fs::read(path)?;
        let pad = (SECTOR - self.disk.len() % SECTOR) % SECTOR;
        self.disk.extend(std::iter::repeat(0).take(pad));
        self.path = Some(path.to_string());
        self.snapshot = snapshot;
        Ok(())
    }

    pub fn save(&self) {
        if self.dirty && !self.snapshot {
            if let Some(p) = &self.path {
                if let Err(e) = std::fs::write(p, &self.disk) {
                    eprintln!("[rvsim] warning: could not write disk image back: {}", e);
                }
            }
        }
    }

    pub fn irq(&self) -> bool {
        self.int_status != 0
    }

    pub fn load(&self, off: u64, _size: usize) -> Option<u64> {
        Some(match off {
            0x000 => 0x7472_6976,
            0x004 => 1,
            0x008 => 2,
            0x00c => 0x554d_4551,
            0x010 => 0,
            0x034 => QUEUE_NUM_MAX as u64,
            0x040 => self.queue_pfn as u64,
            0x060 => self.int_status as u64,
            0x070 => self.status as u64,
            0x100 => ((self.disk.len() / SECTOR) as u64) & 0xffff_ffff,
            0x104 => ((self.disk.len() / SECTOR) as u64) >> 32,
            _ => 0,
        })
    }

    pub fn store(&mut self, off: u64, _size: usize, val: u64, dram: &mut [u8]) -> bool {
        let v = val as u32;
        match off {
            0x028 => self.guest_page_size = v,
            0x030 => self.queue_sel = v,
            0x038 => self.queue_num = v.min(QUEUE_NUM_MAX),
            0x03c => self.queue_align = v,
            0x040 => {
                self.queue_pfn = v;
                self.last_avail = 0;
            }
            0x050 => self.process_queue(dram),
            0x064 => self.int_status &= !v,
            0x070 => {
                self.status = v;
                if v == 0 {
                    self.queue_pfn = 0;
                    self.int_status = 0;
                    self.last_avail = 0;
                }
            }
            _ => {}
        }
        true
    }

    fn process_queue(&mut self, dram: &mut [u8]) {
        if self.queue_pfn == 0 || self.queue_num == 0 {
            return;
        }
        let num = self.queue_num as u64;
        let page = self.guest_page_size.max(1) as u64;
        let align = if self.queue_align != 0 { self.queue_align as u64 } else { page };
        let desc = self.queue_pfn as u64 * page;
        let avail = desc + 16 * num;
        let used = (avail + 6 + 2 * num + align - 1) / align * align;
        let _ = self.process_inner(dram, desc, avail, used, num);
    }

    fn process_inner(&mut self, dram: &mut [u8], desc: u64, avail: u64, used: u64, num: u64) -> Option<()> {
        loop {
            let avail_idx = rd16(dram, avail + 2)?;
            if avail_idx == self.last_avail {
                break;
            }
            let head = rd16(dram, avail + 4 + 2 * (self.last_avail as u64 % num))? as u64;
            // Walk the descriptor chain.
            let mut chain: Vec<(u64, u32, u16)> = Vec::new();
            let mut idx = head;
            loop {
                let d = desc + 16 * idx;
                let addr = rd64(dram, d)?;
                let len = rd32(dram, d + 8)?;
                let flags = rd16(dram, d + 12)?;
                let next = rd16(dram, d + 14)? as u64;
                chain.push((addr, len, flags));
                if flags & 1 == 0 || chain.len() > 16 {
                    break;
                }
                idx = next;
            }
            if chain.len() < 2 {
                return None;
            }
            let hdr = chain[0].0;
            let typ = rd32(dram, hdr)?;
            let sector = rd64(dram, hdr + 8)?;
            let mut off = (sector as usize).checked_mul(SECTOR)?;
            let mut total = 0u32;
            let mut status = 0u8;
            for &(addr, len, _) in &chain[1..chain.len() - 1] {
                let len = len as usize;
                let m = (addr.checked_sub(DRAM_BASE)? as usize, len);
                if off + len > self.disk.len() || m.0 + len > dram.len() {
                    status = 1;
                    break;
                }
                if typ == 0 {
                    dram[m.0..m.0 + len].copy_from_slice(&self.disk[off..off + len]);
                    self.reads += 1;
                } else if typ == 1 {
                    self.disk[off..off + len].copy_from_slice(&dram[m.0..m.0 + len]);
                    self.dirty = true;
                    self.writes += 1;
                } else {
                    status = 2;
                    break;
                }
                off += len;
                total += len as u32;
            }
            // status byte
            let st = chain[chain.len() - 1].0;
            dram[st.checked_sub(DRAM_BASE)? as usize] = status;
            // used ring
            let uidx = rd16(dram, used + 2)?;
            let slot = used + 4 + 8 * (uidx as u64 % num);
            wr32(dram, slot, head as u32)?;
            wr32(dram, slot + 4, total + 1)?;
            wr16(dram, used + 2, uidx.wrapping_add(1))?;
            self.last_avail = self.last_avail.wrapping_add(1);
            self.int_status |= 1;
        }
        Some(())
    }
}
