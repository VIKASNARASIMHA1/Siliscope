//! Builds the guest file-system image. The on-disk layout MUST match kernel/kernel.h:
//!   block 0: unused | block 1: superblock | inode blocks | bitmap blocks | data blocks
//!   BSIZE=1024, 64-byte inodes (12 direct + 1 indirect), 16-byte directory entries.

const BSIZE: usize = 1024;
const FSSIZE: usize = 8192; // blocks (8 MiB)
const NINODES: usize = 200;
const NDIRECT: usize = 12;
const NINDIRECT: usize = BSIZE / 4;
const DINODE: usize = 64;
const IPB: usize = BSIZE / DINODE;
const BPB: usize = BSIZE * 8;
const T_DIR: u16 = 1;
const T_FILE: u16 = 2;
const T_DEV: u16 = 3;
const FSMAGIC: u32 = 0x1020_3040;
/// Blocks reserved after the file system for swap (must match SWAPSTART / SWAPSLOTS in kernel.h:
/// the swap area starts at block FSSIZE and has SWAPSLOTS * 4 blocks).
const SWAP_BLOCKS: usize = 1024 * 4;

struct Fs {
    img: Vec<u8>,
    inodestart: usize,
    bmapstart: usize,
    freeinode: u32,
    freeblock: usize,
}

fn put16(b: &mut [u8], o: usize, v: u16) { b[o..o + 2].copy_from_slice(&v.to_le_bytes()); }
fn put32(b: &mut [u8], o: usize, v: u32) { b[o..o + 4].copy_from_slice(&v.to_le_bytes()); }
fn get32(b: &[u8], o: usize) -> u32 { u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) }

impl Fs {
    fn ioff(&self, inum: u32) -> usize { self.inodestart * BSIZE + inum as usize * DINODE }

    fn ialloc(&mut self, typ: u16, major: u16) -> u32 {
        let inum = self.freeinode;
        self.freeinode += 1;
        assert!((inum as usize) < NINODES, "too many files for the image");
        let o = self.ioff(inum);
        put16(&mut self.img, o, typ);
        put16(&mut self.img, o + 2, major);
        put16(&mut self.img, o + 4, 0);
        put16(&mut self.img, o + 6, 1); // nlink
        inum
    }

    fn alloc_block(&mut self) -> usize {
        let b = self.freeblock;
        self.freeblock += 1;
        assert!(self.freeblock <= FSSIZE, "file system image is full");
        b
    }

    fn iappend(&mut self, inum: u32, data: &[u8]) {
        let o = self.ioff(inum);
        let mut size = get32(&self.img, o + 8) as usize;
        let mut done = 0;
        while done < data.len() {
            let fbn = size / BSIZE;
            assert!(fbn < NDIRECT + NINDIRECT, "file too large for this file system (max {} bytes)", (NDIRECT + NINDIRECT) * BSIZE);
            let blk = if fbn < NDIRECT {
                let a = get32(&self.img, o + 12 + fbn * 4) as usize;
                if a == 0 {
                    let nb = self.alloc_block();
                    put32(&mut self.img, o + 12 + fbn * 4, nb as u32);
                    nb
                } else { a }
            } else {
                let mut ind = get32(&self.img, o + 12 + NDIRECT * 4) as usize;
                if ind == 0 {
                    ind = self.alloc_block();
                    put32(&mut self.img, o + 12 + NDIRECT * 4, ind as u32);
                }
                let eo = ind * BSIZE + (fbn - NDIRECT) * 4;
                let a = get32(&self.img, eo) as usize;
                if a == 0 {
                    let nb = self.alloc_block();
                    put32(&mut self.img, eo, nb as u32);
                    nb
                } else { a }
            };
            let n = (data.len() - done).min((fbn + 1) * BSIZE - size);
            let off = blk * BSIZE + size - fbn * BSIZE;
            self.img[off..off + n].copy_from_slice(&data[done..done + n]);
            done += n;
            size += n;
        }
        put32(&mut self.img, o + 8, size as u32);
    }

    fn dirent(&mut self, dir: u32, inum: u32, name: &str) {
        assert!(name.len() <= 14, "file name `{}` longer than 14 characters", name);
        let mut de = [0u8; 16];
        put16(&mut de, 0, inum as u16);
        de[2..2 + name.len()].copy_from_slice(name.as_bytes());
        self.iappend(dir, &de);
    }
}

pub fn build(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let nbitmap = FSSIZE / BPB + 1;
    let ninodeblocks = NINODES / IPB + 1;
    let nmeta = 2 + ninodeblocks + nbitmap;
    let mut fs = Fs { img: vec![0; FSSIZE * BSIZE], inodestart: 2, bmapstart: 2 + ninodeblocks, freeinode: 1, freeblock: nmeta };

    // superblock
    let sb = BSIZE;
    put32(&mut fs.img, sb, FSMAGIC);
    put32(&mut fs.img, sb + 4, FSSIZE as u32);
    put32(&mut fs.img, sb + 8, (FSSIZE - nmeta) as u32);
    put32(&mut fs.img, sb + 12, NINODES as u32);
    put32(&mut fs.img, sb + 16, 2);
    put32(&mut fs.img, sb + 20, fs.bmapstart as u32);

    let root = fs.ialloc(T_DIR, 0);
    assert_eq!(root, 1);
    fs.dirent(root, root, ".");
    fs.dirent(root, root, "..");

    let console = fs.ialloc(T_DEV, 1); // major 1 = console
    fs.dirent(root, console, "console");

    for (name, data) in files {
        if (data.len()) > (NDIRECT + NINDIRECT) * BSIZE {
            return Err(format!("{} is too large ({} bytes)", name, data.len()));
        }
        let inum = fs.ialloc(T_FILE, 0);
        fs.dirent(root, inum, name);
        fs.iappend(inum, data);
    }
    // round the root directory size up to a whole number of entries (already is)
    // mark all used blocks in the bitmap
    let used = fs.freeblock;
    for b in 0..used {
        let o = fs.bmapstart * BSIZE + b / 8;
        fs.img[o] |= 1 << (b % 8);
    }
    fs.img.resize((FSSIZE + SWAP_BLOCKS) * BSIZE, 0); // zeroed swap area appended after the file system
    Ok(fs.img)
}
