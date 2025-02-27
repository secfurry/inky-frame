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

use core::cell::UnsafeCell;
use core::clone::Clone;
use core::cmp::{self, PartialEq};
use core::convert::AsRef;
use core::default::Default;
use core::iter::Iterator;
use core::marker::Sync;
use core::mem::forget;
use core::ops::{Deref, DerefMut, Drop};
use core::option::Option::{None, Some};
use core::ptr::NonNull;
use core::result::Result::{self, Err, Ok};
use core::str::from_utf8_unchecked;
use core::unreachable;

use rpsp::locks::Spinlock30;

use crate::fs::{DeviceError, FatVersion};

// Shared References are annoying, but this is the best way to handle this as
// the Pico does not like this struct when it's 255 bytes in size, so we'll
// just share it and pray there's never any concurrent access lol.
//
// Actually, we'll use Spinlock30 to sync this. So well 'own' the Spinlock when
// we do 'LongNamePtr::new' and release it when the 'LongNamePtr' object is
// dropped.
static CACHE: Cache = Cache::new();

pub struct LongName(pub(super) [u8; LongName::SIZE]);
pub struct ShortName(pub(super) [u8; ShortName::SIZE]);
pub struct VolumeName(pub(super) [u8; VolumeName::SIZE]);

pub(super) struct LongNamePtr(NonNull<LongName>);

struct Cache(UnsafeCell<LongName>);

impl Cache {
    #[inline(always)]
    const fn new() -> Cache {
        Cache(UnsafeCell::new(LongName::empty()))
    }
}
impl LongName {
    pub const SIZE: usize = 0xFFusize;

    #[inline(always)]
    pub const fn empty() -> LongName {
        LongName([0u8; LongName::SIZE])
    }

    #[inline]
    pub fn from_slice_truncate(v: &[u8]) -> LongName {
        let mut n = LongName([0x20u8; LongName::SIZE]);
        n.fill_inner(v);
        n
    }
    #[inline]
    pub fn from_str_truncate(v: impl AsRef<str>) -> LongName {
        LongName::from_slice_truncate(v.as_ref().as_bytes())
    }
    #[inline]
    pub fn from_slice(v: &[u8]) -> Result<LongName, DeviceError> {
        if v.len() > LongName::SIZE {
            Err(DeviceError::NameTooLong)
        } else {
            Ok(LongName::from_slice_truncate(v))
        }
    }
    #[inline]
    pub fn from_str(v: impl AsRef<str>) -> Result<LongName, DeviceError> {
        if v.as_ref().len() > LongName::SIZE {
            Err(DeviceError::NameTooLong)
        } else {
            Ok(LongName::from_str_truncate(v))
        }
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.0.iter().position(|v| *v == 0).unwrap_or(LongName::SIZE)
    }
    #[inline(always)]
    pub fn as_str(&self) -> &str {
        unsafe { from_utf8_unchecked(&self.0[0..self.0.iter().position(|v| *v == 0).unwrap_or(LongName::SIZE)]) }
    }
    pub fn lfn_size(&self) -> u8 {
        let mut r = self.len();
        if r <= 13 {
            return 1;
        }
        r += 1; // NULL CHAR
        let c = r / 0xC;
        if (c * 0xC) == r {
            return c as u8;
        }
        if r > 0xC {
            r += 1; // ADD PAD
        }
        (r / 0xC) as u8 + 1
    }
    #[inline(always)]
    pub fn is_self(&self) -> bool {
        self.0[0] == b'.' && self.0[1] == 0
    }
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.0[0] == 0
    }
    #[inline(always)]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0[0..self.0.iter().position(|v| *v == 0).unwrap_or(LongName::SIZE)]
    }
    #[inline(always)]
    pub fn is_parent(&self) -> bool {
        self.0[0] == b'.' && self.0[1] == b'.' && self.0[2] == 0
    }
    #[inline]
    pub fn fill(&mut self, v: &[u8]) -> Result<(), DeviceError> {
        if v.len() > LongName::SIZE {
            return Err(DeviceError::NameTooLong);
        }
        self.fill_inner(v);
        Ok(())
    }
    #[inline]
    pub fn fill_str(&mut self, v: impl AsRef<str>) -> Result<(), DeviceError> {
        self.fill(v.as_ref().as_bytes())
    }

    #[inline(always)]
    pub(super) fn reset(&mut self) {
        self.0.fill(0)
    }
    pub(super) fn fill_lfn(&mut self, b: &[u8]) -> u8 {
        if b.len() < 0x20 {
            return 0u8;
        }
        let v = ((b[0] & 0x1F) as usize - 1) * 0xD;
        for i in 0..0xD {
            if i + v >= LongName::SIZE {
                break;
            }
            let c = b[LongName::pos_to_lfn(i)];
            if c == 0 {
                break;
            }
            self.0[v + i] = c;
        }
        b[0xD]
    }

    #[inline(always)]
    pub(super) fn pos_to_lfn(v: usize) -> usize {
        match v {
            0 => 1,
            1 => 3,
            2 => 5,
            3 => 7,
            4 => 9,
            5 => 14,
            6 => 16,
            7 => 18,
            8 => 20,
            9 => 22,
            10 => 24,
            11 => 28,
            12 => 30,
            _ => unreachable!(),
        }
    }

    #[inline]
    fn fill_inner(&mut self, v: &[u8]) {
        if v.len() < LongName::SIZE {
            self.0[0..v.len()].copy_from_slice(v)
        } else {
            self.0.copy_from_slice(&v[0..LongName::SIZE]);
        }
    }
}
impl ShortName {
    pub const SIZE: usize = 0xBusize;
    pub const SIZE_EXT: usize = 0x3usize;
    pub const SIZE_NAME: usize = 0x8usize;

