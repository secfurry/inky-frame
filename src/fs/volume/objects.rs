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
use core::convert::From;
use core::marker::Copy;
use core::mem::transmute;
use core::num::NonZeroU32;
use core::ops::Deref;
use core::option::Option::{self, None, Some};

pub type Cluster = Option<ClusterIndex>;

pub struct FatVersion(u32);
pub struct ClusterIndex(NonZeroU32);

impl FatVersion {
    #[inline]
    pub(super) const fn new_16(v: u16) -> FatVersion {
        FatVersion(v as u32 | 0x80000000)
    }
    #[inline]
    pub(super) const fn new_32(v: u32) -> FatVersion {
        FatVersion(v)
    }

    #[inline]
    pub const fn sector(&self) -> u32 {
        self.0 & 0x7FFFFFFF
    }
    #[inline]
    pub const fn is_fat32(&self) -> bool {
        self.0 & 0x80000000 == 0
    }
}
impl ClusterIndex {
    pub const EMPTY: ClusterIndex = unsafe { ClusterIndex::new_unchecked(0xFFFFFFFCu32) };

    #[inline]
    pub const fn new(i: u32) -> Cluster {
        match NonZeroU32::new(i) {
            Some(v) => Some(ClusterIndex(v)),
            None => None,
        }
    }

    #[inline]
    pub const unsafe fn new_unchecked(v: u32) -> ClusterIndex {
        ClusterIndex(unsafe { NonZeroU32::new_unchecked(v) })
    }

    #[inline]
    pub const fn get(&self) -> u32 {
        self.0.get()
    }
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.0.get() == 0xFFFFFFFCu32
    }
    #[inline]
    pub const fn is_valid(&self) -> bool {
        self.0.get() > 2
    }

    #[inline]
    pub(super) fn sub_one(v: &mut Cluster) {
        *v = match v.as_mut() {
            Some(n) if n.0.get() == 1 => None,
            Some(n) => Some(unsafe { ClusterIndex::new_unchecked(n.0.get().saturating_sub(1)) }),
            None => None,
        }
    }
    #[inline]
    pub(super) fn add_one(v: &mut Cluster) {
        *v = match v.as_mut() {
            Some(n) => Some(ClusterIndex(n.0.saturating_add(1))),
            None => Some(unsafe { ClusterIndex::new_unchecked(1) }),
        };
    }
}

impl Copy for ClusterIndex {}
impl Clone for ClusterIndex {
    #[inline]
    fn clone(&self) -> ClusterIndex {
        *self
    }
}
impl Deref for ClusterIndex {
    type Target = u32;

    #[inline]
    fn deref(&self) -> &u32 {
        unsafe { transmute(&self.0) }
    }
}
impl From<u32> for ClusterIndex {
    #[inline]
    fn from(v: u32) -> ClusterIndex {
        match NonZeroU32::new(v) {
            Some(v) => ClusterIndex(v),
            None => ClusterIndex::EMPTY,
        }
    }
}
