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
extern crate rpsp;

use core::cmp::PartialEq;
use core::convert::{AsRef, Into, TryInto};
use core::marker::PhantomData;
use core::mem::{drop, transmute};
use core::ops::{Deref, Drop};
use core::option::Option::{None, Some};
use core::result::Result::{self, Err, Ok};
use core::{cmp, matches, mem};

use rpsp::ignore_error;
use rpsp::io::{Read, Seek, SeekFrom, Write};
use rpsp::time::{Time, Weekday};

use crate::fs::state::{Safe, Unsafe};
use crate::fs::{Block, BlockCache, BlockDevice, BlockPtr, Cache, Cluster, DIR_SIZE, DeviceError, DirectoryIndex, Error, FatVersion, LongName, LongNamePtr, ShortName, Storage, Volume, le_u16, le_u32, to_le_u16, to_le_u32};

const FILE_MAX_SIZE: u32 = 0xFFFFFFFFu32;

pub enum Mode {}

pub struct DirEntry {
    name:     ShortName,
    lfn:      u8,
    size:     u32,
    attrs:    u8,
    block:    u32,
    offset:   u32,
    created:  Time,
    cluster:  Cluster,
    modified: Time,
}
pub struct DirEntryFull {
    lfn:   LongNamePtr,
    sum:   u8,
    entry: DirEntry,
}
pub struct Reader<'a, B: BlockDevice> {
    f:   File<'a, B, Unsafe>,
    bp:  u32,
    buf: BlockPtr,
}
pub struct Directory<'a, B: BlockDevice> {
    vol: &'a Volume<'a, B>,
    dir: DirEntry,
}
pub struct File<'a, B: BlockDevice, S: FileSync = Safe> {
    vol:   &'a Volume<'a, B>,
    pos:   u32,
    file:  DirEntry,
    last:  u32,
    mode:  u8,
    short: u32,
    _p:    PhantomData<*const S>,
}

pub trait FileSync {
    fn cache() -> BlockPtr;
}

impl Mode {
    pub const READ: u8 = 0x0u8;
    pub const WRITE: u8 = 0x1u8;
    pub const CREATE: u8 = 0x2u8;
    pub const APPEND: u8 = 0x4u8;
    pub const TRUNCATE: u8 = 0x8u8;

    #[inline(always)]
    pub(super) fn is_create(m: u8) -> bool {
        m & Mode::CREATE != 0 && (m & Mode::WRITE != 0 || m & Mode::APPEND != 0)
    }
    #[inline(always)]
    pub(super) fn is_mode_valid(m: u8) -> bool {
        if (m >> 4) > 0 || m & (Mode::TRUNCATE | Mode::APPEND) == 0x12 || m == Mode::CREATE {
            false
        } else {
            true
        }
    }
}
impl DirEntry {
    #[inline]
    pub(super) fn new_root() -> DirEntry {
        DirEntry {
            lfn:      0,
            name:     ShortName::empty(),
            size:     0u32,
            block:    0u32,
            attrs:    0x10u8,
            offset:   0u32,
            created:  Time::empty(),
            cluster:  None,
            modified: Time::empty(),
        }
    }
    #[inline]
    pub(super) fn new(attrs: u8, lfn: u8) -> DirEntry {
        DirEntry {
            lfn,
            attrs,
            name: ShortName::empty(),
            size: 0u32,
            block: 0u32,
            offset: 0u32,
            created: Time::empty(),
            cluster: Some(0),
            modified: Time::empty(),
        }
    }
    #[inline]
    pub(super) fn new_self(parent: &DirEntry, block: u32) -> DirEntry {
        DirEntry {
            block,
            lfn: 0u8,
            name: ShortName::SELF,
            size: 0u32,
            attrs: 0x10u8,
            offset: 0u32,
            created: Time::empty(),
            cluster: parent.cluster,
            modified: Time::empty(),
        }
    }
    #[inline]
    pub(super) fn new_parent(cluster: Cluster, block: u32) -> DirEntry {
        DirEntry {
            block,
            cluster,
            lfn: 0u8,
            name: ShortName::PARENT,
            size: 0u32,
            attrs: 0x10u8,
            offset: 0x20u32,
            created: Time::empty(),
            modified: Time::empty(),
        }
    }

