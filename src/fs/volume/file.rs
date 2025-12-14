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

use core::cmp::{Ord, PartialEq, PartialOrd};
use core::convert::{AsRef, From, TryInto};
use core::marker::PhantomData;
use core::mem::{drop, replace, transmute};
use core::ops::{Deref, DerefMut, Drop};
use core::option::Option::{None, Some};
use core::ptr::{NonNull, copy_nonoverlapping, write_bytes};
use core::result::Result::{self, Err, Ok};

use rpsp::io::{Read, Seek, SeekFrom, Write};
use rpsp::time::{Month, Time, Weekday};

use crate::fs::state::{Safe, Unsafe};
use crate::fs::volume::to_lfn;
use crate::fs::{Block, BlockCache, BlockDevice, BlockPtr, Cache, Cluster, ClusterIndex, DevResult, DeviceError, DirectoryIndex, Error, LongName, LongNamePtr, ShortName, Storage, Volume};
use crate::{Slice, SliceMut};

const FILE_MAX_SIZE: u32 = 0xFFFFFFFFu32;

pub enum Mode {}

pub struct DirEntry {
    lfn:      u8,
    name:     ShortName,
    size:     u32,
    attrs:    u8,
    block:    u32,
    offset:   u32,
    cluster:  Cluster,
    created:  Time,
    modified: Time,
}
pub struct DirEntryFull {
    lfn:   LongNamePtr,
    sum:   u8,
    entry: DirEntry,
}
pub struct DirEntryPtr<'a> {
    ptr: NonNull<DirEntryFull>,
    _p:  PhantomData<&'a DirEntryFull>,
}
pub struct Reader<'a, B: BlockDevice> {
    f:   File<'a, B, Unsafe>,
    bp:  u32,
    buf: BlockPtr,
}
pub struct Directory<'a, B: BlockDevice> {
    dir: DirEntry,
    vol: &'a Volume<'a, B>,
}
pub struct File<'a, B: BlockDevice, S: FileSync = Safe> {
    pos:   u32,
    vol:   &'a Volume<'a, B>,
    file:  DirEntry,
    last:  ClusterIndex,
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

    #[inline]
    pub(super) fn is_create(m: u8) -> bool {
        m & Mode::CREATE != 0 && (m & Mode::WRITE != 0 || m & Mode::APPEND != 0)
    }
    #[inline]
    pub(super) fn is_mode_valid(m: u8) -> bool {
        if unsafe { m.unchecked_shr(4) } > 0 || m & (Mode::TRUNCATE | Mode::APPEND) == 0x12 || m == Mode::CREATE {
            false
        } else {
            true
        }
    }
}
impl DirEntry {
    pub const SIZE: usize = 32usize;
    pub const SIZE_PER_BLOCK: u32 = (Block::SIZE / DirEntry::SIZE) as u32;

