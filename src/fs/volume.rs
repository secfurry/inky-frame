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

use core::cmp::{self, PartialEq};
use core::convert::{AsRef, Into};
use core::iter::Iterator;
use core::option::Option::{self, None, Some};
use core::result::Result::{self, Err, Ok};

use crate::fs::volume::helpers::{Blocks, Manager};
use crate::fs::{Block, BlockCache, BlockDevice, BlockEntryIter, DeviceError, Storage, le_u16, le_u32};

mod file;
mod helpers;
mod iter;
mod name;

pub use self::file::*;
pub use self::iter::*;
pub use self::name::*;

pub(super) const DIR_SIZE: usize = 0x20usize;

pub enum FatVersion {
    Fat16(u16),
    Fat32(u32),
}

pub struct PathIter<'a> {
    buf: &'a [u8],
    pos: usize,
}
pub struct Volume<'a, B: BlockDevice> {
    dev:  &'a Storage<B>,
    man:  Manager,
    name: VolumeName,
}

pub type Cluster = Option<u32>;

impl<'a> PathIter<'a> {
    #[inline(always)]
    pub fn new(buf: &'a [u8]) -> PathIter<'a> {
        PathIter { buf, pos: 0usize }
    }
}
impl<'a, B: BlockDevice> Volume<'a, B> {
    pub(super) fn parse(dev: &'a Storage<B>, mut b: Block, lba: u32, blocks: u32) -> Result<Volume<'a, B>, DeviceError> {
        dev.read_single(&mut b, lba)?;
        if bp_footer(&b) != 0xAA55 {
            return Err(DeviceError::InvalidFileSystem);
        }
        let h = bp_root_entries(&b) * DIR_SIZE as u32;
        let a = h / Block::SIZE as u32;
        let n = (bp_blocks_count(&b) - (bp_blocks_reserved(&b) + (bp_fat_entries(&b) * bp_fat_size(&b)) + if a != h { a + 1 } else { a })) / bp_blocks_in_cluster(&b);
        if n < 0xFF5 {
            return Err(DeviceError::UnsupportedFileSystem);
        }
        let m = Manager::from_info(n, dev, &b, lba, Blocks::new(blocks, b[0xD]))?;
        let n = VolumeName::from_slice(&m.ver, &b);
        Ok(Volume { dev, man: m, name: n })
    }

    #[inline(always)]
    pub fn name(&self) -> &str {
        self.name.as_str()
    }
    #[inline(always)]
    pub fn cluster_count(&self) -> u32 {
        self.man.cluster_count()
    }
    #[inline(always)]
    pub fn device(&self) -> &Storage<B> {
        self.dev
    }
    #[inline(always)]
    pub fn fat_version(&self) -> &FatVersion {
        &self.man.ver
    }
    #[inline(always)]
    pub fn dir_root(&'a self) -> Directory<'a, B> {
        Directory::new(DirEntry::new_root(), self)
    }
    #[inline]
    pub fn open(&'a self, path: impl AsRef<str>) -> Result<File<'a, B>, DeviceError> {
        self.open_inner(path.as_ref().as_bytes(), Mode::READ)
    }
    #[inline]
    pub fn file_create(&'a self, path: impl AsRef<str>) -> Result<File<'a, B>, DeviceError> {
        self.open_inner(
            path.as_ref().as_bytes(),
            Mode::WRITE | Mode::CREATE | Mode::TRUNCATE,
        )
    }
    #[inline]
    pub fn dir_open(&'a self, path: impl AsRef<str>) -> Result<Directory<'a, B>, DeviceError> {
        let p = path.as_ref().as_bytes();
        if p.is_empty() || (p.len() == 1 && is_sep(p[0])) {
            return Ok(Directory::new(DirEntry::new_root(), self));
        }
        let mut x: DirectoryIndex<'_, B> = DirectoryIndex::new(self);
        let mut b = Block::new();
        self.find_dir(&mut x, &mut b, p, false)
    }
    #[inline]
    pub fn dir_create(&'a self, path: impl AsRef<str>) -> Result<Directory<'a, B>, DeviceError> {
        let mut x = DirectoryIndex::new(self);
        let mut b: Block = Block::new();
        self.find_dir(&mut x, &mut b, path.as_ref().as_bytes(), true)
    }
    #[inline]
    pub fn file_open(&'a self, path: impl AsRef<str>, mode: u8) -> Result<File<'a, B>, DeviceError> {
        self.open_inner(path.as_ref().as_bytes(), mode)
    }

    #[inline]
    pub unsafe fn list_entry(&'a self, target: Option<&DirEntry>) -> Result<DirectoryIndex<'a, B>, DeviceError> {
        let mut x = DirectoryIndex::new(self);
        x.setup(target.and_then(|v| v.cluster()))?;
        Ok(x)
    }
    #[inline]
    pub unsafe fn file_entry(&'a self, name: impl AsRef<str>, parent: Option<&DirEntry>, mode: u8) -> Result<File<'a, B>, DeviceError> {
        let mut x = DirectoryIndex::new(self);
        let mut b = Block::new();
        self.node_create_file(&mut x, &mut b, name.as_ref().as_bytes(), parent, mode)
    }
    pub unsafe fn dir_entry(&'a self, name: impl AsRef<str>, parent: Option<&DirEntry>, create: bool) -> Result<Directory<'a, B>, DeviceError> {
        let p = parent.and_then(|v| v.cluster());
        let mut x = DirectoryIndex::new(self);
        x.setup(p)?;
        let n = name.as_ref().as_bytes();
        if let Some(e) = x.find(|e| e.eq(n))? {
            return if e.is_directory() { Ok(Directory::new(e, self)) } else { Err(DeviceError::NotADirectory) };
        }
        if !create {
            return Err(DeviceError::NotFound);
        }
        let mut b = Block::new();
        Ok(Directory::new(self.node_create_dir(&mut b, n, p)?, self))
    }

    fn open_inner(&'a self, path: &[u8], mode: u8) -> Result<File<'a, B>, DeviceError> {
        if !Mode::is_mode_valid(mode) {
            return Err(DeviceError::InvalidOptions);
        }
        if path.is_empty() {
            return Err(DeviceError::NotFound);
        }
        let mut x = DirectoryIndex::new(self);
        let mut k = Block::new();
        // Split file into dirs and path.
        let i = match path.iter().rposition(|v| is_sep(*v)) {
            Some(v) if v + 1 >= path.len() => return Err(DeviceError::NotAFile),
            Some(v) => v,
            None => return self.open_inner_file(&mut x, &mut k, path, mode),
        };
        let d = self.find_dir(&mut x, &mut k, &path[0..i], mode & Mode::CREATE != 0)?;
        self.node_create_file(&mut x, &mut k, &path[i + 1..], Some(d.entry()), mode)
    }
    fn node_create_dir(&self, buf: &mut Block, name: &[u8], parent: Cluster) -> Result<DirEntry, DeviceError> {
        let e = self.node_create(buf, name, 0x10, parent, true)?;
        let i = self.man.block_pos(e.cluster());
        let s = DirEntry::new_self(&e, i);
        let p = DirEntry::new_parent(parent, i);
        s.write_entry(&self.man.ver, buf);
        p.write_entry(&self.man.ver, &mut buf[DIR_SIZE..]);
        self.dev.write_single(&buf, i)?;
        buf.clear();
        for k in (i + 1)..=i + self.man.blocks.blocks_per_cluster() {
            self.dev.write_single(&buf, k)?;
        }
        Ok(e)
    }
    fn node_create(&self, buf: &mut Block, name: &[u8], attrs: u8, parent: Cluster, alloc: bool) -> Result<DirEntry, DeviceError> {
        let mut n = LongName::empty();
        n.fill(name)?; // Save stack allocations
        let s = n.lfn_size() + 1;
        let r = self.find_with(buf, s, parent, |_, b| b[0] == 0 || b[0] == 0xE5)?;
        let (mut e, mut t) = (DirEntry::new(attrs, s - 1), 0u8);
        e.fill(name); // Save stack allocations
        let mut w = BlockEntryIter::new(r.blocks());
        for (_, i, l, o) in r {
            if l && !w.in_scope(i) {
                w.load_and_flush(self.dev, i)?;
            }
            let (b, v) = (w.buffer(i), o * DIR_SIZE);
            if t + 0x1 == s {
                e.write_prep(i, v);
                if alloc {
                    e.allocate(self, buf)?;
                }
                e.write_entry(&self.man.ver, &mut b[v..]);
                break;
            }
            e.write_lfn_entry(&n, t, s - 0x1, &mut b[v..]);
            t += 0x1;
        }
        w.flush(self.dev)?;
        Ok(e.into())
    }
    fn find_with(&self, buf: &mut Block, entries: u8, parent: Cluster, pred: fn(u32, &[u8]) -> bool) -> Result<Range, DeviceError> {
        let mut t = self.man.entries_count(parent);
        let mut k = parent.unwrap_or_else(|| self.man.root());
        let (mut c, mut r) = (BlockCache::new(), Range::new());
        'outer: loop {
            let p = self.man.block_pos_at(k);
            for i in p..=p + t {
                self.dev.read_single(buf, i)?;
                for e in 0..(Block::SIZE / DIR_SIZE) as u32 {
                    let x = e as usize * DIR_SIZE;
                    if !pred(i, &buf[x..x + DIR_SIZE]) {
                        r.clear();
                        continue;
                    }
                    if r.mark(k, i, e as u32) >= entries {
                        r.finish(k, i, e);
                        break 'outer;
                    }
                }
            }
            c.clear();
            k = match self.man.cluster_next(self.dev, buf, &mut c, k)? {
                None => self.man.cluster_allocate(self.dev, buf, Some(k), true)?,
                Some(v) => v,
            };
            t = self.man.blocks.blocks_per_cluster();
        }
        Ok(r)
    }
    #[inline]
    fn open_inner_file(&'a self, x: &mut DirectoryIndex<'a, B>, buf: &mut Block, name: &[u8], mode: u8) -> Result<File<'a, B>, DeviceError> {
        x.setup(None)?;
        match x.find(|e| e.eq(name))? {
            Some(e) if e.is_directory() => return Err(DeviceError::NotAFile),
            Some(e) => Ok(File::new(e, mode, self)),
            None if Mode::is_create(mode) => Ok(File::new(
                self.node_create(buf, name, 0, None, false)?,
                mode,
                self,
            )),
            None => Err(DeviceError::NotFound),
        }
    }
    fn find_dir(&'a self, x: &mut DirectoryIndex<'a, B>, buf: &mut Block, path: &[u8], makedirs: bool) -> Result<Directory<'a, B>, DeviceError> {
        if path.is_empty() || (path.len() == 1 && is_sep(path[0])) {
            return Ok(Directory::new(DirEntry::new_root(), self));
        }
        let mut p: Option<DirEntry> = None; // Current Parent
        let mut o: Option<DirEntry> = None; // Old Parent (for "../")
        for (e, c) in PathIter::new(path) {
            if e.len() == 2 && e[0] == b'.' && e[1] == b'.' {
                // Swap out old parent to the current one, basically going "up"
                // one directory.
                (p, o) = (o, None);
                continue;
            }
            let i = p.as_ref().and_then(|v| v.cluster());
            x.setup(i)?;
            o = p; // Set the current Parent as the old one before setting the new one.
            p = match x.find(|v| v.eq(e))? {
                Some(v) if v.is_file() => return Err(DeviceError::NotADirectory),
                Some(v) if c => return Ok(Directory::new(v, self)),
                Some(v) => Some(v),
                None if makedirs && c => return Ok(Directory::new(self.node_create_dir(buf, e, i)?, self)),
                None if makedirs => Some(self.node_create_dir(buf, e, i)?),
                None => return Err(DeviceError::NotFound),
            };
        }
        match p {
            Some(v) => Ok(Directory::new(v, self)),
            None => Err(DeviceError::NotFound),
        }
    }
    fn node_create_file(&'a self, x: &mut DirectoryIndex<'a, B>, buf: &mut Block, name: &[u8], parent: Option<&DirEntry>, mode: u8) -> Result<File<'a, B>, DeviceError> {
        if !Mode::is_mode_valid(mode) {
            return Err(DeviceError::InvalidOptions);
        }
        let p = parent.and_then(|v| v.cluster());
        x.setup(p)?;
        match x.find(|e| e.eq(name))? {
            Some(e) if e.is_directory() => Err(DeviceError::NotAFile),
            Some(e) => {
                let mut f = File::new(e, mode, self);
                if mode & Mode::APPEND != 0 {
                    f.seek_to_end();
                } else if mode & Mode::TRUNCATE != 0 {
                    let mut b = Block::new();
                    self.man.cluster_truncate(self.dev, &mut b, f.cluster_abs())?;
                    f.zero();
                    f.sync(self.dev, &mut b, &self.man.ver)?;
                }
                Ok(f)
            },
            None if Mode::is_create(mode) => Ok(File::new(
                self.node_create(buf, name, 0, p, false)?,
                mode,
                self,
            )),
            None => Err(DeviceError::NotFound),
        }
    }
}

impl<'a> Iterator for PathIter<'a> {
    type Item = (&'a [u8], bool);

    fn next(&mut self) -> Option<(&'a [u8], bool)> {
        if self.buf.is_empty() || self.pos >= self.buf.len() {
            return None;
        }
        let (s, mut n) = (self.buf.len(), self.pos);
        while n < s {
            let (i, c) = match self.buf[n..].iter().position(|v| is_sep(*v)) {
                Some(i) if i == 1 && self.buf[n] == b'.' => (i + 1, true),
                Some(i) if i == 0 => (i + 1, true),
                Some(i) => (i, false),
                None => (s, false),
            };
            self.pos += i;
            if !c {
                return Some((&self.buf[n..cmp::min(self.pos, s)], self.pos >= s));
            }
            n += i;
        }
        None
    }
}

#[inline(always)]
fn is_sep(v: u8) -> bool {
    v == b'\\' || v == b'/'
}
#[inline(always)]
fn bp_footer(b: &Block) -> u16 {
    le_u16(&b[510..])
}
#[inline]
fn bp_fat_size(b: &Block) -> u32 {
    let r = le_u16(&b[22..]) as u32;
    if r > 0 { r } else { le_u32(&b[36..]) }
}
#[inline(always)]
fn bp_fat32_info(b: &Block) -> u32 {
    le_u16(&b[48..]) as u32
}
#[inline(always)]
fn bp_fat_entries(b: &Block) -> u32 {
    b[16] as u32
}
#[inline]
fn bp_blocks_count(b: &Block) -> u32 {
    let r = le_u16(&b[19..]) as u32;
    if r > 0 { r } else { le_u32(&b[32..]) }
}
#[inline(always)]
fn bp_root_entries(b: &Block) -> u32 {
    le_u16(&b[17..]) as u32
}
#[inline(always)]
fn bp_blocks_bytes(b: &Block) -> usize {
    le_u16(&b[11..]) as usize
}
#[inline(always)]
fn bp_blocks_reserved(b: &Block) -> u32 {
    le_u16(&b[14..]) as u32
}
#[inline(always)]
fn bp_blocks_in_cluster(b: &Block) -> u32 {
    b[13] as u32
}
