//! Shared block allocator for both bake-off variants (`mm-rc`, `mm-gc`).
//!
//! 64 KiB, 64 KiB-aligned blocks; 16-byte granules; size classes (multiples
//! of 16 up to 256, then multiples of 64 up to 2048); objects larger than
//! 2048 bytes get a dedicated ("large") chunk. Free objects are linked
//! through the word at `obj + 8` (so a freed object keeps a valid type byte
//! at `obj + 0`, which the RC variant's stale-purple validation reads).
//!
//! Both memory managers allocate through this heap, so allocation speed
//! itself is held constant and the measurement isolates the management
//! strategy: counts + trial deletion vs mark + conservative trace.

use std::alloc::{alloc, dealloc, handle_alloc_error, Layout};
use std::mem::MaybeUninit;

pub const BLOCK: usize = 65536;
pub const DATA_OFF: usize = 576;
pub const BLOCK_GRANULES: u32 = ((BLOCK - DATA_OFF) / 16) as u32; // 4060
pub const START_WORDS: usize = (BLOCK_GRANULES as usize + 63) / 64; // 64
pub const CHUNK_BLOCKS: usize = 16;
pub const CHUNK_BYTES: usize = BLOCK * CHUNK_BLOCKS;
pub const MAX_SMALL: usize = 2048;

pub const CLASS_NONE: u8 = 44; // block not yet assigned to a size class
pub const OWNER_NONE: u8 = 0; // current allocation source of its class
pub const OWNER_LIST: u8 = 1; // in the class's block list (has free space)
pub const OWNER_EMPTY: u8 = 2; // in the heap-wide empty pool (pristine)

/// 16, 32, ..., 256, 320, 384, ..., 2048.
pub const SIZES: [usize; 44] = [
    16, 32, 48, 64, 80, 96, 112, 128, 144, 160, 176, 192, 208, 224, 240, 256, 320, 384, 448, 512,
    576, 640, 704, 768, 832, 896, 960, 1024, 1088, 1152, 1216, 1280, 1344, 1408, 1472, 1536, 1600,
    1664, 1728, 1792, 1856, 1920, 1984, 2048,
];

#[inline]
pub fn class_index(size: usize) -> usize {
    debug_assert!(size >= 1 && size <= MAX_SMALL);
    if size <= 256 {
        ((size + 15) >> 4) - 1
    } else {
        16 + ((size - 257) >> 6)
    }
}

#[repr(C, align(65536))]
pub struct Block {
    pub magic: u64,
    pub class: u8,
    pub owner: u8,
    /// ever been on a class block list; guards chunk deallocation (a stale
    /// list entry must never point at returned memory).
    pub sticky_listed: u8,
    _pad: u8,
    pub live_objects: u32,
    pub bump_next: u32, // next granule to carve (data-relative)
    pub granules: u32,
    pub freelist: *mut u8,
    pub chunk_base: *mut u8,
    _rsv: [u8; 24],
    /// one bit per granule: this granule starts an object.
    pub starts: [u64; START_WORDS],
    pub data: [MaybeUninit<u8>; BLOCK - DATA_OFF],
}

impl Block {
    #[inline]
    pub fn data_ptr(&self) -> *mut u8 {
        self.data.as_ptr() as *mut u8
    }
    #[inline]
    pub fn start_set(&mut self, g: usize) {
        self.starts[g >> 6] |= 1u64 << (g & 63);
    }
    #[inline]
    pub fn start_test(&self, g: usize) -> bool {
        self.starts[g >> 6] & (1u64 << (g & 63)) != 0
    }
    pub fn clear_starts(&mut self) {
        self.starts = [0; START_WORDS];
    }
}

#[derive(Clone, Copy)]
pub struct Chunk {
    pub base: usize,
    pub end: usize,
    pub large: bool,
    /// small chunks: how many blocks have been handed out (0..CHUNK_BLOCKS)
    pub claimed: u32,
}

pub struct ClassState {
    pub cur: *mut Block,
    pub blocks: Vec<*mut Block>,
}