    pub const SELF: ShortName = ShortName([0x2E, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20]);
    pub const PARENT: ShortName = ShortName([0x2E, 0x2E, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20]);

    #[inline]
    pub fn empty() -> ShortName {
        ShortName([0x20u8; ShortName::SIZE])
    }
    #[inline]
    pub fn from_slice(v: &[u8]) -> ShortName {
        let mut n = ShortName([0x20u8; ShortName::SIZE]);
        n.fill(v);
        n
    }
    #[inline]
    pub fn from_str(v: impl AsRef<str>) -> ShortName {
        ShortName::from_slice(v.as_ref().as_bytes())
    }

    #[inline]
    pub fn name(&self) -> &str {
        unsafe {
            from_utf8_unchecked(
                &self.0[0..self.0[0..ShortName::SIZE_NAME]
                    .iter()
                    .position(|v| *v == 0x20)
                    .unwrap_or(ShortName::SIZE_NAME)],
            )
        }
    }
    #[inline]
    pub fn as_str(&self) -> &str {
        let i = self.extension_len();
        if i == 0 {
            self.name()
        } else {
            unsafe { from_utf8_unchecked(&self.0[0..ShortName::SIZE_NAME + i]) }
        }
    }
    #[inline]
    pub fn checksum(&self) -> u8 {
        let mut s = 0u8;
        for i in self.0.iter() {
            s = ((s & 1) << 7).wrapping_add((s >> 1) + *i);
        }
        s
    }
    #[inline(always)]
    pub fn is_self(&self) -> bool {
        self.0.eq(&ShortName::SELF.0)
    }
    #[inline(always)]
    pub fn is_parent(&self) -> bool {
        self.0.eq(&ShortName::PARENT.0)
    }
    #[inline(always)]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
    #[inline(always)]
    pub fn extension(&self) -> &str {
        unsafe { from_utf8_unchecked(&self.0[ShortName::SIZE_NAME..ShortName::SIZE_NAME + self.extension_len()]) }
    }
    #[inline(always)]
    pub fn fill(&mut self, v: &[u8]) {
        ShortName::to_sfn(v, self);
    }
    #[inline]
    pub fn fill_str(&mut self, v: impl AsRef<str>) {
        self.fill(v.as_ref().as_bytes());
    }

    #[inline(always)]
    pub(super) fn fill_inner(&mut self, v: &[u8]) {
        self.0.copy_from_slice(&v[0..ShortName::SIZE])
    }

