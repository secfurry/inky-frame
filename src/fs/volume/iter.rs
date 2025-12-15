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
use core::result::Result::{Err, Ok};

use crate::Slice;
use crate::fs::{Block, BlockCache, BlockDevice, BlockPtr, Cache, Cluster, ClusterIndex, DevResult, DirEntry, DirEntryFull, DirEntryPtr, Directory, Volume};

pub struct Range {
    sel:   [RangeIndex; 10], // Contagious free Entries found
    sum:   u8,               // Count of free Entries contained.
    index: u8,               // Slots used.
}
pub struct RangeIter {
    cur:   RangeIndex,
    pos:   u8,
    range: Range,
}
pub struct RangeIndex(u32, u32, u32); // Stores Block, Cluster, Entry
pub struct DirectoryIndex<'a, B: BlockDevice> {
    buf:     BlockPtr,
    val:     DirEntryFull,
    vol:     &'a Volume<'a, B>,
    block:   u32,
    cache:   BlockCache,
    entry:   u32,
    blocks:  u32,
    cluster: ClusterIndex,
}
pub struct DirectoryIter<'a, B: BlockDevice>(DirectoryIndex<'a, B>);
pub struct DirectoryIterMut<'b, 'a: 'b, B: BlockDevice>(&'b mut DirectoryIndex<'a, B>);

pub type RangeEntry = (u32, u32, bool, usize);

impl Range {
    #[inline]
    pub fn blocks(&self) -> u8 {
        self.index
    }

    #[inline]
    pub(super) fn new() -> Range {
        Range {
            sel:   [const { RangeIndex::new() }; 10],
            sum:   0u8,
            index: 0u8,
        }
    }

    #[inline]
    pub(super) fn clear(&mut self) {
        (self.sum, self.index) = (0u8, 0u8)
    }
    #[inline]
    pub(super) fn finish(&mut self, c: u32, b: u32, e: u32) {
        // Can't be over 10.
        unsafe { self.sel.get_unchecked_mut(self.index as usize) }.set(c, b, e);
    }
    pub(super) fn mark(&mut self, c: u32, b: u32, e: u32) -> u8 {
        if self.index == 0 {
            unsafe { self.sel.get_unchecked_mut(0) }.set(c, b, e);
            (self.index, self.sum) = (1, 1);
        } else {
            let v = &self.sel[self.index as usize - 1];
            if v.0 != c || v.1 != b {
                if self.index >= 9 {
                    self.clear(); // Reset the array, can't find anything
                }
                unsafe { self.sel.get_unchecked_mut(self.index as usize) }.set(c, b, e);
                self.index += 1;
            }
            self.sum += 1;
        }
        self.sum
    }
}
impl RangeIter {
    #[inline]
    fn is_next(&self) -> bool {
        if self.cur.2 >= DirEntry::SIZE_PER_BLOCK {
            return true;
        }
        if self.pos + 1 < self.range.index {
            return false;
        }
        let v = unsafe { self.range.sel.get_unchecked(self.pos as usize + 1).2 };
        v == 0 || self.cur.2 > v
    }
}
impl RangeIndex {
    #[inline]
    const fn new() -> RangeIndex {
        RangeIndex(0u32, 0u32, 0u32)
    }

    #[inline]
    fn set(&mut self, c: u32, b: u32, e: u32) {
        (self.0, self.1, self.2) = (c, b, e);
    }
}
impl<'b, 'a: 'b, B: BlockDevice> DirectoryIndex<'a, B> {
    /// This is the preferable function to use as it can be reset to save
    /// space in memory.
    ///
    /// This also allows the usage of [`DirEntryPtr`] which contains the full
    /// file name as a LFN.
    #[inline]
    pub fn into_iter_mut(&'b mut self) -> DirectoryIterMut<'b, 'a, B> {
        DirectoryIterMut(self)
    }
    #[inline]
    pub fn reset(&mut self, dir: &Directory<'a, B>) -> DevResult<()> {
        unsafe { self.reset_cluster(dir.cluster()) }
    }
    /// This function call cannot be "shortcut" stopped. Use "find" if stoppage
    /// is needed.
    #[inline]
    pub fn iter(&mut self, mut func: impl FnMut(&DirEntryFull)) -> DevResult<()> {
        loop {
            match self.next(true)? {
                Some(v) => func(v),
                None => break,
            }
        }
        Ok(())
    }
    /// The provided closure can return `true` to "shortcut" and return the
    /// current entry.
    #[inline]
    pub fn find(&mut self, mut func: impl FnMut(&DirEntryFull) -> bool) -> DevResult<Option<DirEntry>> {
        loop {
            match self.next(true)? {
                Some(v) if func(v) => return Ok(Some(v.entry())),
                Some(_) => continue,
                None => break,
            }
        }
        Ok(None)
    }

    #[inline]
    pub unsafe fn reset_cluster(&mut self, dir: Cluster) -> DevResult<()> {
        self.entry = 0;
        self.cache.clear();
        self.cluster = self.vol.root(dir);
        self.block = self.vol.block_pos_at(self.cluster);
        self.blocks = self.block + self.vol.entries(dir);
        self.cache.read_single(self.vol.dev, &mut self.buf, self.block)
    }

