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

use core::clone::Clone;
use core::iter::{IntoIterator, Iterator};
use core::marker::Copy;
use core::ops::FnMut;
use core::option::Option::{self, None, Some};
use core::result::Result::{self, Err, Ok};

use crate::fs::{Block, BlockCache, BlockDevice, Cluster, DIR_SIZE, DeviceError, DirEntry, DirEntryFull, Directory, Volume};

pub struct Range {
    sel:   [RangeIndex; 0xA],
    sum:   u8,
    index: u8,
}
pub struct RangeIter {
    cur:   RangeIndex,
    range: Range,
    pos:   u8,
}
pub struct RangeIndex(u32, u32, u32);
pub struct DirectoryIndex<'a, B: BlockDevice> {
    vol:     &'a Volume<'a, B>,
    val:     DirEntryFull,
    buf:     Block,
    cache:   BlockCache,
    entry:   u32,
    block:   u32,
    blocks:  u32,
    cluster: u32,
}
pub struct DirectoryIter<'a, B: BlockDevice>(DirectoryIndex<'a, B>);
pub struct DirectoryIterMut<'b, 'a: 'b, B: BlockDevice>(&'b mut DirectoryIndex<'a, B>);

pub type RangeEntry = (u32, u32, bool, usize);

impl Range {
    #[inline]
    pub(super) fn new() -> Range {
        Range {
            sel:   [const { RangeIndex::new() }; 0xA],
            sum:   0u8,
            index: 0u8,
        }
    }

    #[inline(always)]
    pub fn blocks(&self) -> u8 {
        self.index
    }

    #[inline(always)]
    pub(super) fn clear(&mut self) {
        (self.sum, self.index) = (0u8, 0u8)
    }
    #[inline(always)]
    pub(super) fn finish(&mut self, c: u32, b: u32, e: u32) {
        self.sel[self.index as usize].set(c, b, e);
    }
    pub(super) fn mark(&mut self, c: u32, b: u32, e: u32) -> u8 {
        if self.index == 0 {
            self.sel[0].set(c, b, e);
            (self.index, self.sum) = (1, 1);
        } else {
            let v = &mut self.sel[self.index as usize - 1];
            if v.0 != c || v.1 != b {
                self.sel[self.index as usize].set(c, b, e);
                self.index += 1;
            }
            self.sum += 1;
        }
        self.sum
    }
}
impl RangeIter {
    fn is_next(&self) -> bool {
        if self.cur.2 >= (Block::SIZE as u32 / DIR_SIZE as u32) {
            return true;
        }
        if self.pos + 1 < self.range.index {
            return false;
        }
        let v = self.range.sel[self.pos as usize + 1].2;
        v == 0 || self.cur.2 > v
    }
    fn next_entry(&mut self) -> Option<RangeEntry> {
        let n = if self.is_next() {
            self.pos += 1;
            if self.pos >= self.range.index {
                return None;
            }
            self.cur = self.range.sel[self.pos as usize];
            true
        } else {
            false || self.range.sel[self.pos as usize].2 == self.cur.2
        };
        let e = self.cur.2 as usize;
        self.cur.2 += 1;
        Some((self.cur.0, self.cur.1, n, e))
    }
}
impl RangeIndex {
    #[inline(always)]
    const fn new() -> RangeIndex {
        RangeIndex(0u32, 0u32, 0u32)
    }