    #[inline(always)]
    pub fn size(&self) -> u32 {
        self.size
    }
    #[inline(always)]
    pub fn name(&self) -> &str {
        self.name.as_str()
    }
    #[inline(always)]
    pub fn offset(&self) -> u32 {
        self.offset
    }
    #[inline(always)]
    pub fn is_file(&self) -> bool {
        !self.is_directory()
    }
    #[inline(always)]
    pub fn created(&self) -> &Time {
        &self.created
    }
    #[inline(always)]
    pub fn attributes(&self) -> u8 {
        self.attrs
    }
    #[inline(always)]
    pub fn modified(&self) -> &Time {
        &self.modified
    }
    #[inline(always)]
    pub fn cluster(&self) -> Cluster {
        self.cluster
    }
    #[inline(always)]
    pub fn is_directory(&self) -> bool {
        self.attrs & 0x10 == 0x10
    }
    #[inline(always)]
    pub fn filename(&self) -> &ShortName {
        &self.name
    }
    #[inline(always)]
    pub fn set_created(&mut self, t: Time) {
        self.created = t
    }
    #[inline(always)]
    pub fn set_modified(&mut self, t: Time) {
        self.modified = t
    }
    #[inline(always)]
    pub fn set_attributes(&mut self, a: u8) {
        self.attrs = a
    }
    #[inline]
    pub fn into_dir<'a, B: BlockDevice>(self, vol: &'a Volume<B>) -> Result<Directory<'a, B>, DeviceError> {
        if !self.is_directory() {
            return Err(DeviceError::NotADirectory);
        }
        Ok(Directory::new(self, vol))
    }
    #[inline]
    pub fn into_file<'a, B: BlockDevice>(self, vol: &'a Volume<B>, mode: u8) -> Result<File<'a, B>, DeviceError> {
        if !Mode::is_mode_valid(mode) {
            return Err(DeviceError::InvalidOptions);
        }
        if !self.is_file() {
            return Err(DeviceError::NotAFile);
        }
        Ok(File::new(self, mode, vol))
    }

    #[inline(always)]
    pub(super) fn fill(&mut self, v: &[u8]) {
        self.name.fill(v);
    }
    #[inline(always)]
    pub(super) fn cluster_abs(&self) -> u32 {
        self.cluster.unwrap_or(0xFFFFFFFC)
    }
    #[inline]
    pub(super) fn is_root_or_parent(&self) -> bool {
        match self.cluster {
            Some(v) if v == 0xFFFFFFFC => return true,
            None => return true,
            _ => (),
        }
        self.name.0.is_empty() || self.name.is_self() || self.name.is_parent()
    }
    #[inline(always)]
    pub(super) fn write_prep(&mut self, block: u32, offset: usize) {
        self.block = block;
        self.offset = offset as u32;
    }
    pub(super) fn write_entry(&self, f: &FatVersion, b: &mut [u8]) {
        b[0..ShortName::SIZE].copy_from_slice(&self.name.as_bytes());
        b[11] = self.attrs;
        b[12] = 0;
        b[13] = 0;
        time_write(&self.created, &mut b[14..]);
        let c = self.cluster_abs();
        match f {
            FatVersion::Fat16(_) => (b[20], b[21]) = (0, 0),
            FatVersion::Fat32(_) => to_le_u16(((c >> 16) & 0xFFFF) as u16, &mut b[20..]),
        }
        time_write(&self.modified, &mut b[22..]);
        to_le_u16((c & 0xFFFF) as u16, &mut b[26..]);
        to_le_u32(self.size, &mut b[28..])
    }
    pub(super) fn write_lfn_entry(&self, lfn: &LongName, pos: u8, s: u8, b: &mut [u8]) {
        let (n, c) = (lfn.len(), self.name.checksum());
        b[0..DIR_SIZE].fill(0);
        b[0] = if pos == 0 { 0x40 } else { 0 } | (s - pos) as u8;
        b[0xB] = 0xF;
        b[0xD] = c;
        let (s, mut p) = ((s - 1 - pos) as usize * 0xD, 0);
        for v in s..cmp::min(s + 0xD, n) {
            b[LongName::pos_to_lfn(p)] = lfn.0[v];
            p += 1;
        }
        // Add NUL padding char.
        if p < 0xC {
            b[LongName::pos_to_lfn(p + 1)] = 0;
            p += 1;
        }
        // Fill remaining with 0xFF
        if p < 0xC {
            for x in LongName::pos_to_lfn(p)..DIR_SIZE {
                match x {
                    0 | 0xB | 0xC | 0xD | 0x1A | 0x1B => continue,
                    _ => (),
                }
                b[x] = 0xFF
            }
        }
    }
    pub(super) fn delete(&self, vol: &Volume<impl BlockDevice>, scratch: &mut Block) -> Result<(), DeviceError> {
        vol.dev.read_single(scratch, self.block)?;
        if self.offset as usize > Block::SIZE {
            return Err(DeviceError::BadData);
        }
        scratch[self.offset as usize] = 0xE5;
        if self.lfn == 0 {
            return vol.dev.write_single(scratch, self.block);
        }
        // NOTE(sf): We try to remove the long filename entries, but we're not
        //            gonna go back further than a whole block for simplicity.
        let n = self.lfn as u32 * DIR_SIZE as u32;
        if n > self.offset {
            return vol.dev.write_single(scratch, self.block);
        }
        let mut i = self.offset.saturating_sub(n) as usize;
        while i < self.offset as usize {
            scratch[i..i + DIR_SIZE].fill(0);
            i += DIR_SIZE;
        }
        vol.dev.write_single(scratch, self.block)?;
        if let Some(v) = self.cluster {
            vol.man.cluster_truncate(vol.dev, scratch, v)?;
        }
        Ok(())
    }
    #[inline(always)]
    pub(super) fn allocate(&mut self, vol: &Volume<impl BlockDevice>, scratch: &mut Block) -> Result<(), DeviceError> {
        self.cluster = Some(vol.man.cluster_allocate(vol.dev, scratch, None, false)?);
        Ok(())
    }
    #[inline]
    pub(super) fn sync(&self, dev: &Storage<impl BlockDevice>, scratch: &mut Block, f: &FatVersion) -> Result<(), DeviceError> {
        dev.read_single(scratch, self.block)?;
        if self.offset as usize > Block::SIZE {
            return Err(DeviceError::BadData);
        }
        self.write_entry(f, &mut scratch[self.offset as usize..]);
        dev.write_single(scratch, self.block)
    }
}
impl DirEntryFull {
    #[inline]
    pub(super) fn new() -> DirEntryFull {
        DirEntryFull {
            lfn:   Cache::lfn(),
            sum:   0u8,
            entry: DirEntry::new(0, 0),
        }
    }

