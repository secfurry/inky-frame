// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.
//

#![no_implicit_prelude]

extern crate core;

use core::cmp::Ord;
use core::default::Default;
use core::hint::unreachable_unchecked;
use core::mem::{MaybeUninit, transmute};
use core::ops::{Deref, DerefMut};
use core::option::Option::{self, None, Some};
use core::ptr::write_bytes;
use core::result::Result::Ok;

use crate::fs::{BlockDevice, DevResult, Storage};
use crate::{Slice, SliceMut};

pub struct BlockBuffer {
    buf:    [Block; 4],
    status: u8,
}
pub struct BlockEntryIter {
    buf:   BlockBuffer,
    last:  u32,
    prev:  Option<u32>,
    count: u8,
}
#[repr(transparent)]
pub struct BlockCache(Option<u32>);
#[repr(transparent)]
pub struct Block([MaybeUninit<u8>; Block::SIZE]);

impl Block {
    pub const SIZE: usize = 0x200;

    #[inline]
    pub const fn new() -> Block {
        Block([MaybeUninit::uninit(); Block::SIZE])
    }

    #[inline]
    pub fn clear(&mut self) {
        unsafe { write_bytes(self.0.as_mut_ptr(), 0u8, Block::SIZE) };
    }
}
impl BlockCache {
    #[inline]
    pub const fn new() -> BlockCache {
        BlockCache(None)
    }

    #[inline]
    pub fn clear(&mut self) {
        let _ = self.0.take();
    }
    #[inline]
    pub fn read_single<B: BlockDevice>(&mut self, dev: &Storage<B>, b: &mut Block, pos: u32) -> DevResult<()> {
        match self.0.replace(pos) {
            Some(v) if v != pos => dev.read_single(b, pos),
            None => dev.read_single(b, pos),
            _ => Ok(()),
        }
    }
}
impl BlockBuffer {
    pub const COUNT: u8 = 0x4u8;
    pub const SLOT_A: u8 = 0x0u8;
    pub const SLOT_B: u8 = 0x1u8;
    pub const SLOT_C: u8 = 0x2u8;
    pub const SLOT_D: u8 = 0x3u8;

    #[inline]
    pub const fn new() -> BlockBuffer {
        BlockBuffer {
            buf:    [Block::new(), Block::new(), Block::new(), Block::new()],
            status: 0u8,
        }
    }

    #[inline]
    pub fn is_dirty(&self, slot: u8) -> bool {
        match slot {
            BlockBuffer::SLOT_A if self.status & 0x80 != 0 => true,
            BlockBuffer::SLOT_B if self.status & 0x40 != 0 => true,
            BlockBuffer::SLOT_C if self.status & 0x20 != 0 => true,
            BlockBuffer::SLOT_D if self.status & 0x10 != 0 => true,
            _ => false,
        }
    }
    #[inline]
    pub fn is_loaded(&self, slot: u8) -> bool {
        match slot {
            BlockBuffer::SLOT_A if self.status & 0x8 != 0 => true,
            BlockBuffer::SLOT_B if self.status & 0x4 != 0 => true,
            BlockBuffer::SLOT_C if self.status & 0x2 != 0 => true,
            BlockBuffer::SLOT_D if self.status & 0x1 != 0 => true,
            _ => false,
        }
    }
    #[inline]
    pub fn buffer(&mut self, slot: u8) -> &mut Block {
        match slot.min(BlockBuffer::COUNT - 1) {
            BlockBuffer::SLOT_A => {
                self.status |= 0x80;
                unsafe { self.buf.get_unchecked_mut(0) }
            },
            BlockBuffer::SLOT_B => {
                self.status |= 0x40;
                unsafe { self.buf.get_unchecked_mut(1) }
            },
            BlockBuffer::SLOT_C => {
                self.status |= 0x20;
                unsafe { self.buf.get_unchecked_mut(2) }
            },
            BlockBuffer::SLOT_D => {
                self.status |= 0x10;
                unsafe { self.buf.get_unchecked_mut(3) }
            },
            _ => unsafe { unreachable_unchecked() },
        }
    }
    pub fn flush<B: BlockDevice>(&mut self, dev: &Storage<B>, start: u32) -> DevResult<()> {
        let s = self.status;
        self.status = s & 0xF;
        match unsafe { s.unchecked_shr(4) } {
            // Contiguous
            // ABCD
            0xF => return dev.write(&self.buf, start),
            // ABC
            0xE => return dev.write(unsafe { self.buf.get_unchecked(0..3) }, start),
            // AB
            0xC => return dev.write(unsafe { self.buf.get_unchecked(0..2) }, start),
            // CD
            0x3 => return dev.write(unsafe { self.buf.get_unchecked(2..) }, start + 2),
            // BCD
            0x7 => return dev.write(unsafe { self.buf.get_unchecked(1..) }, start + 1),
            // BC
            0x6 => return dev.write(unsafe { self.buf.get_unchecked(1..3) }, start + 1),
            // Singles
            // D
            0x1 => return dev.write_single(unsafe { self.buf.get_unchecked(3) }, start + 3),
            // C
            0x2 => return dev.write_single(unsafe { self.buf.get_unchecked(2) }, start + 2),
            // B
            0x4 => return dev.write_single(unsafe { self.buf.get_unchecked(1) }, start + 1),
            // A
            0x8 => return dev.write_single(unsafe { self.buf.get_unchecked(0) }, start),
            _ => (),
        }
        // A
        if s & 0x80 != 0 {
            dev.write_single(unsafe { self.buf.get_unchecked(0) }, start)?;
        }
        // B
        if s & 0x40 != 0 {
            dev.write_single(unsafe { self.buf.get_unchecked(1) }, start + 1)?;
        }
        // C
        if s & 0x20 != 0 {
            dev.write_single(unsafe { self.buf.get_unchecked(2) }, start + 2)?;
        }
        // D
        if s & 0x10 != 0 {
            dev.write_single(unsafe { self.buf.get_unchecked(3) }, start + 3)?;
        }
        Ok(())
    }
    #[inline]
    pub fn read<B: BlockDevice>(&mut self, dev: &Storage<B>, count: u8, start: u32) -> DevResult<()> {
        let n = count.min(BlockBuffer::COUNT - 1) as usize;
        dev.read(unsafe { self.buf.get_unchecked_mut(0..n) }, start)?;
        self.status = match n {
            4 => 0xF,
            3 => 0xE,
            2 => 0xC,
            1 => 0x8,
            _ => unsafe { unreachable_unchecked() },
        };
        Ok(())
    }
}
impl BlockEntryIter {
    #[inline]
    pub fn new(count: u8) -> BlockEntryIter {
        BlockEntryIter {
            count,
            buf: BlockBuffer::new(),
            last: 0u32,
            prev: None,
        }
    }

