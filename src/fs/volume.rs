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

use core::cell::UnsafeCell;
use core::cmp::{PartialEq, PartialOrd};
use core::convert::AsRef;
use core::iter::{FusedIterator, IntoIterator, Iterator};
use core::matches;
use core::option::Option::{self, None, Some};
use core::result::Result::{Err, Ok};

use crate::fs::{Block, BlockCache, BlockDevice, BlockEntryIter, Cache, DevResult, DeviceError, Storage};
use crate::{Slice, SliceMut};

mod file;
mod iter;
mod name;
mod objects;

pub use self::file::*;
pub use self::iter::*;
pub use self::name::*;
pub use self::objects::*;

const CLUSTER_EOF: u32 = 0xFFFFFFFFu32;

pub struct Volume<'a, B: BlockDevice> {
    dev:   &'a Storage<B>,
    ver:   FatVersion,
    info:  Clusters,
    name:  VolumeName,
    block: Blocks,
    index: Index,
}

struct Index {
    fat:  u32,
    lba:  u32,
    data: u32,
    root: ClusterIndex,
}
struct Blocks {
    count:    u32,
    clusters: u8,
}
struct Clusters {
    next:  UnsafeCell<Cluster>,
    free:  UnsafeCell<Cluster>,
    count: u32,
}
struct PathIter<'a>(&'a [u8]);

impl Index {
    #[inline]
    fn new(fat: u32, lba: u32, data: u32, root: u32) -> DevResult<Index> {
        Ok(Index {
            fat,
            lba,
            data,
            root: ClusterIndex::new(root).ok_or(DeviceError::InvalidFileSystem)?,
        })
    }
}
impl Blocks {
    #[inline]
    const fn new(count: u32, clusters: u8) -> Blocks {
        Blocks { count, clusters }
    }

    #[inline]
    const fn bytes(&self) -> u32 {
        self.clusters as u32 * Block::SIZE as u32
    }
    #[inline]
    const fn blocks(&self) -> u32 {
        self.clusters as u32
    }
}
impl Clusters {
    #[inline]
    const fn new_16(v: u32) -> Clusters {
        Clusters {
            free:  UnsafeCell::new(None),
            next:  UnsafeCell::new(None),
            count: v,
        }
    }
    #[inline]
    const fn new_32(v: u32, next: u32, free: u32) -> Clusters {
        Clusters {
            free:  UnsafeCell::new(match free {
                0 | 0xFFFFFFFF => None,
                v => Some(unsafe { ClusterIndex::new_unchecked(v) }),
            }),
            next:  UnsafeCell::new(match next {
                0 | 0x1 | 0xFFFFFFFF => None,
                v => Some(unsafe { ClusterIndex::new_unchecked(v) }),
            }),
            count: v,
        }
    }