    #[inline]
    pub fn name(&self) -> &str {
        if self.entry.lfn == 0 || self.lfn.is_empty() {
            self.entry.name.as_str()
        } else {
            self.lfn.as_str()
        }
    }
    #[inline(always)]
    pub fn longname(&self) -> &LongName {
        &self.lfn
    }
    #[inline(always)]
    pub fn is_name(&self, v: &str) -> bool {
        self.eq(v)
    }

    #[inline]
    pub(super) fn reset(&mut self) {
        self.lfn.reset();
        self.sum = 0;
        self.entry.lfn = 0;
    }
    #[inline(always)]
    pub(super) fn fill(&mut self, b: &[u8]) {
        self.sum = self.lfn.fill_lfn(b)
    }
    #[inline]
    pub(super) fn entry(&mut self) -> DirEntry {
        let mut n = DirEntry::new(0, 0);
        mem::swap(&mut self.entry, &mut n);
        n
    }
    pub(super) fn load(&mut self, f: &FatVersion, b: &[u8], block: u32, offset: u32) {
        let v = if matches!(f, FatVersion::Fat32(_)) {
            (le_u16(&b[20..]) as u32) << 16 | le_u16(&b[26..]) as u32
        } else {
            le_u16(&b[26..]) as u32
        };
        self.entry.lfn = self.lfn.lfn_size();
        self.entry.name.fill_inner(b);
        if self.sum != self.entry.name.checksum() {
            self.lfn.reset();
            self.entry.lfn = 0;
        }
        self.entry.block = block;
        self.entry.offset = offset;
        self.entry.size = le_u32(&b[28..]);
        self.entry.attrs = b[11];
        self.entry.created = time_read(le_u16(&b[16..]), le_u16(&b[14..]));
        self.entry.cluster = if v == 0 && b[11] & 0x10 == 0x10 { None } else { Some(v) };
        self.entry.modified = time_read(le_u16(&b[24..]), le_u16(&b[22..]));
    }
}
impl<'a, B: BlockDevice> File<'a, B> {
    #[inline(always)]
    pub(super) fn new(file: DirEntry, mode: u8, vol: &'a Volume<'a, B>) -> File<'a, B, Safe> {
        File {
            last: file.cluster_abs(),
            vol,
            file,
            mode,
            pos: 0u32,
            short: 0u32,
            _p: PhantomData,
        }
    }
}
impl<'a, B: BlockDevice> Reader<'a, B> {
    #[inline(always)]
    pub fn cursor(&self) -> usize {
        self.f.pos as usize
    }
    #[inline(always)]
    pub fn available(&self) -> usize {
        self.f.file.size.saturating_sub(self.f.pos) as usize
    }
    #[inline(always)]
    pub fn volume(&self) -> &Volume<'a, B> {
        self.f.vol
    }
    #[inline(always)]
    pub fn into_file(self) -> File<'a, B, Unsafe> {
        drop(self.buf);
        self.f
    }
    /// Similar to 'File.read' but does NOT re-read nearby chunks inside the
    /// same Block.
    ///
    /// This will hold the Cache lock until dropped.
    pub fn read(&mut self, b: &mut [u8]) -> Result<usize, DeviceError> {
        if b.is_empty() {
            return Ok(0);
        }
        let (mut p, t) = (0, b.len());
        let (d, mut c) = (&mut *self.buf, BlockCache::new());
        while p < t && self.f.pos < self.f.file.size {
            let (i, o, a) = match self.f.data(d, &mut c) {
                Err(DeviceError::EndOfFile) => return Ok(p),
                Err(e) => return Err(e),
                Ok(v) => v,
            };
            if self.bp == 0 || self.bp != i {
                // Only read if the block changed, to prevent double reads.
                // Speedup is 200%!
                self.f.vol.dev.read_single(d, i)?;
                self.bp = i;
            }
            let n = cmp::min(cmp::min(a, t - p), self.f.available());
            if n == 0 {
                break;
            }
            b[p..p + n].copy_from_slice(&d[o..o + n]);
            self.f.pos = self.f.pos.saturating_add(n as u32);
            p = p.saturating_add(n);
        }
        Ok(p)
    }
}
impl<'a, B: BlockDevice> Directory<'a, B> {
    #[inline(always)]
    pub(super) fn new(dir: DirEntry, vol: &'a Volume<'a, B>) -> Directory<'a, B> {
        Directory { vol, dir }
    }

    #[inline(always)]
    pub fn volume(&self) -> &Volume<'a, B> {
        self.vol
    }
    #[inline]
    pub fn delete(self, force: bool) -> Result<(), DeviceError> {
        let mut b = Cache::block_a();
        Directory::delete_inner(self.vol, &self.dir, &mut b, force)
    }
    #[inline(always)]
    pub fn list(&self) -> Result<DirectoryIndex<'a, B>, DeviceError> {
        // Safe as we're the entry and valid.
        unsafe { self.vol.list_entry(Some(&self.dir)) }
    }
    #[inline]
    pub fn file(&'a self, name: impl AsRef<str>, mode: u8) -> Result<File<'a, B>, DeviceError> {
        // Safe as we're the entry and valid.
        unsafe { self.vol.file_entry(name, Some(&self.dir), mode) }
    }
    #[inline]
    pub fn dir(&'a self, name: impl AsRef<str>, create: bool) -> Result<Directory<'a, B>, DeviceError> {
        // Safe as we're the entry and valid.
        unsafe { self.vol.dir_entry(name, Some(&self.dir), create) }
    }

    #[inline(always)]
    pub(super) fn entry(&self) -> &DirEntry {
        &self.dir
    }
    #[inline(always)]
    pub(super) fn cluster(&self) -> Cluster {
        self.dir.cluster
    }

    fn delete_inner(vol: &'a Volume<'a, B>, dir: &DirEntry, scratch: &mut Block, force: bool) -> Result<(), DeviceError> {
        // NOTE(sf): This works at deleting recursively for directories. However,
        //           it sometimes can be buggy just due to how the FAT FS is
        //           structured.
        for e in unsafe { vol.list_entry(Some(dir))? } {
            if !force {
                return Err(DeviceError::NonEmptyDirectory);
            }
            let v = e?;
            if v.is_root_or_parent() {
                // NOTE(sf): Usually this is the start of the Block, so we
                //           should be good to stop here to prevent deleting
                //           other things.
                break;
            }
            if v.is_directory() {
                Directory::delete_inner(vol, &v, scratch, force)?;
            } else {
                v.delete(vol, scratch)?;
            }
        }
        dir.delete(vol, scratch)?;
        vol.man.cluster_truncate(vol.dev, scratch, dir.cluster_abs())
    }
}
impl<'a, B: BlockDevice> File<'a, B, Safe> {
    #[inline(always)]
    /// Remove the locking requirement for file Read/Writes.
    ///
    /// Use only if sure that reads/writes will not happen on the same or
    /// multiple files by different cores at the same.
    pub unsafe fn into_unsafe(self) -> File<'a, B, Unsafe> {
        unsafe { transmute(self) }
    }
    #[inline(always)]
    /// Transform the File into a high-speed Reader with better caching
    /// and locking mechanisms.
    pub unsafe fn into_reader(self) -> Result<Reader<'a, B>, DeviceError> {
        if !self.is_readable() {
            return Err(DeviceError::NotReadable);
        }
        Ok(Reader {
            f:   unsafe { self.into_unsafe() },
            bp:  0u32,
            buf: Cache::block_a(),
        })
    }
}
impl<'a, B: BlockDevice> File<'a, B, Unsafe> {
    #[inline(always)]
    pub fn into_safe(self) -> File<'a, B, Safe> {
        unsafe { transmute(self) }
    }
}
impl<'a, B: BlockDevice, S: FileSync> File<'a, B, S> {
    #[inline(always)]
    pub fn cursor(&self) -> usize {
        self.pos as usize
    }
    #[inline(always)]
    pub fn is_dirty(&self) -> bool {
        self.mode & 0x80 != 0
    }
    #[inline(always)]
    pub fn available(&self) -> usize {
        self.file.size.saturating_sub(self.pos) as usize
    }
    #[inline(always)]
    pub fn is_readable(&self) -> bool {
        self.mode >> 4 == 0 || self.mode & Mode::READ != 0
    }
    #[inline(always)]
    pub fn is_writeable(&self) -> bool {
        self.mode & Mode::WRITE != 0
    }
    #[inline(always)]
    pub fn is_allocated(&self) -> bool {
        self.file.cluster.is_some_and(|v| v > 2)
    }
    #[inline(always)]
    pub fn volume(&self) -> &Volume<'a, B> {
        self.vol
    }
    #[inline]
    pub fn delete(self) -> Result<(), DeviceError> {
        let mut b = S::cache();
        self.file.delete(self.vol, &mut b)
    }
    #[inline(always)]
    pub fn close(mut self) -> Result<(), DeviceError> {
        self.flush()
    }
    #[inline]
    pub fn flush(&mut self) -> Result<(), DeviceError> {
        if !self.is_dirty() {
            return Ok(());
        }
        let mut b = S::cache();
        self.vol.man.sync(self.vol.dev, &mut b)?;
        self.file.sync(self.vol.dev, &mut b, &self.vol.man.ver)
    }
    pub fn write(&mut self, b: &[u8]) -> Result<usize, DeviceError> {
        if !self.is_writeable() {
            return Err(DeviceError::NotWritable);
        }
        if b.is_empty() {
            return Ok(0);
        }
        self.mode |= 0x80;
        let mut d = S::cache();
        if !self.is_allocated() {
            self.file.cluster = Some(self.vol.man.cluster_allocate(self.vol.dev, &mut d, None, false)?);
            d.clear();
        }
        let c = self.file.cluster.ok_or(DeviceError::WriteError)?;
        if self.last < c {
            (self.last, self.short) = (c, 0);
        }
        let t = cmp::min(b.len(), (FILE_MAX_SIZE - self.pos) as usize);
        let (mut c, mut p) = (BlockCache::new(), 0);
        while p < t {
            let (i, o, a) = match self.data(&mut d, &mut c) {
                Ok(v) => v,
                Err(DeviceError::EndOfFile) => {
                    self.vol
                        .man
                        .cluster_allocate(self.vol.dev, &mut d, self.cluster(), false)
                        .map_err(|_| DeviceError::NoSpace)?;
                    self.data(&mut d, &mut c).map_err(|_| DeviceError::WriteError)?
                },
                Err(e) => return Err(e),
            };
            let n = cmp::min(a, t.saturating_sub(p));
            if n == 0 {
                break;
            }
            if o != 0 {
                self.vol.dev.read_single(&mut d, i)?;
            }
            d[o..o + n].copy_from_slice(&b[p..p + n]);
            self.vol.dev.write_single(&d, i)?;
            self.file.size = self.file.size.saturating_add(n as u32);
            self.pos = self.pos.saturating_add(n as u32);
            p = p.saturating_add(n);
        }
        self.file.attrs |= 0x20;
        Ok(p)
    }
    /// Does not save the file and keeps the current Cluster intact.
    /// To fully truncate a File entry, it must be opened with 'Mode::TRUNCATE'.
    pub fn truncate(&mut self, pos: usize) -> Result<(), DeviceError> {
        let i = pos.try_into().map_err(|_| DeviceError::Overflow)?;
        if i > self.file.size {
            return Err(DeviceError::InvalidIndex);
        }
        self.file.size = i;
        if self.file.size > self.pos {
            self.pos = i;
        }
        Ok(())
    }
    pub fn read(&mut self, b: &mut [u8]) -> Result<usize, DeviceError> {
        if !self.is_readable() {
            return Err(DeviceError::NotReadable);
        }
        if b.is_empty() {
            return Ok(0);
        }
        let (mut p, t) = (0, b.len());
        let (mut d, mut c) = (S::cache(), BlockCache::new());
        while p < t && self.pos < self.file.size {
            let (i, o, a) = match self.data(&mut d, &mut c) {
                Err(DeviceError::EndOfFile) => return Ok(p),
                Err(e) => return Err(e),
                Ok(v) => v,
            };
            self.vol.dev.read_single(&mut d, i)?;
            let n = cmp::min(cmp::min(a, t - p), self.available());
            if n == 0 {
                break;
            }
            b[p..p + n].copy_from_slice(&d[o..o + n]);
            self.pos = self.pos.saturating_add(n as u32);
            p = p.saturating_add(n);
        }
        Ok(p)
    }

    #[inline(always)]
    pub(super) fn zero(&mut self) {
        self.file.size = 0
    }
    #[inline(always)]
    pub(super) fn seek_to_end(&mut self) {
        self.pos = self.file.size
    }

    fn data(&mut self, scratch: &mut Block, cache: &mut BlockCache) -> Result<(u32, usize, usize), DeviceError> {
        if self.pos < self.short {
            (self.short, self.last) = (0, self.cluster_abs());
        }
        let c = self.vol.man.blocks.bytes_per_cluster();
        let n = self.pos.saturating_sub(self.short);
        cache.clear();
        for _ in 0..(n / c) {
            self.last = self
                .vol
                .man
                .cluster_next(self.vol.dev, scratch, cache, self.last)?
                .ok_or_else(|| DeviceError::EndOfFile)?;
            self.short += c;
        }
        let i = self.vol.man.block_pos_at(self.last) + (self.pos.saturating_sub(self.short) / Block::SIZE as u32);
        let o = self.pos as usize % Block::SIZE;
        Ok((i, o, Block::SIZE - o))
    }
}