    #[inline]
    pub(super) const fn new_root() -> DirEntry {
        DirEntry {
            lfn:      0,
            name:     ShortName::empty(),
            size:     0u32,
            block:    0u32,
            attrs:    0x10u8,
            offset:   0u32,
            cluster:  None,
            created:  Time::empty(),
            modified: Time::empty(),
        }
    }
    #[inline]
    pub(super) const fn new(attrs: u8, lfn: u8) -> DirEntry {
        DirEntry {
            lfn,
            attrs,
            name: ShortName::empty(),
            size: 0u32,
            block: 0u32,
            offset: 0u32,
            cluster: None,
            created: Time::empty(),
            modified: Time::empty(),
        }
    }
    #[inline]
    pub(super) const fn new_self(parent: &DirEntry, block: u32) -> DirEntry {
        DirEntry {
            block,
            lfn: 0u8,
            name: ShortName::SELF,
            size: 0u32,
            attrs: 0x10u8,
            offset: 0u32,
            cluster: parent.cluster,
            created: Time::empty(),
            modified: Time::empty(),
        }
    }
    #[inline]
    pub(super) const fn new_parent(cluster: Cluster, block: u32) -> DirEntry {
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

    #[inline]
    pub fn size(&self) -> u32 {
        self.size
    }
    #[inline]
    pub fn name(&self) -> &str {
        self.name.as_str()
    }
    #[inline]
    pub fn offset(&self) -> u32 {
        self.offset
    }
    #[inline]
    pub fn is_file(&self) -> bool {
        !self.is_directory() && self.attrs & 0x8 == 0
    }
    #[inline]
    pub fn created(&self) -> &Time {
        &self.created
    }
    #[inline]
    pub fn attributes(&self) -> u8 {
        self.attrs
    }
    #[inline]
    pub fn is_hidden(&self) -> bool {
        self.attrs & 0x2 != 0 || self.name.as_raw().read_u8(0) == b'.'
    }
    #[inline]
    pub fn modified(&self) -> &Time {
        &self.modified
    }
    #[inline]
    pub fn cluster(&self) -> Cluster {
        self.cluster
    }
    #[inline]
    pub fn is_directory(&self) -> bool {
        self.attrs & 0x10 == 0x10
    }
    #[inline]
    pub fn filename(&self) -> &ShortName {
        &self.name
    }
    #[inline]
    pub fn shortname(&self) -> &ShortName {
        &self.name
    }
    #[inline]
    pub fn set_created(&mut self, t: Time) {
        self.created = t
    }
    #[inline]
    pub fn set_attributes(&mut self, a: u8) {
        self.attrs = a
    }
    #[inline]
    pub fn set_modified(&mut self, t: Time) {
        self.modified = t
    }
    #[inline]
    pub fn into_dir<'a, B: BlockDevice>(self, vol: &'a Volume<B>) -> DevResult<Directory<'a, B>> {
        if !self.is_directory() {
            return Err(DeviceError::NotADirectory);
        }
        Ok(Directory::new(self, vol))
    }
    #[inline]
    pub fn into_file<'a, B: BlockDevice>(self, vol: &'a Volume<B>, mode: u8) -> DevResult<File<'a, B>> {
        if !Mode::is_mode_valid(mode) {
            return Err(DeviceError::InvalidOptions);
        }
        if !self.is_file() {
            return Err(DeviceError::NotAFile);
        }
        Ok(File::new(self, mode, vol))
    }

    #[inline]
    pub(super) fn index(&self) -> ClusterIndex {
        match self.cluster {
            Some(v) => v,
            None => ClusterIndex::EMPTY,
        }
    }
    #[inline]
    pub(super) fn fill_name(&mut self, v: &[u8]) {
        self.name.fill(v);
    }
    #[inline]
    pub(super) fn is_root_or_parent(&self) -> bool {
        match &self.cluster {
            Some(v) if v.is_empty() => true,
            None => true,
            _ => self.name.is_empty() || self.name.is_self() || self.name.is_parent(),
        }
    }
    #[inline]
    pub(super) fn prepare(&mut self, b: u32, o: usize) {
        (self.block, self.offset) = (b, o as u32)
    }
    pub(super) fn write_entry(&self, f: bool, mut b: &mut [u8]) {
        b.write_from(0, self.name.as_raw());
        b.write_u8(11, self.attrs);
        // We set these to zero as we don't need them
        b.write_u8(12, 0); // WinNT bit ??
        b.write_u8(13, 0); // Created Time milliseconds
        b.write_u16(18, 0); // Last Access Time
        time_write(&self.created, 14, b);
        let c = self.index();
        if f {
            b.write_u16(20, unsafe { (c.unchecked_shr(16) & 0xFFFF) as u16 });
        } else {
            b.write_u16(20, 0);
        }
        time_write(&self.modified, 22, b);
        b.write_u16(26, (*c & 0xFFFF) as u16);
        b.write_u32(28, self.size);
    }
    pub(super) fn write_lfn_entry(&self, lfn: &LongName, pos: u8, s: u8, mut b: &mut [u8]) {
        let (n, c) = (lfn.len(), self.name.checksum());
        // Clear out data.
        unsafe { write_bytes(b.as_mut_ptr(), 0, DirEntry::SIZE) };
        b.write_u8(0, if pos == 0 { 0x40 } else { 0 } | (s - pos) as u8);
        b.write_u8(11, 0xF);
        b.write_u8(12, 0);
        b.write_u8(13, c);
        b.write_u16(26, 0);
        let (s, mut p) = ((s - 1 - pos) as usize * 13, 0);
        for v in s..(s + 0xD).min(n) {
            b.write_u8(to_lfn(p), lfn.as_raw().read_u8(v));
            p += 1;
        }
        // Add NULL padding char.
        if p < 12 {
            b.write_u8(to_lfn(p + 1), 0);
            p += 1;
        }
        // Fill remaining with 0xFF
        if p < 12 {
            for x in to_lfn(p)..DirEntry::SIZE {
                match x {
                    0 | 0xB | 0xC | 0xD | 0x1A | 0x1B => (), // Skip these spots
                    _ => b.write_u8(x, 0xFF),
                }
            }
        }
    }
    pub(super) fn delete<B: BlockDevice>(&self, vol: &Volume<B>, t: &mut Block) -> DevResult<()> {
        let _ = vol.dev.read_single(t, self.block)?;
        if self.offset as usize > Block::SIZE {
            return Err(DeviceError::BadData);
        }
        t.write_u8(self.offset as usize, 0xE5);
        if self.lfn == 0 {
            return vol.dev.write_single(t, self.block);
        }
        // NOTE(sf): We try to remove the long filename entries, but we're not
        //           gonna go back further than a whole block for simplicity.
        let n = self.lfn as u32 * DirEntry::SIZE as u32;
        if n > self.offset {
            return vol.dev.write_single(t, self.block);
        }
        let mut i = self.offset.saturating_sub(n) as usize;
        while i < self.offset as usize {
            unsafe { write_bytes(t.as_mut_ptr().add(i), 0, DirEntry::SIZE) };
            i += DirEntry::SIZE;
        }
        let _ = vol.dev.write_single(t, self.block)?;
        if let Some(v) = self.cluster {
            let _ = vol.truncate(t, v)?;
        }
        Ok(())
    }
    #[inline]
    pub(super) fn allocate<B: BlockDevice>(&mut self, vol: &Volume<B>, t: &mut Block) -> DevResult<()> {
        self.cluster = Some(vol.allocate(t, None, false)?);
        Ok(())
    }
    #[inline]
    pub(super) fn sync(&self, dev: &Storage<impl BlockDevice>, t: &mut Block, f: bool) -> DevResult<()> {
        let _ = dev.read_single(t, self.block)?;
        if self.offset as usize > Block::SIZE {
            return Err(DeviceError::BadData);
        }
        self.write_entry(f, unsafe { t.get_unchecked_mut(self.offset as usize..) });
        dev.write_single(t, self.block)
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
    #[inline]
    pub fn longname(&self) -> &LongName {
        &self.lfn
    }
    #[inline]
    pub fn shortname(&self) -> &ShortName {
        &self.entry.name
    }
    #[inline]
    pub fn is_name(&self, v: &str) -> bool {
        self.eq(v)
    }
    #[inline]
    pub fn into_entry(&mut self) -> DirEntry {
        self.entry()
    }
    #[inline]
    pub fn into_dir<'a, B: BlockDevice>(&mut self, vol: &'a Volume<B>) -> DevResult<Directory<'a, B>> {
        if !self.is_directory() {
            return Err(DeviceError::NotADirectory);
        }
        Ok(Directory::new(self.entry(), vol))
    }
    #[inline]
    pub fn into_file<'a, B: BlockDevice>(&mut self, vol: &'a Volume<B>, mode: u8) -> DevResult<File<'a, B>> {
        if !Mode::is_mode_valid(mode) {
            return Err(DeviceError::InvalidOptions);
        }
        if !self.is_file() {
            return Err(DeviceError::NotAFile);
        }
        Ok(File::new(self.entry(), mode, vol))
    }

    #[inline]
    pub(super) fn reset(&mut self) {
        self.lfn.reset();
        (self.sum, self.entry.lfn) = (0, 0);
    }
    #[inline]
    pub(super) fn fill(&mut self, b: &[u8]) {
        self.sum = self.lfn.lfn(b)
    }
    #[inline]
    pub(super) fn entry(&mut self) -> DirEntry {
        replace(&mut self.entry, DirEntry::new(0, 0))
    }
    #[inline]
    pub(super) fn ptr<'a>(&mut self) -> DirEntryPtr<'a> {
        DirEntryPtr {
            ptr: unsafe { NonNull::new_unchecked(self) },
            _p:  PhantomData,
        }
    }
    pub(super) fn load(&mut self, f: bool, b: &[u8], block: u32, offset: u32) {
        let v = if f {
            ClusterIndex::new(unsafe { (b.read_u16(20) as u32).unchecked_shl(16) | (b.read_u16(26) as u32) })
        } else {
            ClusterIndex::new(b.read_u16(26) as u32)
        };
        self.entry.lfn = self.lfn.lfn_size();
        self.entry.name.fill_inner(b);
        if self.sum != self.entry.name.checksum() {
            self.lfn.reset();
            self.entry.lfn = 0;
        }
        self.entry.block = block;
        self.entry.offset = offset;
        self.entry.size = b.read_u32(28);
        self.entry.attrs = b.read_u8(11);
        self.entry.created = time_read(b.read_u16(16), b.read_u16(14));
        self.entry.modified = time_read(b.read_u16(24), b.read_u16(22));
        self.entry.cluster = if v.is_none() && self.entry.attrs & 0x10 == 0x10 { None } else { v };
    }
}
impl<'a, B: BlockDevice> File<'a, B> {
    #[inline]
    pub(super) fn new(file: DirEntry, mode: u8, vol: &'a Volume<'a, B>) -> File<'a, B, Safe> {
        File {
            last: file.index(),
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
    #[inline]
    pub fn into_file(self) -> File<'a, B, Unsafe> {
        drop(self.buf);
        self.f
    }
    /// Similar to 'File.read' but does NOT re-read nearby chunks inside the
    /// same Block when calling 'read' multiple times.
    ///
    /// This will hold the Cache lock until dropped.
    pub fn read(&mut self, b: &mut [u8]) -> DevResult<usize> {
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
            if self.bp != i {
                // Only read if the block changed, to prevent double reads.
                // Speedup is 200%!
                let _ = self.f.vol.dev.read_single(d, i)?;
                self.bp = i; // Don't re-read the same Block.
            }
            let n = a.min(t.saturating_sub(p)).min(self.f.available());
            if n == 0 {
                break;
            }
            unsafe { copy_nonoverlapping(d.as_ptr().add(o), b.as_mut_ptr().add(p), n) };
            (self.f.pos, p) = (self.f.pos.saturating_add(n as u32), p.saturating_add(n));
        }
        Ok(p)
    }
}
impl<'a, B: BlockDevice> Directory<'a, B> {
    #[inline]
    pub(super) fn new(dir: DirEntry, vol: &'a Volume<'a, B>) -> Directory<'a, B> {
        Directory { vol, dir }
    }

    #[inline]
    pub fn volume(&self) -> &Volume<'a, B> {
        self.vol
    }
    /// Set `force` to true to recursively delete this Directory and it's
    /// contents. Otherwise a `NonEmptyDirectory` error will be returned for
    /// Directories that contain Files or other Directories.
    #[inline]
    pub fn delete(self, force: bool) -> DevResult<()> {
        let mut b = Cache::block_a();
        delete(self.vol, &self.dir, &mut b, force)
    }
    #[inline]
    pub fn list(&self) -> DevResult<DirectoryIndex<'a, B>> {
        // Safe as we're the entry and valid.
        unsafe { self.vol.list_entry(Some(&self.dir)) }
    }
    #[inline]
    pub fn file(&'a self, name: impl AsRef<str>, mode: u8) -> DevResult<File<'a, B>> {
        // Safe as we're the entry and valid.
        unsafe { self.vol.file_entry(name, Some(&self.dir), mode) }
    }
    #[inline]
    pub fn dir(&'a self, name: impl AsRef<str>, create: bool) -> DevResult<Directory<'a, B>> {
        // Safe as we're the entry and valid.
        unsafe { self.vol.dir_entry(name, Some(&self.dir), create) }
    }

    #[inline]
    pub(super) fn cluster(&self) -> Cluster {
        self.dir.cluster
    }
}
impl<'a, B: BlockDevice> File<'a, B, Safe> {
    /// Remove the locking requirement for file Read/Writes.
    ///
    /// Use only if sure that reads/writes will not happen on the same or
    /// multiple files by different cores at the same!
    #[inline]
    pub unsafe fn into_unsafe(self) -> File<'a, B, Unsafe> {
        unsafe { transmute(self) }
    }
    /// Transform the File into a high-speed Reader with better caching
    /// and locking mechanisms.
    #[inline]
    pub unsafe fn into_reader(self) -> DevResult<Reader<'a, B>> {
        if !self.is_readable() {
            return Err(DeviceError::NotReadable);
        }
        Ok(Reader {
            f:   unsafe { self.into_unsafe() },
            bp:  u32::MAX,
            buf: Cache::block_a(),
        })
    }
}
impl<'a, B: BlockDevice> File<'a, B, Unsafe> {
    #[inline]
    pub fn into_safe(self) -> File<'a, B, Safe> {
        unsafe { transmute(self) }
    }
}
impl<'a, B: BlockDevice, S: FileSync> File<'a, B, S> {
    #[inline]
    pub fn mode(&self) -> u8 {
        self.mode
    }
    #[inline]
    pub fn cursor(&self) -> usize {
        self.pos as usize
    }
    #[inline]
    pub fn is_dirty(&self) -> bool {
        self.mode & 0x80 != 0
    }
    #[inline]
    pub fn available(&self) -> usize {
        self.file.size.saturating_sub(self.pos) as usize
    }
    #[inline]
    pub fn is_readable(&self) -> bool {
        unsafe { self.mode.unchecked_shr(4) == 0 || self.mode & Mode::READ != 0 }
    }
    #[inline]
    pub fn is_writeable(&self) -> bool {
        self.mode & Mode::WRITE != 0
    }
    #[inline]
    pub fn is_allocated(&self) -> bool {
        self.file.cluster.is_some_and(|v| v.is_valid())
    }
    #[inline]
    pub fn delete(self) -> DevResult<()> {
        let mut b = S::cache();
        self.file.delete(self.vol, &mut b)
    }
    #[inline]
    pub fn volume(&self) -> &Volume<'a, B> {
        &self.vol
    }
    #[inline]
    pub fn close(mut self) -> DevResult<()> {
        self.flush()
    }
    #[inline]
    pub fn flush(&mut self) -> DevResult<()> {
        if !self.is_dirty() {
            return Ok(());
        }
        let mut b = S::cache();
        let _ = self.vol.sync(&mut b)?;
        self.file.sync(self.vol.dev, &mut b, self.vol.ver.is_fat32())
    }
    pub fn write(&mut self, b: &[u8]) -> DevResult<usize> {
        if !self.is_writeable() {
            return Err(DeviceError::NotWritable);
        }
        if b.is_empty() {
            return Ok(0);
        }
        self.mode |= 0x80;
        let mut d = S::cache();
        if !self.is_allocated() {
            self.file.cluster = Some(self.vol.allocate(&mut d, None, false)?);
            d.clear();
        }
        let c = self.file.cluster.ok_or(DeviceError::Write)?;
        if self.last.lt(&c) {
            (self.last, self.short) = (c, 0);
        }
        let t = b.len().min((FILE_MAX_SIZE - self.pos) as usize);
        let (mut c, mut p, mut l) = (BlockCache::new(), 0usize, u32::MAX);
        while p < t {
            let (i, o, a) = match self.data(&mut d, &mut c) {
                Ok(v) => v,
                Err(DeviceError::EndOfFile) => {
                    let _ = self.vol.allocate(&mut d, self.file.cluster, false)?;
                    self.data(&mut d, &mut c).or(Err(DeviceError::Write))?
                },
                Err(e) => return Err(e),
            };
            let n = a.min(t.saturating_sub(p));
            if n == 0 {
                break;
            }
            if o != 0 && l != i {
                let _ = self.vol.dev.read_single(&mut d, i)?;
                l = i; // Don't re-read the same Block.
            }
            unsafe { copy_nonoverlapping(b.as_ptr().add(p), d.as_mut_ptr().add(o), n) };
            let _ = self.vol.dev.write_single(&d, i)?;
            self.pos = self.pos.saturating_add(n as u32);
            self.file.size = self.file.size.saturating_add(n as u32);
            p = p.saturating_add(n);
        }
        self.file.attrs |= 0x20;
        Ok(p)
    }
    /// Does not save the file and keeps the current Cluster intact.
    /// To fully truncate a File entry, it must be opened with 'Mode::TRUNCATE'.
    #[inline]
    pub fn truncate(&mut self, pos: usize) -> DevResult<()> {
        let i = pos.try_into().or(Err(DeviceError::Overflow))?;
        if i > self.file.size {
            return Err(DeviceError::InvalidIndex);
        }
        self.file.size = i;
        if self.file.size > self.pos {
            self.pos = i;
        }
        Ok(())
    }
    pub fn read(&mut self, b: &mut [u8]) -> DevResult<usize> {
        if !self.is_readable() {
            return Err(DeviceError::NotReadable);
        }
        if b.is_empty() {
            return Ok(0);
        }
        let (mut p, t, mut l) = (0usize, b.len(), u32::MAX);
        let (mut d, mut c) = (S::cache(), BlockCache::new());
        while p < t && self.pos < self.file.size {
            let (i, o, a) = match self.data(&mut d, &mut c) {
                Err(DeviceError::EndOfFile) => return Ok(p),
                Err(e) => return Err(e),
                Ok(v) => v,
            };
            if i != l {
                let _ = self.vol.dev.read_single(&mut d, i)?;
                l = i; // Don't re-read the same Block.
            }
            let n = a.min(t.saturating_sub(p)).min(self.available());
            if n == 0 {
                break;
            }
            unsafe { copy_nonoverlapping(d.as_ptr().add(o), b.as_mut_ptr().add(p), n) };
            (self.pos, p) = (self.pos.saturating_add(n as u32), p.saturating_add(n));
        }
        Ok(p)
    }

    #[inline]
    pub(super) fn zero(&mut self) {
        self.file.size = 0
    }
    #[inline]
    pub(super) fn seek_to_end(&mut self) {
        self.pos = self.file.size
    }

    fn data(&mut self, scratch: &mut Block, cache: &mut BlockCache) -> DevResult<(u32, usize, usize)> {
        if self.pos < self.short {
            (self.short, self.last) = (0, self.index());
        }
        let c = self.vol.block.bytes();
        let n = self.pos.saturating_sub(self.short);
        cache.clear();
        for _ in 0..(n / c) {
            self.last = self.vol.next(scratch, cache, self.last)?.ok_or(DeviceError::EndOfFile)?;
            self.short += c;
        }
        let i = self.vol.block_pos_at(self.last) + (self.pos.saturating_sub(self.short) / Block::SIZE as u32);
        let o = self.pos as usize % Block::SIZE;
        Ok((i, o, Block::SIZE - o))
    }
}