    #[inline(always)]
    fn set(&mut self, c: u32, b: u32, e: u32) {
        (self.0, self.1, self.2) = (c, b, e);
    }
}
impl<'b, 'a: 'b, B: BlockDevice> DirectoryIndex<'a, B> {
    #[inline]
    pub(super) fn new(vol: &'a Volume<'a, B>) -> DirectoryIndex<'a, B> {
        DirectoryIndex {
            vol,
            val: DirEntryFull::new(),
            buf: Block::new(),
            cache: BlockCache::new(),
            entry: 0u32,
            block: 0u32,
            blocks: 0u32,
            cluster: 0u32,
        }
    }

    /// This is the preferable function to use as it can be reset to save
    /// space in memory.
    #[inline(always)]
    pub fn into_iter_mut(&'b mut self) -> DirectoryIterMut<'b, 'a, B> {
        DirectoryIterMut(self)
    }
    #[inline(always)]
    pub fn reset(&mut self, dir: &Directory<'a, B>) -> Result<(), DeviceError> {
        self.setup(dir.cluster())
    }
    /// This function call cannot be "shortcut" stopped. Use "find" if stoppage
    /// is needed.
    #[inline]
    pub fn iter(&mut self, mut func: impl FnMut(&DirEntryFull)) -> Result<(), DeviceError> {
        loop {
            match self.next(true)? {
                Some(v) => func(v),
                None => break,
            }
        }
        Ok(())
    }
    #[inline]
    pub fn find(&mut self, mut func: impl FnMut(&DirEntryFull) -> bool) -> Result<Option<DirEntry>, DeviceError> {
        loop {
            match self.next(true)? {
                Some(v) if func(v) => return Ok(Some(v.entry())),
                Some(_) => continue,
                None => break,
            }
        }
        Ok(None)
    }

    pub(super) fn setup(&mut self, start: Cluster) -> Result<(), DeviceError> {
        self.entry = 0u32;
        self.cache.clear();
        self.cluster = start.unwrap_or_else(|| self.vol.man.root());
        let i = self.vol.man.block_pos_at(self.cluster);
        self.block = i;
        self.blocks = i + self.vol.man.entries_count(start);
        self.cache.read_single(self.vol.dev, &mut self.buf, i)?;
        Ok(())
    }

    #[inline(always)]
    fn is_loop_done(&self) -> bool {
        self.entry >= (Block::SIZE / DIR_SIZE) as u32
    }
    fn is_complete(&mut self) -> Result<bool, DeviceError> {
        if !self.is_loop_done() {
            return Ok(false);
        }
        (self.entry, self.block) = (0, self.block + 1);
        if self.block <= self.blocks {
            return self
                .cache
                .read_single(self.vol.dev, &mut self.buf, self.block)
                .map(|_| false);
        }
        self.cluster = match self
            .vol
            .man
            .cluster_next(self.vol.dev, &mut self.buf, &mut self.cache, self.cluster)?
        {
            None => return Ok(true),
            Some(v) => v,
        };
        let i = self.vol.man.block_pos_at(self.cluster);
        (self.block, self.blocks) = (i, i + self.vol.man.blocks.blocks_per_cluster());
        Ok(false)
    }
    fn next(&'b mut self, r: bool) -> Result<Option<&'b mut DirEntryFull>, DeviceError> {
        if self.is_complete()? {
            return Ok(None);
        }
        if r {
            // NOTE(sf): If re-entrant (from user call) reset the LFN cache.
            //           Since LFNs might span blocks.
            self.val.reset();
        }
        while !self.is_loop_done() {
            let s = self.entry as usize * DIR_SIZE;
            if self.buf[s] == 0 {
                return Ok(None);
            }
            self.entry += 1;
            if self.buf[s + 0xB] & 0xF == 0xF {
                self.val.fill(&self.buf[s..]);
                continue;
            }
            if self.buf[s + 0xB] & 0x8 != 0 && self.buf[s + 0x1C] == 0 {
                continue;
            }
            if self.buf[s] != 0xE5 {
                self.val.load(&self.vol.man.ver, &self.buf[s..], self.block, s as u32);
                // NOTE(sf): We're just returning a reference of the same struct.
                //           Since the Iter trait won't allow lifetimes, this
                //           is the next best thing. It should prevent the
                //           pointer leaking out of scope.
                return Ok(Some(&mut self.val));
            }
        }
        self.next(false)
    }
}

impl Copy for RangeIndex {}
impl Clone for RangeIndex {
    #[inline(always)]
    fn clone(&self) -> RangeIndex {
        RangeIndex(self.0, self.1, self.2)
    }
}

impl Iterator for RangeIter {
    type Item = RangeEntry;

    #[inline(always)]
    fn next(&mut self) -> Option<RangeEntry> {
        self.next_entry()
    }
}

impl Clone for Range {
    #[inline(always)]
    fn clone(&self) -> Range {
        Range {
            sel:   self.sel.clone(),
            sum:   self.sum,
            index: self.index,
        }
    }
}
impl IntoIterator for Range {
    type Item = RangeEntry;
    type IntoIter = RangeIter;

    #[inline(always)]
    fn into_iter(self) -> RangeIter {
        RangeIter {
            pos:   0u8,
            cur:   self.sel[0],
            range: self,
        }
    }
}

impl<'a, B: BlockDevice> IntoIterator for DirectoryIndex<'a, B> {
    type IntoIter = DirectoryIter<'a, B>;
    type Item = Result<DirEntry, DeviceError>;

    #[inline(always)]
    fn into_iter(self) -> DirectoryIter<'a, B> {
        DirectoryIter(self)
    }
}
impl<'b, 'a: 'b, B: BlockDevice> IntoIterator for &'b mut DirectoryIndex<'a, B> {
    type IntoIter = DirectoryIterMut<'b, 'a, B>;
    type Item = Result<DirEntry, DeviceError>;

    #[inline(always)]
    fn into_iter(self) -> DirectoryIterMut<'b, 'a, B> {
        DirectoryIterMut(self)
    }
}

impl<'a, B: BlockDevice> Iterator for DirectoryIter<'a, B> {
    type Item = Result<DirEntry, DeviceError>;

    #[inline]
    fn next(&mut self) -> Option<Result<DirEntry, DeviceError>> {
        match self.0.next(true) {
            Ok(None) => None,
            Ok(Some(v)) => Some(Ok(v.entry())),
            Err(e) => Some(Err(e)),
        }
    }
}

impl<'b, 'a: 'b, B: BlockDevice> Iterator for DirectoryIterMut<'b, 'a, B> {
    type Item = Result<DirEntry, DeviceError>;

    #[inline]
    fn next(&mut self) -> Option<Result<DirEntry, DeviceError>> {
        match self.0.next(true) {
            Ok(None) => None,
            Ok(Some(v)) => Some(Ok(v.entry())),
            Err(e) => Some(Err(e)),
        }
    }
}