impl<B: BlockDevice, S: FileSync> Drop for File<'_, B, S> {
    #[inline(always)]
    fn drop(&mut self) {
        ignore_error!(self.flush());
    }
}
impl<B: BlockDevice, S: FileSync> Deref for File<'_, B, S> {
    type Target = DirEntry;

    #[inline(always)]
    fn deref(&self) -> &DirEntry {
        &self.file
    }
}
impl<B: BlockDevice, S: FileSync> Seek<DeviceError> for File<'_, B, S> {
    fn seek(&mut self, s: SeekFrom) -> Result<u64, Error> {
        let r = match s {
            SeekFrom::End(v) => {
                if v > 0 {
                    return Err(Error::InvalidIndex);
                }
                self.pos
                    .saturating_sub(v.unsigned_abs().try_into().map_err(|_| DeviceError::Overflow)?)
            },
            SeekFrom::Start(v) => v.try_into().map_err(|_| DeviceError::Overflow)?,
            SeekFrom::Current(v) if v > 0 => self.pos.saturating_add(v.try_into().map_err(|_| DeviceError::Overflow)?),
            SeekFrom::Current(v) => self
                .pos
                .saturating_sub(v.unsigned_abs().try_into().map_err(|_| DeviceError::Overflow)?),
        };
        if r > self.file.size {
            return Err(Error::InvalidIndex);
        }
        self.pos = r;
        Ok(self.pos as u64)
    }
}
impl<B: BlockDevice, S: FileSync> Read<DeviceError> for File<'_, B, S> {
    #[inline(always)]
    fn read(&mut self, b: &mut [u8]) -> Result<usize, Error> {
        Ok(self.read(b)?)
    }
}
impl<B: BlockDevice, S: FileSync> Write<DeviceError> for File<'_, B, S> {
    #[inline(always)]
    fn flush(&mut self) -> Result<(), Error> {
        Ok(self.flush()?)
    }
    #[inline(always)]
    fn write(&mut self, b: &[u8]) -> Result<usize, Error> {
        Ok(self.write(b)?)
    }
}