    #[inline(always)]
    fn transform_char(v: u8) -> u8 {
        match v {
            0x00..=0x1F | 0x20 | 0x22 | 0x2A | 0x2B | 0x2C | 0x2F | 0x3A | 0x3B | 0x3C | 0x3D | 0x3E | 0x3F | 0x5B | 0x5C | 0x5D | 0x7C => b'+',
            v if v >= b'a' && v <= b'z' => v - 0x20,
            v => v,
        }
    }
    fn to_sfn(b: &[u8], sfn: &mut ShortName) {
        // Look for SELF and PARENT names
        match b.len() {
            0 => return,
            1 if b[0] == b'.' => {
                sfn.0.copy_from_slice(&ShortName::SELF.0);
                return;
            },
            2 if b[0] == b'.' && b[1] == b'.' => {
                sfn.0.copy_from_slice(&ShortName::PARENT.0);
                return;
            },
            _ => (),
        }
        // Extract the extension first, if there is one.
        // 'n' is the end of the 'name'.
        let n = match b.iter().rposition(|v| *v == b'.') {
            Some(i) => {
                // We drop any chars more than 3 for the extension.
                match b.len().saturating_sub(i + 1) {
                    0 => (),
                    1 => sfn.0[ShortName::SIZE_NAME] = b[i + 1],
                    2 => {
                        sfn.0[ShortName::SIZE_NAME] = b[i + 1];
                        sfn.0[ShortName::SIZE_NAME + 1] = b[i + 2];
                    },
                    _ => sfn.0[ShortName::SIZE_NAME..ShortName::SIZE].copy_from_slice(&b[i + 1..i + 4]),
                }
                i
            },
            None => b.len(),
        };
        if n < ShortName::SIZE_NAME {
            sfn.0[0..n].copy_from_slice(&b[0..n]);
            sfn.0[n..ShortName::SIZE_NAME].fill(0x20)
        } else if n == ShortName::SIZE_NAME {
            sfn.0[0..ShortName::SIZE_NAME].copy_from_slice(&b[0..ShortName::SIZE_NAME])
        } else {
            sfn.0[0..ShortName::SIZE_NAME - 2].copy_from_slice(&b[0..ShortName::SIZE_NAME - 2]);
            // NOTE(sf): We don't really know the "number" of files contained, like
            //           we could read the dir first before setting this last bit
            //           (or after setting it).
            // NOTE(sf): ^ Would be a lot of work tbh.
            sfn.0[ShortName::SIZE_NAME - 2] = b'~';
            sfn.0[ShortName::SIZE_NAME - 1] = b'1';
        }
        // Flatten any spaces to '_'. We don't flatten spaces in extensions.
        // This only goes the length of the original string, so the empty
        // bytes stay as spaces.
        for i in 0..cmp::min(n, ShortName::SIZE_NAME) {
            if sfn.0[i] == 0x20 {
                sfn.0[i] = b'_'
            }
        }
        // Last, make sure every a-z is capitalized.
        for i in sfn.0[0..ShortName::SIZE_NAME].iter_mut() {
            *i = ShortName::transform_char(*i)
        }
    }

    #[inline(always)]
    fn extension_len(&self) -> usize {
        match (
            self.0[ShortName::SIZE_NAME],
            self.0[ShortName::SIZE_NAME + 1],
            self.0[ShortName::SIZE_NAME + 2],
        ) {
            (0x20, 0x20, 0x20) => 0,
            (_, 0x20, 0x20) => 1,
            (_, _, 0x20) => 2,
            (..) => 3,
        }
    }
}
impl VolumeName {
    pub const SIZE: usize = 0xBusize;

    #[inline]
    pub fn empty() -> VolumeName {
        VolumeName([0x20u8; VolumeName::SIZE])
    }

    #[inline]
    pub(super) fn from_slice(f: &FatVersion, v: &[u8]) -> VolumeName {
        let i = match f {
            FatVersion::Fat16(_) => 0x2B,
            FatVersion::Fat32(_) => 0x47,
        };
        let mut n = VolumeName([0x20u8; VolumeName::SIZE]);
        if (v.len().saturating_sub(i)) < VolumeName::SIZE {
            n.0[0..(v.len().saturating_sub(i))].copy_from_slice(v)
        } else {
            n.0.copy_from_slice(&v[i..VolumeName::SIZE + i]);
        }
        n
    }

