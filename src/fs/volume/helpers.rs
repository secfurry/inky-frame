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
use core::matches;
use core::option::Option::{None, Some};
use core::result::Result::{self, Err, Ok};

use crate::fs::volume::{bp_blocks_bytes, bp_blocks_reserved, bp_fat_entries, bp_fat_size, bp_fat32_info, bp_root_entries};
use crate::fs::{Block, BlockCache, BlockDevice, Cache, Cluster, DIR_SIZE, DeviceError, FatVersion, Storage, le_u16, le_u32, to_le_u16, to_le_u32};

pub enum ClusterValue {
    Empty,
    EndOfFile,
    Value(u32),
}

pub struct Blocks {
    count:         u32,
    count_cluster: u8,
}
pub struct Manager {
    pub ver:    FatVersion,
    pub blocks: Blocks,
    fat:        u32,
    lba:        u32,
    data:       u32,
    root:       u32,
    clusters:   Clusters,
}

struct Clusters {
    free:  UnsafeCell<ClustersFree>,
    count: u32,
}
struct ClustersFree {
    next:  Cluster,
    count: Cluster,
}

impl Blocks {
    #[inline(always)]
    pub fn new(count: u32, count_cluster: u8) -> Blocks {
        Blocks { count, count_cluster }
    }

    #[inline(always)]
    pub fn bytes_per_cluster(&self) -> u32 {
        self.count_cluster as u32 * Block::SIZE as u32
    }
    #[inline(always)]
    pub fn blocks_per_cluster(&self) -> u32 {
        self.count_cluster as u32
    }
}
impl Manager {
    pub fn from_info(clusters: u32, dev: &Storage<impl BlockDevice>, b: &Block, lba: u32, blocks: Blocks) -> Result<Manager, DeviceError> {
        let (v, d, r, f, n) = if clusters > 0xFFF5 { new_32(b, dev, lba) } else { new_16(b) }?;
        Ok(Manager::new(
            v,
            r,
            lba,
            le_u16(&b[0xE..]) as u32,
            d,
            blocks,
            Clusters::new(clusters, f, n),
        ))
    }