    #[inline]
    pub(super) fn new(vol: &'a Volume<'a, B>) -> DirectoryIndex<'a, B> {
        DirectoryIndex {
            vol,
            buf: Cache::block_b(),
            val: DirEntryFull::new(),
            cache: BlockCache::new(),
            entry: 0u32,
            block: 0u32,
            blocks: 0u32,
            cluster: ClusterIndex::EMPTY,
        }
    }

    #[inline]
    fn is_loop_done(&self) -> bool {
        self.entry >= DirEntry::SIZE_PER_BLOCK
    }
    fn is_complete(&mut self) -> DevResult<bool> {
        if !self.is_loop_done() {
            return Ok(false);
        }
        (self.entry, self.block) = (0, self.block + 1);
        if self.block < self.blocks {
            let _ = self.cache.read_single(self.vol.dev, &mut self.buf, self.block)?;
            return Ok(false);
        }
        self.cluster = match self.vol.next(&mut self.buf, &mut self.cache, self.cluster)? {
            Some(v) => v,
            None => return Ok(true),
        };
        let i = self.vol.block_pos_at(self.cluster);
        (self.block, self.blocks) = (i, i + self.vol.block.blocks());
        // Read new Block data
        let _ = self.cache.read_single(self.vol.dev, &mut self.buf, self.block)?;
        Ok(false)
    }
    fn next(&'b mut self, r: bool) -> DevResult<Option<&'b mut DirEntryFull>> {
        if self.is_complete()? {
            return Ok(None);
        }
        if r {
            // If re-entrant (from user call) reset the LFN cache.
            // Since LFNs might span blocks.
            self.val.reset();
        }
        while !self.is_loop_done() {
            let s = self.entry as usize * DirEntry::SIZE;
            if s + DirEntry::SIZE > Block::SIZE || self.buf.read_u8(s) == 0 {
                return Ok(None);
            }
            self.entry += 1;
            let v = self.buf.read_u8(s + 11);
            if v & 0xF == 0xF {
                self.val.fill(unsafe { self.buf.get_unchecked(s..) });
                continue;
            }
            if (v & 0x8 != 0 && self.buf.read_u8(s + 28) == 0) || self.buf.read_u8(s) == 0xE5 {
                continue;
            }
            self.val.load(
                self.vol.ver.is_fat32(),
                unsafe { self.buf.get_unchecked(s..) },
                self.block,
                s as u32,
            );
            // NOTE(sf): We're just returning a reference of the same struct.
            //           Since the Iter trait won't allow lifetimes, this
            //           is the next best thing. It should prevent the
            //           pointer leaking out of scope.
            return Ok(Some(&mut self.val));
        }
        self.next(false)
    }
}

impl Copy for RangeIndex {}
impl Clone for RangeIndex {
    #[inline]
    fn clone(&self) -> RangeIndex {
        RangeIndex(self.0, self.1, self.2)
    }
}

impl Clone for Range {
    #[inline]
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

    #[inline]
    fn into_iter(self) -> RangeIter {
        RangeIter {
            pos:   0u8,
            cur:   unsafe { *self.sel.get_unchecked(0) },
            range: self,
        }
    }
}

impl Iterator for RangeIter {
    type Item = RangeEntry;

    #[inline]
    fn next(&mut self) -> Option<RangeEntry> {
        let n = if self.is_next() {
            self.pos += 1;
            if self.pos >= self.range.index {
                return None;
            }
            self.cur = unsafe { *self.range.sel.get_unchecked(self.pos as usize) };
            true
        } else {
            false || unsafe { self.range.sel.get_unchecked(self.pos as usize).2 == self.cur.2 }
        };
        let e = self.cur.2 as usize;
        self.cur.2 += 1;
        Some((self.cur.0, self.cur.1, n, e))
    }
}

impl<'a, B: BlockDevice> Iterator for DirectoryIter<'a, B> {
    type Item = DevResult<DirEntry>;

    #[inline]
    fn next(&mut self) -> Option<DevResult<DirEntry>> {
        match self.0.next(true) {
            Ok(Some(v)) => Some(Ok(v.entry())),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}
impl<'b, 'a: 'b, B: BlockDevice> Iterator for DirectoryIterMut<'b, 'a, B> {
    type Item = DevResult<DirEntryPtr<'b>>;

    #[inline]
    fn next(&mut self) -> Option<DevResult<DirEntryPtr<'b>>> {
        // NOTE(sf): DirEntryPtr is only avaliable here as we can bind the
        //           ptr value to the "b" lifetime to prevent it from escaping.
        //           This can't be done with "DirectoryIter" as binding to "a"
        //           would allow the ptr to be alive as long as the Volume, which
        //           would not be valid.
        match self.0.next(true) {
            Ok(Some(v)) => Some(Ok(v.ptr())),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}

impl<'a, B: BlockDevice> IntoIterator for DirectoryIndex<'a, B> {
    type Item = DevResult<DirEntry>;
    type IntoIter = DirectoryIter<'a, B>;

    #[inline]
    fn into_iter(self) -> DirectoryIter<'a, B> {
        DirectoryIter(self)
    }
}
impl<'b, 'a: 'b, B: BlockDevice> IntoIterator for &'b mut DirectoryIndex<'a, B> {
    type Item = DevResult<DirEntryPtr<'b>>;
    type IntoIter = DirectoryIterMut<'b, 'a, B>;

    #[inline]
    fn into_iter(self) -> DirectoryIterMut<'b, 'a, B> {
        DirectoryIterMut(self)
    }
}