pub struct Heap {
    /// sorted by base
    pub chunks: Vec<Chunk>,
    pub classes: Vec<ClassState>,
    pub empty_pool: Vec<*mut Block>,
    pub min_addr: usize,
    pub max_addr: usize,
    pub reserved: usize,
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}

impl Heap {
    pub fn new() -> Heap {
        Heap {
            chunks: Vec::new(),
            classes: (0..SIZES.len())
                .map(|_| ClassState {
                    cur: std::ptr::null_mut(),
                    blocks: Vec::new(),
                })
                .collect(),
            empty_pool: Vec::new(),
            min_addr: usize::MAX,
            max_addr: 0,
            reserved: 0,
        }
    }

    /// Returns (base, index into `self.chunks`): the chunk list is sorted
    /// by address and mmap does not hand out monotonic addresses, so the
    /// new chunk is not necessarily the last one.
    fn new_chunk(&mut self, large: bool, min_payload: usize) -> (*mut u8, usize) {
        let total = if large {
            min_payload.div_ceil(BLOCK) * BLOCK
        } else {
            CHUNK_BYTES
        };
        let layout = Layout::from_size_align(total, BLOCK).unwrap();
        let base = unsafe { alloc(layout) };
        if base.is_null() {
            handle_alloc_error(layout);
        }
        let base = base as usize;
        let idx = self.chunks.partition_point(|c| c.base < base);
        self.chunks.insert(
            idx,
            Chunk {
                base,
                end: base + total,
                large,
                claimed: 0,
            },
        );
        if !large {
            for i in 0..CHUNK_BLOCKS {
                let b = (base + i * BLOCK) as *mut Block;
                unsafe {
                    std::ptr::write(
                        b,
                        Block {
                            magic: 0x524f4249474f4241,
                            class: CLASS_NONE,
                            owner: OWNER_NONE,
                            sticky_listed: 0,
                            _pad: 0,
                            live_objects: 0,
                            bump_next: 0,
                            granules: BLOCK_GRANULES,
                            freelist: std::ptr::null_mut(),
                            chunk_base: base as *mut u8,
                            _rsv: [0; 24],
                            starts: [0; START_WORDS],
                            data: [MaybeUninit::uninit(); BLOCK - DATA_OFF],
                        },
                    );
                }
            }
        }
        self.min_addr = self.min_addr.min(base);
        self.max_addr = self.max_addr.max(base + total);
        self.reserved += total;
        (base as *mut u8, idx)
    }

    fn fresh_block(&mut self) -> *mut Block {
        for ci in (0..self.chunks.len()).rev() {
            let c = &self.chunks[ci];
            if !c.large && (c.claimed as usize) < CHUNK_BLOCKS {
                let b = (c.base + c.claimed as usize * BLOCK) as *mut Block;
                self.chunks[ci].claimed += 1;
                return b;
            }
        }
        let (base, idx) = self.new_chunk(false, 0);
        self.chunks[idx].claimed = 1;
        base as *mut Block
    }

    pub fn alloc(&mut self, size: usize) -> *mut u8 {
        debug_assert!(size >= 16);
        if size > MAX_SMALL {
            return self.alloc_large(size);
        }
        self.alloc_small(size)
    }