impl<B: BlockDevice, S: FileSync> Drop for File<'_, B, S> {
    #[inline]
    fn drop(&mut self) {
        let _ = self.flush();
    }
}
impl<B: BlockDevice, S: FileSync> Deref for File<'_, B, S> {
    type Target = DirEntry;

    #[inline]
    fn deref(&self) -> &DirEntry {
        &self.file
    }
}
impl<B: BlockDevice, S: FileSync> Seek<DeviceError> for File<'_, B, S> {
    fn seek(&mut self, s: SeekFrom) -> Result<u64, Error> {
        let r = match s {
            SeekFrom::End(v) if v > 0 => return Err(Error::InvalidIndex),
            SeekFrom::End(v) => self
                .pos
                .saturating_sub(v.unsigned_abs().try_into().or(Err(DeviceError::Overflow))?),
            SeekFrom::Start(v) => v.try_into().or(Err(DeviceError::Overflow))?,
            SeekFrom::Current(v) if v > 0 => self.pos.saturating_add(v.try_into().or(Err(DeviceError::Overflow))?),
            SeekFrom::Current(v) => self
                .pos
                .saturating_sub(v.unsigned_abs().try_into().or(Err(DeviceError::Overflow))?),
        };
        if r > self.file.size {
            return Err(Error::InvalidIndex);
        }
        self.pos = r;
        Ok(self.pos as u64)
    }
}
impl<B: BlockDevice, S: FileSync> Read<DeviceError> for File<'_, B, S> {
    #[inline]
    fn read(&mut self, b: &mut [u8]) -> Result<usize, Error> {
        Ok(self.read(b)?)
    }
}
impl<B: BlockDevice, S: FileSync> Write<DeviceError> for File<'_, B, S> {
    #[inline]
    fn flush(&mut self) -> Result<(), Error> {
        Ok(self.flush()?)
    }
    #[inline]
    fn write(&mut self, b: &[u8]) -> Result<usize, Error> {
        Ok(self.write(b)?)
    }
}