impl<B: BlockDevice> Deref for Reader<'_, B> {
    type Target = DirEntry;

    #[inline(always)]
    fn deref(&self) -> &DirEntry {
        &self.f.file
    }
}
impl<B: BlockDevice> Seek<DeviceError> for Reader<'_, B> {
    fn seek(&mut self, s: SeekFrom) -> Result<u64, Error> {
        self.f.seek(s)
    }
}
impl<B: BlockDevice> Read<DeviceError> for Reader<'_, B> {
    #[inline(always)]
    fn read(&mut self, b: &mut [u8]) -> Result<usize, Error> {
        Ok(self.read(b)?)
    }
}

impl PartialEq<str> for DirEntry {
    #[inline(always)]
    fn eq(&self, other: &str) -> bool {
        self.eq(other.as_bytes())
    }
}
impl PartialEq<[u8]> for DirEntry {
    #[inline(always)]
    fn eq(&self, other: &[u8]) -> bool {
        self.name.eq(other)
    }
}

impl Deref for DirEntryFull {
    type Target = DirEntry;

    #[inline(always)]
    fn deref(&self) -> &DirEntry {
        &self.entry
    }
}
impl PartialEq<str> for DirEntryFull {
    #[inline(always)]
    fn eq(&self, other: &str) -> bool {
        self.eq(other.as_bytes())
    }
}
impl PartialEq<[u8]> for DirEntryFull {
    #[inline]
    fn eq(&self, other: &[u8]) -> bool {
        if self.entry.lfn == 0 { self.entry.name.eq(other) } else { self.lfn.eq(other) }
    }
}