    fn alloc_small(&mut self, size: usize) -> *mut u8 {
        let ci = class_index(size);
        let g = size.div_ceil(16) as u32;
        loop {
            let cs = unsafe { &mut *(&mut self.classes[ci] as *mut ClassState) };
            if !cs.cur.is_null() {
                let b = unsafe { &mut *cs.cur };
                if !b.freelist.is_null() {
                    let p = b.freelist;
                    b.freelist = unsafe { *((p as *mut *mut u8).add(1)) }; // next at +8
                    // the sweeper rebuilds start bits from survivors only,
                    // so a slot recycled from the free list needs its bit
                    // back or the next sweep cannot see (nor free, nor
                    // resolve) the new object
                    unsafe { b.start_set((p as usize - b.data_ptr() as usize) / 16) };
                    b.live_objects += 1;
                    return p;
                }
                if b.bump_next + g <= b.granules {
                    let p = unsafe { b.data_ptr().add(b.bump_next as usize * 16) };
                    b.start_set(b.bump_next as usize);
                    b.bump_next += g;
                    b.live_objects += 1;
                    return p;
                }
                cs.cur = std::ptr::null_mut();
            }
            // advance to a block with free space
            let mut found = std::ptr::null_mut();
            while let Some(b) = cs.blocks.pop() {
                let bb = unsafe { &mut *b };
                if bb.owner == OWNER_LIST && bb.class as usize == ci {
                    bb.owner = OWNER_NONE;
                    found = b;
                    break;
                }
            }
            if !found.is_null() {
                cs.cur = found;
                continue;
            }
            if let Some(b) = self.empty_pool.pop() {
                let bb = unsafe { &mut *b };
                bb.owner = OWNER_NONE;
                bb.class = ci as u8;
                cs.cur = b;
                continue;
            }
            let b = self.fresh_block();
            unsafe { (*b).class = ci as u8 };
            cs.cur = b;
        }
    }

    fn alloc_large(&mut self, size: usize) -> *mut u8 {
        // object header + payload live after a pseudo block header area
        let total_payload = size + DATA_OFF;
        let (base, _) = self.new_chunk(true, total_payload);
        let base = base as usize;
        (base + DATA_OFF) as *mut u8
    }

    /// Immediate free of a small object (the RC variant's death path).
    pub fn free_small(&mut self, p: *mut u8) {
        let b = (p as usize & !(BLOCK - 1)) as *mut Block;
        let ci = unsafe { (*b).class as usize };
        debug_assert!(ci < SIZES.len());
        unsafe {
            *((p as *mut *mut u8).add(1)) = (*b).freelist;
            (*b).freelist = p;
            (*b).live_objects -= 1;
            if (*b).owner == OWNER_NONE && self.classes[ci].cur != b {
                (*b).owner = OWNER_LIST;
                (*b).sticky_listed = 1;
                self.classes[ci].blocks.push(b);
            }
        }
    }

    /// Immediate free of a large object: the whole chunk goes back to the OS.
    pub fn free_large(&mut self, p: *mut u8) {
        let w = p as usize;
        let idx = self.chunks.partition_point(|c| c.base < w) - 1;
        if idx >= self.chunks.len() {
            return;
        }
        let c = &self.chunks[idx];
        debug_assert!(c.large && w == c.base + DATA_OFF);
        let total = c.end - c.base;
        unsafe {
            dealloc(
                c.base as *mut u8,
                Layout::from_size_align(total, BLOCK).unwrap(),
            );
        }
        self.reserved -= total;
        self.chunks.remove(idx);
    }

    /// Binary-search the chunk containing `w`, if any.
    #[inline]
    pub fn chunk_of(&self, w: usize) -> Option<&Chunk> {
        if w < self.min_addr || w >= self.max_addr {
            return None;
        }
        let idx = self.chunks.partition_point(|c| c.base <= w) - 1;
        let c = &self.chunks[idx];
        if w < c.end {
            Some(c)
        } else {
            None
        }
    }

    /// Resolve a possibly-interior pointer to the start of the object it
    /// points into (conservative stack scanning).
    pub fn resolve(&self, w: usize) -> Option<*mut u8> {
        let c = self.chunk_of(w)?;
        if c.large {
            let obj = c.base + DATA_OFF;
            return if w >= obj { Some(obj as *mut u8) } else { None };
        }
        let baddr = w & !(BLOCK - 1);
        if baddr < c.base {
            return None;
        }
        let b = baddr as *mut Block;
        unsafe {
            let data = (*b).data_ptr() as usize;
            if w < data {
                return None;
            }
            let g = (w - data) / 16;
            if g >= (*b).granules as usize {
                return None;
            }
            let mut i = g;
            loop {
                if (*b).start_test(i) {
                    return Some((data + i * 16) as *mut u8);
                }
                if i == 0 {
                    return None;
                }
                i -= 1;
            }
        }
    }

    #[inline]
    pub fn block_of(&self, p: *const u8) -> *mut Block {
        (p as usize & !(BLOCK - 1)) as *mut Block
    }
}