impl<B: BlockDevice> Deref for Reader<'_, B> {
    type Target = DirEntry;

    #[inline]
    fn deref(&self) -> &DirEntry {
        &self.f.file
    }
}
impl<B: BlockDevice> Seek<DeviceError> for Reader<'_, B> {
    #[inline]
    fn seek(&mut self, s: SeekFrom) -> Result<u64, Error> {
        self.f.seek(s)
    }
}
impl<B: BlockDevice> Read<DeviceError> for Reader<'_, B> {
    #[inline]
    fn read(&mut self, b: &mut [u8]) -> Result<usize, Error> {
        Ok(self.read(b)?)
    }
}

impl PartialEq<str> for DirEntry {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self.eq(other.as_bytes())
    }
}
impl PartialEq<[u8]> for DirEntry {
    #[inline]
    fn eq(&self, other: &[u8]) -> bool {
        self.name.eq(other)
    }
}
impl From<DirEntryPtr<'_>> for DirEntry {
    #[inline]
    fn from(mut v: DirEntryPtr<'_>) -> DirEntry {
        v.entry()
    }
}
impl From<&mut DirEntryPtr<'_>> for DirEntry {
    #[inline]
    fn from(v: &mut DirEntryPtr<'_>) -> DirEntry {
        v.entry()
    }
}

