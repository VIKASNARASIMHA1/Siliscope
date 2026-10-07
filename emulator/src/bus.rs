//! Physical address space: DRAM + memory-mapped devices (QEMU `virt`-compatible map).

use crate::devices::{clint::Clint, plic::Plic, uart::Uart, virtio_blk::VirtioBlk};

pub const DRAM_BASE: u64 = 0x8000_0000;
pub const FINISHER_BASE: u64 = 0x0010_0000;
pub const CLINT_BASE: u64 = 0x0200_0000;
pub const CLINT_SIZE: u64 = 0x1_0000;
pub const PLIC_BASE: u64 = 0x0c00_0000;
pub const PLIC_SIZE: u64 = 0x40_0000;
pub const UART_BASE: u64 = 0x1000_0000;
pub const UART_SIZE: u64 = 0x100;
pub const VIRTIO_BASE: u64 = 0x1000_1000;
pub const VIRTIO_SIZE: u64 = 0x1000;

pub const UART_IRQ: u32 = 10;
pub const VIRTIO_IRQ: u32 = 1;

pub struct Bus {
    pub dram: Vec<u8>,
    pub uart: Uart,
    pub clint: Clint,
    pub plic: Plic,
    pub virtio: VirtioBlk,
    /// Set when the guest asks to power off (test finisher or HTIF tohost).
    pub exit: Option<i32>,
    pub tohost: u64,
}

impl Bus {
    pub fn new(dram_bytes: usize, uart: Uart) -> Self {
        Bus {
            dram: vec![0; dram_bytes],
            uart,
            clint: Clint::new(),
            plic: Plic::new(),
            virtio: VirtioBlk::new(),
            exit: None,
            tohost: 0,
        }
    }

    pub fn write_bytes(&mut self, pa: u64, data: &[u8]) -> bool {
        match pa.checked_sub(DRAM_BASE) {
            Some(o) if (o as usize) + data.len() <= self.dram.len() => {
                self.dram[o as usize..o as usize + data.len()].copy_from_slice(data);
                true
            }
            _ => false,
        }
    }

    pub fn zero_bytes(&mut self, pa: u64, n: usize) -> bool {
        match pa.checked_sub(DRAM_BASE) {
            Some(o) if (o as usize) + n <= self.dram.len() => {
                self.dram[o as usize..o as usize + n].fill(0);
                true
            }
            _ => false,
        }
    }

    pub fn poll(&mut self) {
        self.uart.poll();
        self.sync_irq();
    }

    pub fn sync_irq(&mut self) {
        self.plic.set_line(UART_IRQ, self.uart.irq());
        self.plic.set_line(VIRTIO_IRQ, self.virtio.irq());
    }

    #[inline]
    pub fn load(&mut self, pa: u64, size: usize) -> Option<u64> {
        let off = pa.wrapping_sub(DRAM_BASE);
        if off < self.dram.len() as u64 {
            let o = off as usize;
            if o + size > self.dram.len() {
                return None;
            }
            let mut buf = [0u8; 8];
            buf[..size].copy_from_slice(&self.dram[o..o + size]);
            return Some(u64::from_le_bytes(buf));
        }
        self.load_mmio(pa, size)
    }

    #[inline]
    pub fn store(&mut self, pa: u64, size: usize, val: u64) -> bool {
        let off = pa.wrapping_sub(DRAM_BASE);
        if off < self.dram.len() as u64 {
            let o = off as usize;
            if o + size > self.dram.len() {
                return false;
            }
            self.dram[o..o + size].copy_from_slice(&val.to_le_bytes()[..size]);
            if self.tohost != 0 && pa >= self.tohost && pa < self.tohost + 8 {
                self.check_tohost();
            }
            return true;
        }
        self.store_mmio(pa, size, val)
    }

    fn check_tohost(&mut self) {
        let o = (self.tohost - DRAM_BASE) as usize;
        let v = u64::from_le_bytes(self.dram[o..o + 8].try_into().unwrap());
        if v != 0 && v & 1 == 1 {
            self.exit = Some((v >> 1) as i32);
        }
    }

    #[cold]
    fn load_mmio(&mut self, pa: u64, size: usize) -> Option<u64> {
        let r = if (CLINT_BASE..CLINT_BASE + CLINT_SIZE).contains(&pa) {
            self.clint.load(pa - CLINT_BASE, size)
        } else if (PLIC_BASE..PLIC_BASE + PLIC_SIZE).contains(&pa) {
            self.plic.load(pa - PLIC_BASE, size)
        } else if (UART_BASE..UART_BASE + UART_SIZE).contains(&pa) {
            self.uart.load(pa - UART_BASE, size)
        } else if (VIRTIO_BASE..VIRTIO_BASE + VIRTIO_SIZE).contains(&pa) {
            self.virtio.load(pa - VIRTIO_BASE, size)
        } else if (FINISHER_BASE..FINISHER_BASE + 0x1000).contains(&pa) {
            Some(0)
        } else {
            None
        };
        self.sync_irq();
        r
    }

    #[cold]
    fn store_mmio(&mut self, pa: u64, size: usize, val: u64) -> bool {
        let ok = if (CLINT_BASE..CLINT_BASE + CLINT_SIZE).contains(&pa) {
            self.clint.store(pa - CLINT_BASE, size, val)
        } else if (PLIC_BASE..PLIC_BASE + PLIC_SIZE).contains(&pa) {
            self.plic.store(pa - PLIC_BASE, size, val)
        } else if (UART_BASE..UART_BASE + UART_SIZE).contains(&pa) {
            self.uart.store(pa - UART_BASE, size, val)
        } else if (VIRTIO_BASE..VIRTIO_BASE + VIRTIO_SIZE).contains(&pa) {
            self.virtio.store(pa - VIRTIO_BASE, size, val, &mut self.dram)
        } else if (FINISHER_BASE..FINISHER_BASE + 0x1000).contains(&pa) {
            // SiFive test finisher: 0x5555 = pass, (code<<16)|0x3333 = fail(code)
            let v = val as u32;
            match v & 0xffff {
                0x5555 => self.exit = Some(0),
                0x3333 => self.exit = Some(((v >> 16) as i32).max(1)),
                0x7777 => self.exit = Some(0),
                _ => {}
            }
            true
        } else {
            false
        };
        self.sync_irq();
        ok
    }
}