impl FileSync for Safe {
    #[inline(always)]
    fn cache() -> BlockPtr {
        Cache::block_a()
    }
}
impl FileSync for Unsafe {
    #[inline(always)]
    fn cache() -> BlockPtr {
        unsafe { Cache::block_a_nolock() }
    }
}

impl<B: BlockDevice> Deref for Directory<'_, B> {
    type Target = DirEntry;

    #[inline(always)]
    fn deref(&self) -> &DirEntry {
        &self.dir
    }
}

#[inline]
fn time_read(a: u16, b: u16) -> Time {
    Time {
        year:    ((a >> 0x9) + 0xA) + 0x7B2u16,
        month:   (((a >> 0x5) & 0xF) as u8 + 1).into(),
        day:     (a & 0x1F) as u8,
        hours:   ((b >> 0xB) & 0x1F) as u8,
        mins:    ((b >> 0x5) & 0x3F) as u8,
        secs:    ((b << 0x1) & 0x3F) as u8,
        weekday: Weekday::None,
    }
}
#[inline]
fn time_write(t: &Time, b: &mut [u8]) {
    to_le_u16(
        (((t.hours as u16) << 11) & 0xF800) | (((t.mins as u16) << 5) & 0x7E0) | (((t.secs as u16) / 2) & 0x1F),
        b,
    );
    to_le_u16(
        (((t.year.saturating_sub(0x7B2)).saturating_sub(10) << 9) & 0xFE00) | (((t.month as u16 + 1) << 5) & 0x1E0) | ((t.day as u16 + 1) & 0x1F),
        &mut b[2..],
    )
}

pub mod state {
    pub struct Safe;
    pub struct Unsafe;
}