    #[inline]
    fn free_add(&self) {
        ClusterIndex::add_one(unsafe { &mut *self.free.get() })
    }
    #[inline]
    fn free_remove(&self) {
        ClusterIndex::sub_one(unsafe { &mut *self.free.get() })
    }
    #[inline(always)]
    fn next(&self) -> Cluster {
        unsafe { *self.next.get() }
    }
    #[inline(always)]
    fn free(&self) -> Cluster {
        unsafe { *self.free.get() }
    }
    #[inline(always)]
    fn is_empty(&self) -> bool {
        unsafe { (&*self.next.get()).is_none() && (&*self.free.get()).is_none() }
    }
    #[inline]
    fn next_set(&self, v: Cluster) {
        unsafe { *self.next.get() = v }
    }
    #[inline]
    fn free_set(&self, v: ClusterIndex) {
        let n = unsafe { &mut *self.free.get() };
        if n.is_some_and(|i| i.le(&v)) {
            return;
        }
        *n = Some(v);
    }
}
impl<'a, B: BlockDevice> Volume<'a, B> {
    pub(super) fn new(dev: &'a Storage<B>, b: &mut Block, lba: u32, blocks: u32) -> DevResult<Volume<'a, B>> {
        let _ = dev.read_single(b, lba)?;
        // 0x1FE - Boot Sector Signature
        if b.read_u16(510) != 0xAA55 {
            return Err(DeviceError::InvalidFileSystem);
        }
        // 0x11 - Max Root Directory Entries
        let e = b.read_u16(17) as u32 * DirEntry::SIZE as u32;
        let (h, c) = (e / Block::SIZE as u32, _blocks(b));
        // 0xD - Logical Sectors per Cluster
        // 0xE - Count of Reserved Logical Sectors
        let (r, s, k) = (b.read_u16(14) as u32, _size(b), b.read_u8(13));
        let n = (c - (r + s + if e != h { h + 1 } else { h })) / k as u32;
        if n < 0xFF5 {
            return Err(DeviceError::UnsupportedFileSystem);
        }
        let z = Blocks::new(blocks, k);
        let v = VolumeName::new(n > 0xFFF5, b);
        if n <= 0xFFF5 {
            // 0xB - Bytes per Logical Sector
            if b.read_u16(11) as usize != Block::SIZE {
                return Err(DeviceError::InvalidFileSystem);
            }
            return Ok(Volume {
                dev,
                ver: FatVersion::new_16(b.read_u16(17)),
                info: Clusters::new_16(n),
                name: v,
                block: z,
                index: Index::new(
                    r,
                    lba,
                    r + s + ((e + (Block::SIZE as u32 - 1)) / Block::SIZE as u32),
                    r + s,
                )?,
            });
        }
        // 0x2C - Cluster Root Start
        // 0x30 - FS Information Sector
        let (i, t) = (b.read_u16(48) as u32 + lba, b.read_u32(44));
        let _ = dev.read_single(b, i)?;
        if b.read_u32(0) != 0x41615252 || b.read_u32(484) != 0x61417272 || b.read_u32(508) != 0xAA550000 {
            return Err(DeviceError::InvalidFileSystem);
        }
        // 0x1E8 - Last Known Number of Free Data Clusters
        // 0x1EC - Most Recent Allocated Cluster
        Ok(Volume {
            dev,
            ver: FatVersion::new_32(i),
            info: Clusters::new_32(n, b.read_u32(492), b.read_u32(488)),
            name: v,
            block: z,
            index: Index::new(r, lba, r + s, t)?,
        })
    }

    #[inline]
    pub fn pos_fat(&self) -> u32 {
        self.index.fat
    }
    #[inline]
    pub fn pos_lba(&self) -> u32 {
        self.index.lba
    }
    #[inline]
    pub fn pos_data(&self) -> u32 {
        self.index.data
    }
    #[inline]
    pub fn pos_root(&self) -> u32 {
        self.index.root.get()
    }
    #[inline]
    pub fn name(&self) -> &VolumeName {
        &self.name
    }
    #[inline]
    pub fn cluster_count(&self) -> u32 {
        self.info.count
    }
    #[inline]
    pub fn device(&self) -> &Storage<B> {
        self.dev
    }
    #[inline]
    pub fn fat_version(&self) -> &FatVersion {
        &self.ver
    }
    #[inline]
    pub fn dir_root(&'a self) -> Directory<'a, B> {
        Directory::new(DirEntry::new_root(), self)
    }
    #[inline]
    pub fn open(&'a self, path: impl AsRef<str>) -> DevResult<File<'a, B>> {
        self.open_inner(path.as_ref().as_bytes(), Mode::READ)
    }
    #[inline]
    pub fn file_create(&'a self, path: impl AsRef<str>) -> DevResult<File<'a, B>> {
        self.open_inner(
            path.as_ref().as_bytes(),
            Mode::WRITE | Mode::CREATE | Mode::TRUNCATE,
        )
    }
    #[inline]
    pub fn dir_open(&'a self, path: impl AsRef<str>) -> DevResult<Directory<'a, B>> {
        let p = path.as_ref().as_bytes();
        if p.is_empty() || (p.len() == 1 && unsafe { is_sep(p.get_unchecked(0)) }) {
            return Ok(Directory::new(DirEntry::new_root(), self));
        }
        let mut x = DirectoryIndex::new(self);
        {
            let mut b = Cache::block_a();
            self.find_dir(&mut x, &mut b, p, false)
        }
    }
    #[inline]
    pub fn dir_create(&'a self, path: impl AsRef<str>) -> DevResult<Directory<'a, B>> {
        let mut x = DirectoryIndex::new(self);
        {
            let mut b = Cache::block_a();
            self.find_dir(&mut x, &mut b, path.as_ref().as_bytes(), true)
        }
    }
    #[inline]
    pub fn file_open(&'a self, path: impl AsRef<str>, mode: u8) -> DevResult<File<'a, B>> {
        self.open_inner(path.as_ref().as_bytes(), mode)
    }

    #[inline]
    pub unsafe fn list_entry(&'a self, target: Option<&DirEntry>) -> DevResult<DirectoryIndex<'a, B>> {
        let mut x = DirectoryIndex::new(self);
        let _ = unsafe { x.reset_cluster(target.and_then(|v| v.cluster()))? };
        Ok(x)
    }
    #[inline]
    pub unsafe fn file_entry(&'a self, name: impl AsRef<str>, parent: Option<&DirEntry>, mode: u8) -> DevResult<File<'a, B>> {
        let mut x = DirectoryIndex::new(self);
        {
            let mut b = Cache::block_a();
            self.create_file(&mut x, &mut b, name.as_ref().as_bytes(), parent, mode)
        }
    }
    pub unsafe fn dir_entry(&'a self, name: impl AsRef<str>, parent: Option<&DirEntry>, create: bool) -> DevResult<Directory<'a, B>> {
        let p = parent.and_then(|v| v.cluster());
        let mut x = DirectoryIndex::new(self);
        let _ = unsafe { x.reset_cluster(p)? };
        let n = name.as_ref().as_bytes();
        if let Some(e) = x.find(|e| e.eq(n))? {
            return if e.is_directory() { Ok(Directory::new(e, self)) } else { Err(DeviceError::NotADirectory) };
        }
        if !create {
            return Err(DeviceError::NotFound);
        }
        {
            let mut b = Cache::block_a();
            Ok(Directory::new(self.create_dir(&mut b, n, p)?, self))
        }
    }

    #[inline]
    fn entries(&self, idx: Cluster) -> u32 {
        match idx {
            Some(_) => self.block.blocks(),
            None if self.ver.is_fat32() => self.block.blocks(),
            None => self.ver.sector() * DirEntry::SIZE as u32,
        }
    }
    #[inline]
    fn block_pos(&self, idx: Cluster) -> u32 {
        match idx {
            Some(v) => self.block_pos_at(v),
            None => self.block_pos_at(self.index.root),
        }
    }
    #[inline]
    fn offset(&self, idx: u32) -> (usize, u32) {
        let v = if self.ver.is_fat32() { 4 } else { 2 };
        (
            (idx as usize * v) % Block::SIZE,
            self.index.lba + (self.index.fat + ((idx as usize * v) as u32 / Block::SIZE as u32)),
        )
    }
    #[inline]
    fn root(&self, idx: Cluster) -> ClusterIndex {
        match idx {
            Some(v) => v,
            None => self.index.root,
        }
    }
    #[inline]
    fn block_pos_at(&self, idx: ClusterIndex) -> u32 {
        self.index.lba + self.index.data + ((*idx - 0x2) * self.block.blocks())
    }
    #[inline]
    fn sync(&self, tmp: &mut Block) -> DevResult<()> {
        if !self.ver.is_fat32() {
            return Ok(());
        }
        if self.info.is_empty() {
            return Ok(());
        }
        let i = self.ver.sector();
        let _ = self.dev.read_single(tmp, i)?;
        if let Some(v) = self.info.free() {
            // 0x1E8 - Last Known Number of Free Data Clusters
            tmp.write_u32(488, *v);
        }
        if let Some(v) = self.info.next() {
            // 0x1EC - Most Recent Allocated Cluster
            tmp.write_u32(492, *v);
        }
        self.dev.write_single(tmp, i)
    }
    fn truncate(&self, tmp: &mut Block, idx: ClusterIndex) -> DevResult<()> {
        if !idx.is_valid() {
            return Ok(());
        }
        let mut c = BlockCache::new();
        let n = match self.next(tmp, &mut c, idx)? {
            None => return Ok(()),
            Some(v) => v,
        };
        self.info.free_set(n);
        let _ = self.update(tmp, idx, CLUSTER_EOF)?;
        let mut x = n;
        loop {
            c.clear();
            let r = self.next(tmp, &mut c, x)?;
            let _ = self.update(tmp, x, 0)?;
            x = match r {
                Some(v) => v,
                None => break,
            };
            self.info.free_add();
        }
        Ok(())
    }
    fn open_inner(&'a self, path: &[u8], mode: u8) -> DevResult<File<'a, B>> {
        if !Mode::is_mode_valid(mode) {
            return Err(DeviceError::InvalidOptions);
        }
        if path.is_empty() {
            return Err(DeviceError::NotFound);
        }
        let mut x = DirectoryIndex::new(self);
        {
            let mut k = Cache::block_a();
            // Split file into dirs and path.
            let i = match path.iter().rposition(is_sep) {
                Some(v) if v + 1 >= path.len() => return Err(DeviceError::NotAFile),
                Some(v) => v,
                None => return self.open_inner_file(&mut x, &mut k, path, mode),
            };
            let d = self.find_dir(
                &mut x,
                &mut k,
                unsafe { path.get_unchecked(0..i) },
                mode & Mode::CREATE != 0,
            )?;
            self.create_file(
                &mut x,
                &mut k,
                unsafe { path.get_unchecked(i + 1..) },
                Some(&d),
                mode,
            )
        }
    }
    #[inline]
    fn free_try(&self, tmp: &mut Block, start: u32, end: u32) -> DevResult<Cluster> {
        match self.free(tmp, start, end) {
            Ok(v) => Ok(Some(v)),
            Err(_) if start > 0x2 => Ok(Some(self.free(tmp, 0x2, end)?)),
            Err(e) => return Err(e),
        }
    }
    fn update(&self, tmp: &mut Block, idx: ClusterIndex, val: u32) -> DevResult<()> {
        let (i, n) = self.offset(*idx);
        let _ = self.dev.read_single(tmp, n)?;
        if self.ver.is_fat32() {
            if i + 4 < Block::SIZE {
                tmp.write_u32(i, (tmp.read_u32(i) & 0xF0000000) | val);
            }
        } else {
            if i + 2 < Block::SIZE {
                tmp.write_u16(i, val as u16);
            }
        }
        self.dev.write_single(tmp, n)
    }
    fn free(&self, tmp: &mut Block, start: u32, end: u32) -> DevResult<ClusterIndex> {
        let (mut p, v) = (start, self.ver.is_fat32());
        let (a, mut c) = (if v { 0x4 } else { 0x2 }, BlockCache::new());
        while p < end {
            let (mut i, n) = self.offset(p);
            let _ = c.read_single(&self.dev, tmp, n)?;
            while i <= Block::SIZE - a {
                if (v && tmp.read_u32(i) & 0xFFFFFFF == 0) || (!v && tmp.read_u16(i) == 0) {
                    return Ok(unsafe { ClusterIndex::new_unchecked(p) });
                }
                (p, i) = (p + 0x1, i + a);
            }
        }
        Err(DeviceError::NoSpace)
    }
    fn allocate(&self, tmp: &mut Block, prev: Cluster, zero: bool) -> DevResult<ClusterIndex> {
        let e = self.block.count + 0x2;
        let s = match prev.as_deref() {
            Some(&v) if v < e => v,
            _ => 2,
        };
        let n = self.free_try(tmp, s, e)?.ok_or(DeviceError::NoSpace)?;
        let _ = self.update(tmp, n, CLUSTER_EOF)?;
        if let Some(v) = prev {
            let _ = self.update(tmp, v, *n)?;
        }
        let f = self.free_try(tmp, *n, e)?;
        self.info.next_set(f);
        self.info.free_remove();
        if zero {
            tmp.clear();
            let p = self.block_pos_at(n);
            for i in p..=p + self.block.blocks() {
                self.dev.write_single(tmp, i)?;
            }
        }
        Ok(n)
    }
    #[inline]
    fn create_dir(&self, tmp: &mut Block, name: &[u8], parent: Cluster) -> DevResult<DirEntry> {
        let e = self.create(tmp, name, 0x10, parent, true)?;
        let i = self.block_pos(e.cluster());
        unsafe {
            let s = DirEntry::new_self(&e, i);
            let p = DirEntry::new_parent(parent, i);
            // Write Parent and Self entries
            s.write_entry(self.ver.is_fat32(), tmp);
            p.write_entry(self.ver.is_fat32(), tmp.get_unchecked_mut(DirEntry::SIZE..));
        }
        let _ = self.dev.write_single(&tmp, i)?;
        tmp.clear();
        // Write empty blocks to create initial Directory space.
        for k in (i + 1)..=i + self.block.blocks() {
            let _ = self.dev.write_single(&tmp, k)?;
        }
        Ok(e)
    }
    #[inline]
    fn next(&self, tmp: &mut Block, cache: &mut BlockCache, idx: ClusterIndex) -> DevResult<Cluster> {
        // NOTE(sf): Instead of returning an error for the EOF or absence of
        //           a next cluster, we return None for easier signaling.
        let (s, i) = self.offset(*idx);
        cache.read_single(&self.dev, tmp, i)?;
        match self.ver.is_fat32() {
            // FAT16
            false if s + 2 > Block::SIZE => Err(DeviceError::InvalidCluster),
            false => match tmp.read_u16(s) {
                0 => Err(DeviceError::InvalidChain),
                0xFFF7 => Err(DeviceError::InvalidCluster),
                0xFFF8..=0xFFFF => Ok(None),
                v => Ok(Some(unsafe { ClusterIndex::new_unchecked(v as u32) })),
            },
            // FAT32
            true if s + 4 > Block::SIZE => Err(DeviceError::InvalidCluster),
            true => match tmp.read_u32(s) & 0xFFFFFFF {
                0 => Err(DeviceError::InvalidChain),
                0xFFFFFF7 => Err(DeviceError::InvalidCluster),
                0x1 | 0xFFFFFF8..=0xFFFFFFF => Ok(None),
                v => Ok(Some(unsafe { ClusterIndex::new_unchecked(v) })),
            },
        }
    }
    fn find(&self, tmp: &mut Block, count: u8, parent: Cluster, pred: fn(u32, &[u8]) -> bool) -> DevResult<Range> {
        let mut t = self.entries(parent);
        let (mut c, mut r, mut k) = (BlockCache::new(), Range::new(), self.root(parent));
        'outer: loop {
            let p = self.block_pos_at(k);
            for i in p..=p + t {
                let _ = self.dev.read_single(tmp, i)?;
                for e in 0..DirEntry::SIZE_PER_BLOCK {
                    let x = e as usize * DirEntry::SIZE;
                    if x < Block::SIZE && !pred(i, tmp.read_slice(x, DirEntry::SIZE)) {
                        r.clear();
                        continue;
                    }
                    if r.mark(*k, i, e) >= count {
                        r.finish(*k, i, e);
                        break 'outer;
                    }
                }
            }
            c.clear();
            k = match self.next(tmp, &mut c, k)? {
                None => self.allocate(tmp, Some(k), true)?,
                Some(v) => v,
            };
            t = self.block.blocks();
        }
        Ok(r)
    }
    fn create(&self, tmp: &mut Block, name: &[u8], attrs: u8, parent: Cluster, alloc: bool) -> DevResult<DirEntry> {
        let mut n = Cache::lfn();
        let _ = n.fill(name)?;
        let s = n.lfn_size();
        // Look for empty or free'd spaces
        let r = self.find(tmp, s, parent, |_, b| matches!(b.read_u8(0), 0 | 0xE5))?;
        let (mut e, mut t) = (DirEntry::new(attrs, s - 1), 0u8);
        e.fill_name(name);
        let mut w = BlockEntryIter::new(r.blocks());
        for (_, i, l, o) in r {
            if l && !w.in_scope(i) {
                let _ = w.load_and_flush(&self.dev, i)?;
            }
            let (b, v) = (w.buffer(i), o * DirEntry::SIZE);
            let d = unsafe { b.get_unchecked_mut(v..) };
            if t + 1 == s {
                e.prepare(i, v);
                if alloc {
                    let _ = e.allocate(self, tmp)?;
                }
                e.write_entry(self.ver.is_fat32(), d);
                break;
            }
            e.write_lfn_entry(&n, t, s - 1, d);
            t += 1;
        }
        let _ = w.flush(self.dev)?;
        Ok(e)
    }
    #[inline]
    fn open_inner_file(&'a self, x: &mut DirectoryIndex<'a, B>, tmp: &mut Block, name: &[u8], mode: u8) -> DevResult<File<'a, B>> {
        let _ = unsafe { x.reset_cluster(None)? };
        match x.find(|e| e.eq(name))? {
            Some(e) if e.is_directory() => return Err(DeviceError::NotAFile),
            Some(e) => Ok(File::new(e, mode, self)),
            None if Mode::is_create(mode) => Ok(File::new(
                self.create(tmp, name, 0, None, false)?,
                mode,
                self,
            )),
            None => Err(DeviceError::NotFound),
        }
    }
    fn find_dir(&'a self, x: &mut DirectoryIndex<'a, B>, tmp: &mut Block, path: &[u8], makedirs: bool) -> DevResult<Directory<'a, B>> {
        if path.is_empty() || (path.len() == 1 && unsafe { is_sep(path.get_unchecked(0)) }) {
            return Ok(Directory::new(DirEntry::new_root(), self));
        }
        let mut p: Option<DirEntry> = None; // Current Parent
        let mut o: Option<DirEntry> = None; // Previous Parent (for "../")
        for (e, c) in PathIter(path).into_iter() {
            if e.len() == 2 && e.read_u8(0) == b'.' && e.read_u8(1) == b'.' {
                // Swap out old parent to the current one, basically going "up"
                // one directory.
                (p, o) = (o, None);
                continue;
            }
            let i = p.as_ref().and_then(|v| v.cluster());
            let _ = unsafe { x.reset_cluster(i)? };
            o = p; // Set the current Parent as the old one before setting the new one.
            p = match x.find(|v| v.eq(e))? {
                Some(v) if v.is_file() => return Err(DeviceError::NotADirectory),
                Some(v) if c => return Ok(Directory::new(v, self)),
                Some(v) => Some(v),
                None if makedirs && c => return Ok(Directory::new(self.create_dir(tmp, e, i)?, self)),
                None if makedirs => Some(self.create_dir(tmp, e, i)?),
                None => return Err(DeviceError::NotFound),
            };
        }
        match p {
            Some(v) => Ok(Directory::new(v, self)),
            None => Err(DeviceError::NotFound),
        }
    }
    fn create_file(&'a self, x: &mut DirectoryIndex<'a, B>, tmp: &mut Block, name: &[u8], parent: Option<&DirEntry>, mode: u8) -> DevResult<File<'a, B>> {
        if !Mode::is_mode_valid(mode) {
            return Err(DeviceError::InvalidOptions);
        }
        let p = parent.and_then(|v| v.cluster());
        let _ = unsafe { x.reset_cluster(p)? };
        match x.find(|e| e.eq(name))? {
            Some(e) if e.is_directory() => Err(DeviceError::NotAFile),
            Some(e) => {
                let mut f = File::new(e, mode, self);
                if mode & Mode::APPEND != 0 {
                    f.seek_to_end();
                } else if mode & Mode::TRUNCATE != 0 {
                    let _ = self.truncate(tmp, f.index())?;
                    f.zero();
                    let _ = f.sync(&self.dev, tmp, self.ver.is_fat32())?;
                }
                Ok(f)
            },
            None if Mode::is_create(mode) => Ok(File::new(self.create(tmp, name, 0, p, false)?, mode, self)),
            None => Err(DeviceError::NotFound),
        }
    }
}

impl<'a> Iterator for PathIter<'a> {
    type Item = (&'a [u8], bool);

    fn next(&mut self) -> Option<(&'a [u8], bool)> {
        if self.0.is_empty() {
            return None;
        }
        while !self.0.is_empty() {
            // NOTE(sf): We handle ".." separately since that requires changing the dir.
            let (i, c) = match self.0.iter().position(is_sep) {
                // First char is a seperator
                // --> /dir1/dir2/file
                //     ^ i
                Some(0) => (1, false),
                // First char is a "same dir" marker.
                // --> ./dir1/dir2/file
                //      ^ i
                Some(1) if self.0.read_u8(0) == b'.' => (2, false),
                // Found a dir
                // --> dir1/dir2/file
                //         ^ i
                Some(i) => (i, true),
                // Didn't find a dir, return the whole slice.
                // --> dir1_dir2_file
                None => (self.0.len(), true),
            };
            let (r, v) = unsafe { self.0.split_at_unchecked(i) };
            // Check if the first char in the subslice is a seperator.
            self.0 = if v.first().is_some_and(is_sep) { unsafe { v.get_unchecked(1..) } } else { v };
            // If 'c' is false, we continue the loop, otherwise break and return.
            if c {
                return Some((r, self.0.is_empty()));
            }
        }
        None
    }
}
impl FusedIterator for PathIter<'_> {}

#[inline]
fn is_sep(v: &u8) -> bool {
    *v == b'\\' || *v == b'/'
}
#[inline]
fn _size(b: &[u8]) -> u32 {
    // 0x10 - Number of FAT
    // 0x16 - Logical Sectors per FAT
    // 0x24 - Logical Sectors per FAT (FAT32)
    let (v, n) = (b.read_u16(22) as u32, b.read_u8(16) as u32);
    if v > 0 { v * n } else { n * b.read_u32(36) }
}
#[inline]
fn _blocks(b: &[u8]) -> u32 {
    // 0x13 - Total Logical Sectors
    // 0x20 - Total Logical Sectors (FAT32)
    let v = b.read_u16(19) as u32;
    if v > 0 { v } else { b.read_u32(32) }
}