    #[inline(always)]
    pub fn root(&self) -> u32 {
        self.root
    }
    #[inline(always)]
    pub fn cluster_count(&self) -> u32 {
        self.clusters.count
    }
    #[inline]
    pub fn block_pos_at(&self, pos: u32) -> u32 {
        self.lba + self.data + ((pos - 0x2) * self.blocks.blocks_per_cluster())
    }
    #[inline]
    pub fn block_pos(&self, pos: Cluster) -> u32 {
        match pos {
            Some(v) => self.block_pos_at(v),
            None => self.block_pos_at(self.root),
        }
    }
    #[inline]
    pub fn entries_count(&self, pos: Cluster) -> u32 {
        match pos {
            Some(_) => self.blocks.blocks_per_cluster(),
            None => match self.ver {
                FatVersion::Fat16(v) => v as u32 * DIR_SIZE as u32,
                FatVersion::Fat32(_) => self.blocks.blocks_per_cluster(),
            },
        }
    }
    #[inline]
    pub fn cluster_offset(&self, pos: u32) -> (usize, u32) {
        let m = match self.ver {
            FatVersion::Fat16(_) => 0x2,
            FatVersion::Fat32(_) => 0x4,
        };
        (
            (pos as usize * m) % Block::SIZE,
            self.lba + (self.fat + ((pos as usize * m) as u32 / Block::SIZE as u32)),
        )
    }
    pub fn sync(&self, dev: &Storage<impl BlockDevice>, scratch: &mut Block) -> Result<(), DeviceError> {
        let i = match self.ver {
            FatVersion::Fat32(v) => v,
            _ => return Ok(()),
        };
        let f = self.clusters.free();
        if f.is_empty() {
            return Ok(());
        }
        dev.read_single(scratch, i)?;
        if let Some(v) = f.count {
            to_le_u32(v, &mut scratch[0x1E8..]);
        }
        if let Some(v) = f.next {
            to_le_u32(v, &mut scratch[0x1EC..]);
        }
        dev.write_single(scratch, i)
    }
    pub fn cluster_truncate(&self, dev: &Storage<impl BlockDevice>, scratch: &mut Block, pos: u32) -> Result<(), DeviceError> {
        if pos < 2 {
            return Ok(());
        }
        let mut c = BlockCache::new();
        let n = match self.cluster_next(dev, scratch, &mut c, pos)? {
            None => return Ok(()),
            Some(v) => v,
        };
        self.clusters.set_free(n);
        self.cluster_update(dev, scratch, pos, ClusterValue::EndOfFile)?;
        let mut x = n;
        loop {
            c.clear();
            let r = self.cluster_next(dev, scratch, &mut c, x)?;
            self.cluster_update(dev, scratch, x, ClusterValue::Empty)?;
            x = match r {
                Some(v) => v,
                None => break,
            };
            self.clusters.add_free();
        }
        Ok(())
    }
    pub fn cluster_next_free(&self, dev: &Storage<impl BlockDevice>, scratch: &mut Block, start: u32, end: u32) -> Result<u32, DeviceError> {
        let (mut p, v) = (start, matches!(self.ver, FatVersion::Fat32(_)));
        let (a, mut c) = (if v { 0x4 } else { 0x2 }, BlockCache::new());
        while p < end {
            let (mut i, n) = self.cluster_offset(p);
            c.read_single(dev, scratch, n)?;
            while i <= Block::SIZE - a {
                if v && le_u32(&scratch[i..]) & 0xFFFFFFF == 0 {
                    return Ok(p);
                } else if !v && le_u16(&scratch[i..]) == 0 {
                    return Ok(p);
                }
                (p, i) = (p + 0x1, i + a);
            }
        }
        Err(DeviceError::NoSpace)
    }
    pub fn cluster_update(&self, dev: &Storage<impl BlockDevice>, scratch: &mut Block, pos: u32, val: ClusterValue) -> Result<(), DeviceError> {
        scratch.clear();
        let (i, n) = self.cluster_offset(pos);
        dev.read_single(scratch, n)?;
        match self.ver {
            FatVersion::Fat16(_) if i + 2 < Block::SIZE => to_le_u16(val.value_16(), &mut scratch[i..]),
            FatVersion::Fat32(_) if i + 4 < Block::SIZE => to_le_u32(
                (le_u32(&scratch[i..]) & 0xF0000000) | (val.value_32() & 0xFFFFFFF),
                &mut scratch[i..],
            ),
            _ => (),
        }
        dev.write_single(scratch, n)
    }
    pub fn cluster_allocate(&self, dev: &Storage<impl BlockDevice>, scratch: &mut Block, prev: Cluster, zero: bool) -> Result<u32, DeviceError> {
        let e = self.blocks.count + 0x2;
        let s = match prev {
            Some(v) if v < e => v,
            _ => 2,
        };
        let n = self
            .cluster_next_free_try(dev, scratch, s, e)?
            .ok_or(DeviceError::NoSpace)?;
        self.cluster_update(dev, scratch, n, ClusterValue::EndOfFile)?;
        if let Some(v) = prev {
            self.cluster_update(dev, scratch, v, ClusterValue::Value(n))?;
        }
        let f = self.cluster_next_free_try(dev, scratch, n, e)?;
        self.clusters.set_next(f);
        self.clusters.remove_free();
        if zero {
            scratch.clear();
            let p = self.block_pos_at(n);
            for i in p..=p + self.blocks.blocks_per_cluster() {
                dev.write_single(scratch, i)?;
            }
        }
        Ok(n)
    }
    #[inline]
    pub fn cluster_next_free_try(&self, dev: &Storage<impl BlockDevice>, scratch: &mut Block, start: u32, end: u32) -> Result<Cluster, DeviceError> {
        match self.cluster_next_free(dev, scratch, start, end) {
            Ok(v) => Ok(Some(v)),
            Err(_) if start > 0x2 => self.cluster_next_free(dev, scratch, 0x2, end).map(|v| Some(v)),
            Err(e) => return Err(e),
        }
    }
    #[inline]
    pub fn cluster_next(&self, dev: &Storage<impl BlockDevice>, scratch: &mut Block, cache: &mut BlockCache, pos: u32) -> Result<Cluster, DeviceError> {
        // NOTE(sf): Instead of returning an error for the EOF or absence of
        //           a next cluster, we return None for easier signaling.
        let (s, i) = self.cluster_offset(pos);
        cache.read_single(dev, scratch, i)?;
        match self.ver {
            FatVersion::Fat16(_) if s + 2 > Block::SIZE => Err(DeviceError::InvalidCluster),
            FatVersion::Fat32(_) if s + 4 > Block::SIZE => Err(DeviceError::InvalidCluster),
            FatVersion::Fat16(_) => match le_u16(&scratch[s..]) {
                0xFFF7 => Err(DeviceError::InvalidCluster),
                0xFFF8..=0xFFFF => Ok(None),
                v => Ok(Some(v as u32)),
            },
            FatVersion::Fat32(_) => match le_u32(&scratch[s..]) & 0xFFFFFFF {
                0 => Err(DeviceError::InvalidChain),
                0xFFFFFF7 => Err(DeviceError::InvalidCluster),
                0x1 | 0xFFFFFF8..=0xFFFFFFF => Ok(None),
                v => Ok(Some(v)),
            },
        }
    }