    #[inline(always)]
    pub fn as_str(&self) -> &str {
        unsafe { from_utf8_unchecked(&self.0[0..self.0.iter().position(|v| *v == 0x20).unwrap_or(VolumeName::SIZE)]) }
    }
    #[inline(always)]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}
impl LongNamePtr {
    #[inline(always)]
    pub(super) fn new() -> LongNamePtr {
        let c = Spinlock30::claim();
        // Claim and 'forget' the lock.
        forget(c);
        // Now nobody else can create this struct until we drop it.
        LongNamePtr(unsafe { NonNull::new_unchecked(CACHE.0.get()) })
    }
}

impl PartialEq for ShortName {
    #[inline(always)]
    fn eq(&self, other: &ShortName) -> bool {
        self.0.eq(&other.0)
    }
}
impl PartialEq<str> for ShortName {
    #[inline(always)]
    fn eq(&self, other: &str) -> bool {
        self.eq(other.as_bytes())
    }
}
impl PartialEq<[u8]> for ShortName {
    #[inline]
    fn eq(&self, other: &[u8]) -> bool {
        let mut v = ShortName::empty();
        v.fill(other);
        v.0.eq(&self.0)
    }
}

impl PartialEq for LongName {
    #[inline(always)]
    fn eq(&self, other: &LongName) -> bool {
        self.0.eq(&other.0)
    }
}
impl PartialEq<str> for LongName {
    #[inline(always)]
    fn eq(&self, other: &str) -> bool {
        self.eq(other.as_bytes())
    }
}
impl PartialEq<[u8]> for LongName {
    #[inline(always)]
    fn eq(&self, other: &[u8]) -> bool {
        self.as_bytes().eq(other)
    }
}

impl Deref for LongName {
    type Target = [u8];

    #[inline(always)]
    fn deref(&self) -> &[u8] {
        self.as_bytes()
    }
}
impl AsRef<str> for LongName {
    #[inline(always)]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Clone for ShortName {
    #[inline(always)]
    fn clone(&self) -> ShortName {
        ShortName(self.0.clone())
    }
}
impl Deref for ShortName {
    type Target = [u8];

    #[inline(always)]
    fn deref(&self) -> &[u8] {
        &self.0
    }
}
impl AsRef<str> for ShortName {
    #[inline(always)]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Clone for VolumeName {
    #[inline(always)]
    fn clone(&self) -> VolumeName {
        VolumeName(self.0.clone())
    }
}
impl Deref for VolumeName {
    type Target = [u8];

    #[inline(always)]
    fn deref(&self) -> &[u8] {
        self.as_bytes()
    }
}
impl Default for VolumeName {
    #[inline(always)]
    fn default() -> VolumeName {
        VolumeName::empty()
    }
}
impl AsRef<str> for VolumeName {
    #[inline(always)]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Drop for LongNamePtr {
    #[inline(always)]
    fn drop(&mut self) {
        unsafe { Spinlock30::free() }
    }
}
impl Deref for LongNamePtr {
    type Target = LongName;

    #[inline(always)]
    fn deref(&self) -> &LongName {
        unsafe { self.0.as_ref() }
    }
}
impl DerefMut for LongNamePtr {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut LongName {
        unsafe { &mut *self.0.as_ptr() }
    }
}

unsafe impl Sync for Cache {}

#[cfg(feature = "debug")]
mod display {
    extern crate core;

    use core::fmt::{Debug, Display, Formatter, Result, Write};
    use core::iter::Iterator;
    use core::result::Result::Ok;

    use crate::fs::{LongName, ShortName, VolumeName};

    impl Debug for LongName {
        #[inline(always)]
        fn fmt(&self, f: &mut Formatter<'_>) -> Result {
            f.write_str(self.as_str())
        }
    }
    impl Display for LongName {
        #[inline(always)]
        fn fmt(&self, f: &mut Formatter<'_>) -> Result {
            f.write_str(self.as_str())
        }
    }

    impl Debug for ShortName {
        fn fmt(&self, f: &mut Formatter<'_>) -> Result {
            for (i, v) in self.0.iter().enumerate() {
                if i == 8 {
                    f.write_char('.')?;
                }
                if *v == 0x20 {
                    f.write_char('.')?;
                } else {
                    f.write_char(*v as _)?;
                }
            }
            Ok(())
        }
    }
    impl Display for ShortName {
        #[inline]
        fn fmt(&self, f: &mut Formatter<'_>) -> Result {
            f.write_str(self.name())?;
            f.write_char('.')?;
            f.write_str(self.extension())
        }
    }

    impl Debug for VolumeName {
        #[inline(always)]
        fn fmt(&self, f: &mut Formatter<'_>) -> Result {
            f.write_str(self.as_str())
        }
    }
    impl Display for VolumeName {
        #[inline(always)]
        fn fmt(&self, f: &mut Formatter<'_>) -> Result {
            f.write_str(self.as_str())
        }
    }
}
