//! Minimal ELF64 little-endian loader (PT_LOAD segments + `tohost` symbol).

use crate::bus::Bus;

pub struct Loaded {
    pub entry: u64,
    pub tohost: Option<u64>,
}

fn rd16(d: &[u8], o: usize) -> u64 {
    u16::from_le_bytes([d[o], d[o + 1]]) as u64
}
fn rd32(d: &[u8], o: usize) -> u64 {
    u32::from_le_bytes(d[o..o + 4].try_into().unwrap()) as u64
}
fn rd64(d: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(d[o..o + 8].try_into().unwrap())
}

pub fn load(d: &[u8], bus: &mut Bus) -> Result<Loaded, String> {
    if d.len() < 64 || &d[0..4] != b"\x7fELF" {
        return Err("not an ELF file".into());
    }
    if d[4] != 2 || d[5] != 1 {
        return Err("only ELF64 little-endian is supported".into());
    }
    if rd16(d, 18) != 243 {
        return Err("not a RISC-V ELF (e_machine != 243)".into());
    }
    let entry = rd64(d, 24);
    let phoff = rd64(d, 32) as usize;
    let shoff = rd64(d, 40) as usize;
    let phentsize = rd16(d, 54) as usize;
    let phnum = rd16(d, 56) as usize;
    let shentsize = rd16(d, 58) as usize;
    let shnum = rd16(d, 60) as usize;

    for i in 0..phnum {
        let ph = phoff + i * phentsize;
        if ph + 56 > d.len() {
            return Err("truncated program header".into());
        }
        if rd32(d, ph) != 1 {
            continue; // not PT_LOAD
        }
        let off = rd64(d, ph + 8) as usize;
        let paddr = rd64(d, ph + 24);
        let filesz = rd64(d, ph + 32) as usize;
        let memsz = rd64(d, ph + 40) as usize;
        if off + filesz > d.len() {
            return Err("segment extends past end of file".into());
        }
        if !bus.write_bytes(paddr, &d[off..off + filesz]) || !bus.zero_bytes(paddr + filesz as u64, memsz - filesz) {
            return Err(format!("segment at {:#x} (size {:#x}) does not fit in DRAM", paddr, memsz));
        }
    }

    // Look for the `tohost` symbol (used by riscv-tests to report pass/fail).
    let mut tohost = None;
    if shoff != 0 {
        for i in 0..shnum {
            let sh = shoff + i * shentsize;
            if sh + 64 > d.len() {
                break;
            }
            if rd32(d, sh + 4) != 2 {
                continue; // SHT_SYMTAB
            }
            let symoff = rd64(d, sh + 24) as usize;
            let symsize = rd64(d, sh + 32) as usize;
            let link = rd32(d, sh + 40) as usize;
            let strsh = shoff + link * shentsize;
            let stroff = rd64(d, strsh + 24) as usize;
            let mut s = symoff;
            while s + 24 <= symoff + symsize && s + 24 <= d.len() {
                let name = rd32(d, s) as usize;
                let value = rd64(d, s + 8);
                let start = stroff + name;
                if start < d.len() && d[start..].starts_with(b"tohost\0") {
                    tohost = Some(value);
                }
                s += 24;
            }
        }
    }
    Ok(Loaded { entry, tohost })
}