    #[inline]
    pub fn pos(&self) -> u32 {
        self.prev.unwrap_or(0)
    }
    #[inline]
    pub fn is_loaded(&self) -> bool {
        self.prev.is_none()
    }
    #[inline]
    pub fn in_scope(&self, pos: u32) -> bool {
        (pos - self.pos()) < BlockBuffer::COUNT as u32
    }
    #[inline]
    pub fn buffer(&mut self, pos: u32) -> &mut [u8] {
        self.buf.buffer((pos - self.pos()) as u8)
    }
    #[inline]
    pub fn flush<B: BlockDevice>(&mut self, dev: &Storage<B>) -> DevResult<()> {
        if let Some(v) = self.prev.take() {
            self.buf.flush(dev, v)?;
        }
        Ok(())
    }
    pub fn load<B: BlockDevice>(&mut self, dev: &Storage<B>, pos: u32) -> DevResult<()> {
        if self.last != 0 && pos < self.last {
            return Ok(());
        }
        let i = self.count % BlockBuffer::COUNT;
        self.buf.read(dev, i, pos)?;
        self.last = self.last.saturating_add(i as u32);
        self.count = self.count.saturating_sub(i);
        self.prev = Some(pos);
        Ok(())
    }
    #[inline]
    pub fn load_and_flush<B: BlockDevice>(&mut self, dev: &Storage<B>, pos: u32) -> DevResult<()> {
        self.flush(dev)?;
        self.load(dev, pos)
    }
}

impl Deref for Block {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &[u8] {
        unsafe { transmute(self.0.as_slice()) }
    }
}
impl Default for Block {
    #[inline]
    fn default() -> Block {
        Block::new()
    }
}
impl DerefMut for Block {
    #[inline]
    fn deref_mut(&mut self) -> &mut [u8] {
        unsafe { transmute(self.0.as_mut_slice()) }
    }
}

impl Slice for Block {
    #[inline]
    fn as_ptr(&self) -> *const u8 {
        self.0.as_ptr() as *const u8
    }
}
impl Slice for &Block {
    #[inline]
    fn as_ptr(&self) -> *const u8 {
        self.0.as_ptr() as *const u8
    }
}
impl Slice for &mut Block {
    #[inline]
    fn as_ptr(&self) -> *const u8 {
        self.0.as_ptr() as *const u8
    }
}

impl SliceMut for Block {
    #[inline]
    fn as_mut_ptr(&mut self) -> *mut u8 {
        self.0.as_mut_ptr() as *mut u8
    }
}
impl SliceMut for &mut Block {
    #[inline]
    fn as_mut_ptr(&mut self) -> *mut u8 {
        self.0.as_mut_ptr() as *mut u8
    }
}

impl Default for BlockCache {
    #[inline]
    fn default() -> BlockCache {
        BlockCache(None)
    }
}
impl Default for BlockBuffer {
    #[inline]
    fn default() -> BlockBuffer {
        BlockBuffer::new()
    }
}