impl Deref for DirEntryPtr<'_> {
    type Target = DirEntryFull;

    #[inline]
    fn deref(&self) -> &DirEntryFull {
        unsafe { &*self.ptr.as_ptr() }
    }
}
impl DerefMut for DirEntryPtr<'_> {
    #[inline]
    fn deref_mut(&mut self) -> &mut DirEntryFull {
        unsafe { &mut *self.ptr.as_ptr() }
    }
}

impl Deref for DirEntryFull {
    type Target = DirEntry;

    #[inline]
    fn deref(&self) -> &DirEntry {
        &self.entry
    }
}
impl PartialEq<str> for DirEntryFull {
    #[inline]
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

impl<B: BlockDevice> Deref for Directory<'_, B> {
    type Target = DirEntry;

    #[inline]
    fn deref(&self) -> &DirEntry {
        &self.dir
    }
}

impl FileSync for Safe {
    #[inline]
    fn cache() -> BlockPtr {
        Cache::block_a()
    }
}
impl FileSync for Unsafe {
    #[inline]
    fn cache() -> BlockPtr {
        unsafe { Cache::block_a_nolock() }
    }
}

#[inline]
fn time_read(a: u16, b: u16) -> Time {
    unsafe {
        Time {
            year:    (a.unchecked_shr(9) + 0xA) + 0x7B2u16,
            month:   Month::from((a.unchecked_shr(5) & 0xF) as u8 + 1),
            day:     (a & 0x1F) as u8,
            hours:   (b.unchecked_shr(11) & 0x1F) as u8,
            mins:    (b.unchecked_shr(5) & 0x3F) as u8,
            secs:    (b.unchecked_shl(1) & 0x3F) as u8,
            weekday: Weekday::None,
        }
    }
}
#[inline]
fn time_write(t: &Time, pos: usize, mut b: &mut [u8]) {
    b.write_u16(pos, unsafe {
        ((t.hours as u16).unchecked_shl(11) & 0xF800) | ((t.mins as u16).unchecked_shl(5) & 0x7E0) | (((t.secs as u16) / 2) & 0x1F)
    });
    b.write_u16(pos + 2, unsafe {
        ((t.year.saturating_sub(0x7B2)).saturating_sub(10).unchecked_shl(9) & 0xFE00) | ((t.month as u16 + 1).unchecked_shl(5) & 0x1E0) | ((t.day as u16 + 1) & 0x1F)
    });
}
fn delete<'a, B: BlockDevice>(vol: &'a Volume<'a, B>, dir: &DirEntry, t: &mut Block, force: bool) -> DevResult<()> {
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
            let _ = delete(vol, &v, t, force)?;
        } else {
            let _ = v.delete(vol, t)?;
        }
    }
    let _ = dir.delete(vol, t)?;
    vol.truncate(t, dir.index())
}

pub mod state {
    pub struct Safe;
    pub struct Unsafe;
}
