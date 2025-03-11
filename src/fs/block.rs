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

use core::default::Default;
use core::ops::{Deref, DerefMut};
use core::option::Option::{self, None, Some};
use core::result::Result::{self, Ok};
use core::{cmp, unreachable};

use crate::fs::{BlockDevice, DeviceError, Storage};

pub struct BlockBuffer {
    buf:    [Block; 4],
    status: u8,
}
pub struct BlockEntryIter {
    buf:   BlockBuffer,
    prev:  Option<u32>,
    last:  u32,
    count: u8,
}
#[repr(transparent)]
pub struct BlockCache(Option<u32>);
#[repr(transparent)]
pub struct Block([u8; Block::SIZE]);

impl Block {
    pub const SIZE: usize = 0x200;

    #[inline(always)]
    pub const fn new() -> Block {
        Block([0u8; Block::SIZE])
    }

    #[inline(always)]
    pub fn clear(&mut self) {
        self.0.fill(0)
    }
}
impl BlockCache {
    #[inline(always)]
    pub fn new() -> BlockCache {
        BlockCache(None)
    }

    #[inline(always)]
    pub fn clear(&mut self) {
        self.0.take();
    }
    #[inline]
    pub fn read_single(&mut self, dev: &Storage<impl BlockDevice>, b: &mut Block, pos: u32) -> Result<(), DeviceError> {
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
    pub fn new() -> BlockBuffer {
        BlockBuffer {
            buf:    [Block::new(), Block::new(), Block::new(), Block::new()],
            status: 0u8,
        }
    }

    #[inline]
    pub fn is_dirty(&self, slot: u8) -> bool {
        match slot % BlockBuffer::COUNT {
            BlockBuffer::SLOT_A if self.status & 0x80 != 0 => true,
            BlockBuffer::SLOT_B if self.status & 0x40 != 0 => true,
            BlockBuffer::SLOT_C if self.status & 0x20 != 0 => true,
            BlockBuffer::SLOT_D if self.status & 0x10 != 0 => true,
            _ => false,
        }
    }
    #[inline]
    pub fn is_loaded(&self, slot: u8) -> bool {
        match slot % BlockBuffer::COUNT {
            BlockBuffer::SLOT_A if self.status & 0x8 != 0 => true,
            BlockBuffer::SLOT_B if self.status & 0x4 != 0 => true,
            BlockBuffer::SLOT_C if self.status & 0x2 != 0 => true,
            BlockBuffer::SLOT_D if self.status & 0x1 != 0 => true,
            _ => false,
        }
    }
    #[inline]
    pub fn buffer(&mut self, slot: u8) -> &mut [u8] {
        match slot % BlockBuffer::COUNT {
            BlockBuffer::SLOT_A => {
                self.status |= 0x80;
                &mut self.buf[0]
            },
            BlockBuffer::SLOT_B => {
                self.status |= 0x40;
                &mut self.buf[1]
            },
            BlockBuffer::SLOT_C => {
                self.status |= 0x20;
                &mut self.buf[2]
            },
            BlockBuffer::SLOT_D => {
                self.status |= 0x10;
                &mut self.buf[3]
            },
            _ => unreachable!(),
        }
    }
    pub fn flush(&mut self, dev: &Storage<impl BlockDevice>, start: u32) -> Result<(), DeviceError> {
        let s = self.status;
        self.status = s & 0xF;
        match s {
            // Contiguous
            // ABCD
            v if (v >> 4) == 0xF => return dev.write(&self.buf, start),
            // ABC
            v if (v >> 4) == 0xE => return dev.write(&self.buf[0..3], start),
            // AB
            v if (v >> 4) == 0xC => return dev.write(&self.buf[0..2], start),
            // CD
            v if (v >> 4) == 0x3 => return dev.write(&self.buf[2..], start + 2),
            // BCD
            v if (v >> 4) == 0x7 => return dev.write(&self.buf[1..], start + 1),
            // BC
            v if (v >> 4) == 0x6 => return dev.write(&self.buf[1..3], start + 1),
            // Singles
            // D
            v if (v >> 4) == 1 => return dev.write_single(&self.buf[3], start + 3),
            // C
            v if (v >> 4) == 2 => return dev.write_single(&self.buf[2], start + 2),
            // B
            v if (v >> 4) == 4 => return dev.write_single(&self.buf[1], start + 1),
            // A
            v if (v >> 4) == 8 => return dev.write_single(&self.buf[0], start),
            _ => (),
        }
        // A
        if s & 0x80 != 0 {
            dev.write_single(&self.buf[0], start)?;
        }
        // B
        if s & 0x40 != 0 {
            dev.write_single(&self.buf[1], start + 1)?;
        }
        // C
        if s & 0x20 != 0 {
            dev.write_single(&self.buf[2], start + 2)?;
        }
        // D
        if s & 0x10 != 0 {
            dev.write_single(&self.buf[3], start + 3)?;
        }
        Ok(())
    }
    #[inline]
    pub fn read(&mut self, dev: &Storage<impl BlockDevice>, count: u8, start: u32) -> Result<(), DeviceError> {
        let n = cmp::min(count, BlockBuffer::COUNT);
        dev.read(&mut self.buf[0..n as usize], start)?;
        self.status = match n {
            4 => 0xF,
            3 => 0xE,
            2 => 0xC,
            1 => 0x8,
            _ => unreachable!(),
        };
        Ok(())
    }
}
impl BlockEntryIter {
    #[inline(always)]
    pub fn new(count: u8) -> BlockEntryIter {
        BlockEntryIter {
            count,
            buf: BlockBuffer::new(),
            prev: None,
            last: 0u32,
        }
    }

    #[inline(always)]
    pub fn pos(&self) -> u32 {
        self.prev.unwrap_or(0)
    }
    #[inline(always)]
    pub fn is_loaded(&self) -> bool {
        self.prev.is_none()
    }
    #[inline(always)]
    pub fn in_scope(&self, pos: u32) -> bool {
        (pos - self.pos()) < BlockBuffer::COUNT as u32
    }
    #[inline(always)]
    pub fn buffer(&mut self, pos: u32) -> &mut [u8] {
        self.buf.buffer((pos - self.pos()) as u8)
    }
    #[inline]
    pub fn flush(&mut self, dev: &Storage<impl BlockDevice>) -> Result<(), DeviceError> {
        if let Some(v) = self.prev.take() {
            self.buf.flush(dev, v)?;
        }
        Ok(())
    }
    pub fn load(&mut self, dev: &Storage<impl BlockDevice>, pos: u32) -> Result<(), DeviceError> {
        if self.last != 0 && pos < self.last {
            return Ok(());
        }
        let i = cmp::min(self.count, BlockBuffer::COUNT);
        self.buf.read(dev, i, pos)?;
        self.last = self.last.saturating_add(i as u32);
        self.count = self.count.saturating_sub(i);
        self.prev = Some(pos);
        Ok(())
    }
    #[inline]
    pub fn load_and_flush(&mut self, dev: &Storage<impl BlockDevice>, pos: u32) -> Result<(), DeviceError> {
        self.flush(dev)?;
        self.load(dev, pos)
    }
}

impl Deref for Block {
    type Target = [u8];

    #[inline(always)]
    fn deref(&self) -> &[u8] {
        &self.0
    }
}
impl Default for Block {
    #[inline]
    fn default() -> Block {
        Block([0u8; 0x200])
    }
}
impl DerefMut for Block {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut [u8] {
        &mut self.0
    }
}

impl Default for BlockCache {
    #[inline(always)]
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