    #[inline(always)]
    fn new(ver: FatVersion, root: u32, lba: u32, fat: u32, data: u32, blocks: Blocks, clusters: Clusters) -> Manager {
        Manager {
            ver,
            fat,
            lba,
            root,
            data,
            blocks,
            clusters,
        }
    }
}
impl Clusters {
    #[inline(always)]
    fn new(count: u32, next_free: Cluster, free_count: Cluster) -> Clusters {
        Clusters {
            count,
            free: UnsafeCell::new(ClustersFree {
                count: free_count,
                next:  next_free,
            }),
        }
    }

    #[inline(always)]
    fn add_free(&self) {
        unsafe { (&mut *self.free.get()).add() }
    }
    #[inline(always)]
    fn remove_free(&self) {
        unsafe { (&mut *self.free.get()).remove() }
    }
    #[inline(always)]
    fn set_free(&self, v: u32) {
        unsafe { (&mut *self.free.get()).set_free(v) }
    }
    #[inline(always)]
    fn set_next(&self, v: Cluster) {
        unsafe { (&mut *self.free.get()).set_next(v) }
    }
    #[inline(always)]
    fn free(&self) -> &ClustersFree {
        unsafe { &*self.free.get() }
    }
}
impl ClustersFree {
    #[inline(always)]
    fn add(&mut self) {
        if let Some(v) = self.count.as_mut() {
            *v = v.saturating_add(1);
        }
    }
    #[inline(always)]
    fn remove(&mut self) {
        if let Some(v) = self.count.as_mut() {
            *v = v.saturating_sub(1);
        }
    }
    #[inline(always)]
    fn is_empty(&self) -> bool {
        self.count.is_none() && self.next.is_none()
    }
    #[inline(always)]
    fn set_free(&mut self, v: u32) {
        match self.count {
            Some(i) if i <= v => return,
            _ => (),
        }
        let _ = self.count.replace(v);
    }
    #[inline(always)]
    fn set_next(&mut self, v: Cluster) {
        self.next = v
    }
}
impl ClusterValue {
    #[inline(always)]
    fn value_16(self) -> u16 {
        match self {
            ClusterValue::Empty => 0u16,
            ClusterValue::EndOfFile => 0xFFFFu16,
            ClusterValue::Value(v) => v as u16,
        }
    }
    #[inline(always)]
    fn value_32(self) -> u32 {
        match self {
            ClusterValue::Empty => 0u32,
            ClusterValue::EndOfFile => 0xFFFFFFFFu32,
            ClusterValue::Value(v) => v,
        }
    }
}

#[inline]
fn is_free_next(b: &Block) -> Cluster {
    match le_u32(&b[0x1EC..]) {
        0 | 0x1 | 0xFFFFFFFF => None,
        n => Some(n),
    }
}
#[inline]
fn is_free_clusters(b: &Block) -> Cluster {
    match le_u32(&b[0x1E8..]) {
        0xFFFFFFFF => None,
        n => Some(n),
    }
}
fn new_16(b: &Block) -> Result<(FatVersion, u32, u32, Cluster, Cluster), DeviceError> {
    if bp_blocks_bytes(&b) != Block::SIZE {
        return Err(DeviceError::InvalidFileSystem);
    }
    let f = ((bp_root_entries(&b) * DIR_SIZE as u32) + (Block::SIZE as u32 - 0x1)) / Block::SIZE as u32;
    let r = bp_blocks_reserved(&b) + (bp_fat_entries(&b) * bp_fat_size(&b));
    Ok((FatVersion::Fat16(le_u16(&b[0x11..])), f + r, r, None, None))
}
fn new_32(b: &Block, dev: &Storage<impl BlockDevice>, lba: u32) -> Result<(FatVersion, u32, u32, Cluster, Cluster), DeviceError> {
    let (i, mut d) = (bp_fat32_info(&b) + lba, Cache::block_a());
    dev.read_single(&mut d, i)?;
    if le_u32(&d) != 0x41615252 || le_u32(&d[0x1E4..]) != 0x61417272 || le_u32(&d[0x1FC..]) != 0xAA550000 {
        return Err(DeviceError::InvalidFileSystem);
    }
    Ok((
        FatVersion::Fat32(i),
        bp_blocks_reserved(&b) + (bp_fat_entries(&b) * bp_fat_size(&b)),
        le_u32(&b[0x2C..]),
        is_free_next(&d),
        is_free_clusters(&d),
    ))
}
