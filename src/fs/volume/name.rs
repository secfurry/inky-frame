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
use core::cmp::{Ord, PartialEq};
use core::convert::AsRef;
use core::default::Default;
use core::hint::unreachable_unchecked;
use core::iter::Iterator;
use core::matches;
use core::ops::Deref;
use core::option::Option::{None, Some};
use core::ptr::{copy_nonoverlapping, write_bytes};
use core::result::Result::{Err, Ok};
use core::str::from_utf8_unchecked;

use crate::fs::{DevResult, DeviceError};
use crate::{Slice, SliceMut};

pub struct LongName([u8; LongName::SIZE]);
pub struct ShortName([u8; ShortName::SIZE]);
pub struct VolumeName([u8; VolumeName::SIZE]);

impl LongName {
    pub const SIZE: usize = 255usize;

    #[inline]
    pub const fn empty() -> LongName {
        LongName([0u8; LongName::SIZE])
    }

    #[inline]
    pub fn from_slice_truncate(v: &[u8]) -> LongName {
        let mut n = LongName([0u8; LongName::SIZE]);
        n.fill_inner(v);
        n
    }
    #[inline]
    pub fn from_slice(v: &[u8]) -> DevResult<LongName> {
        if v.len() > LongName::SIZE {
            Err(DeviceError::NameTooLong)
        } else {
            Ok(LongName::from_slice_truncate(v))
        }
    }
    #[inline]
    pub fn from_str_truncate(v: impl AsRef<str>) -> LongName {
        LongName::from_slice_truncate(v.as_ref().as_bytes())
    }
    #[inline]
    pub fn from_str(v: impl AsRef<str>) -> DevResult<LongName> {
        if v.as_ref().len() > LongName::SIZE {
            Err(DeviceError::NameTooLong)
        } else {
            Ok(LongName::from_str_truncate(v))
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.0.iter().position(|v| *v == 0).unwrap_or(LongName::SIZE)
    }
    #[inline]
    pub fn as_str(&self) -> &str {
        unsafe { from_utf8_unchecked(self.as_bytes()) }
    }
    pub fn lfn_size(&self) -> u8 {
        if self.is_empty() {
            return 0;
        }
        let mut r = self.len();
        if r < 13 {
            return 1; // Need NULL so anything UNDER 13
        }
        r += 1; // NULL CHAR
        let c = (r / 0xC) as u8;
        if (c * 0xC) as usize >= r {
            c
        } else {
            c + 1 // PAD
        }
    }
    #[inline]
    pub fn as_raw(&self) -> &[u8] {
        &self.0
    }
    #[inline]
    pub fn is_self(&self) -> bool {
        self.0.read_u8(0) == b'.' && self.0.read_u8(1) == 0
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.read_u8(0) == 0
    }
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        unsafe { self.0.get_unchecked(0..self.len()) }
    }
    #[inline]
    pub fn is_parent(&self) -> bool {
        self.0.read_u8(0) == b'.' && self.0.read_u8(1) == b'.' && self.0.read_u8(2) == 0
    }
    #[inline]
    pub fn fill(&mut self, v: impl AsRef<[u8]>) -> DevResult<()> {
        let b = v.as_ref();
        if b.len() > LongName::SIZE {
            return Err(DeviceError::NameTooLong);
        }
        self.fill_inner(b);
        Ok(())
    }
    #[inline]
    pub fn fill_str(&mut self, v: impl AsRef<str>) -> DevResult<()> {
        self.fill(v.as_ref().as_bytes())
    }

    #[inline]
    pub(super) fn reset(&mut self) {
        unsafe { write_bytes(self.0.as_mut_ptr(), 0, LongName::SIZE) };
    }
    pub(super) fn lfn(&mut self, b: &[u8]) -> u8 {
        if b.len() < 32 {
            return 0;
        }
        let v = ((b.read_u8(0) & 0x1F) as usize - 1) * 0xD;
        for i in 0..0xD {
            if i + v >= LongName::SIZE {
                break;
            }
            let c = b.read_u8(to_lfn(i));
            if c == 0 {
                break;
            }
            self.0.write_u8(v + i, c);
        }
        b.read_u8(13)
    }

    #[inline]
    fn fill_inner(&mut self, v: &[u8]) {
        unsafe { copy_nonoverlapping(v.as_ptr(), self.0.as_mut_ptr(), v.len().min(LongName::SIZE)) };
    }
}
impl ShortName {
    pub const SIZE: usize = 11usize;
    pub const SIZE_EXT: usize = 3usize;
    pub const SIZE_NAME: usize = 8usize;

    pub const SELF: ShortName = ShortName([0x2E, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20]);
    pub const PARENT: ShortName = ShortName([0x2E, 0x2E, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20]);

    #[inline]
    pub const fn empty() -> ShortName {
        ShortName([0x20u8; ShortName::SIZE])
    }

    #[inline]
    pub fn from_slice(v: &[u8]) -> ShortName {
        let mut n = ShortName([0x20u8; ShortName::SIZE]);
        n.sfn(v);
        n
    }
    #[inline]
    pub fn from_str(v: impl AsRef<str>) -> ShortName {
        ShortName::from_slice(v.as_ref().as_bytes())
    }

    #[inline]
    pub unsafe fn from_raw(v: &[u8]) -> ShortName {
        let mut n = ShortName([0x20u8; ShortName::SIZE]);
        unsafe { copy_nonoverlapping(v.as_ptr(), n.0.as_mut_ptr(), v.len().min(ShortName::SIZE)) };
        n
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.0
            .iter()
            .position(|&v| v == 0 || v == 0x20)
            .unwrap_or(ShortName::SIZE_NAME)
    }
    #[inline]
    pub fn name(&self) -> &str {
        unsafe { from_utf8_unchecked(self.0.get_unchecked(0..self.len())) }
    }
    #[inline]
    pub fn as_str(&self) -> &str {
        let i = self.extension_len();
        if i == 0 {
            self.name()
        } else {
            unsafe { from_utf8_unchecked(self.0.get_unchecked(0..ShortName::SIZE_NAME + i)) }
        }
    }
    #[inline]
    pub fn checksum(&self) -> u8 {
        let mut s = 0u8;
        for i in self.0.iter() {
            s = unsafe { (s & 1).unchecked_shl(7).wrapping_add(s.unchecked_shr(1) + *i) };
        }
        s
    }
    #[inline]
    pub fn as_raw(&self) -> &[u8] {
        &self.0
    }
    #[inline]
    pub fn is_self(&self) -> bool {
        self.0.eq(&ShortName::SELF.0)
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        matches!(self.0.read_u8(0), 0 | 0x20)
    }
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        unsafe { self.0.get_unchecked(0..self.len()) }
    }
    #[inline]
    pub fn extension(&self) -> &str {
        unsafe {
            from_utf8_unchecked(
                self.0
                    .get_unchecked(ShortName::SIZE_NAME..ShortName::SIZE_NAME + self.extension_len()),
            )
        }
    }
    #[inline]
    pub fn is_parent(&self) -> bool {
        self.0.eq(&ShortName::PARENT.0)
    }
    #[inline]
    pub fn fill(&mut self, v: impl AsRef<[u8]>) {
        self.sfn(v.as_ref());
    }
    #[inline]
    pub fn fill_str(&mut self, v: impl AsRef<str>) {
        self.sfn(v.as_ref().as_bytes());
    }

    #[inline]
    pub(super) fn fill_inner(&mut self, v: &[u8]) {
        unsafe {
            copy_nonoverlapping(
                v.as_ptr(),
                self.0.as_mut_ptr(),
                v.len().min(ShortName::SIZE),
            )
        }
    }

    fn sfn(&mut self, b: &[u8]) {
        // Look for SELF and PARENT names
        match b.len() {
            0 => return,
            1 if b.read_u8(0) == b'.' => {
                self.0.copy_from_slice(&ShortName::SELF.0);
                return;
            },
            2 if b.read_u8(0) == b'.' && b.read_u8(1) == b'.' => {
                self.0.copy_from_slice(&ShortName::PARENT.0);
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
                    v @ (1 | 2 | 3) => unsafe {
                        copy_nonoverlapping(
                            b.as_ptr().add(i + 1),
                            self.0.as_mut_ptr().add(ShortName::SIZE_NAME),
                            v,
                        )
                    },
                    _ => (),
                }
                i
            },
            None => b.len(),
        };
        if n == ShortName::SIZE_NAME {
            unsafe { copy_nonoverlapping(b.as_ptr(), self.0.as_mut_ptr(), ShortName::SIZE_NAME) };
        } else if n < ShortName::SIZE_NAME {
            unsafe {
                copy_nonoverlapping(b.as_ptr(), self.0.as_mut_ptr(), n);
                write_bytes(self.0.as_mut_ptr().add(n), 0x20, ShortName::SIZE_NAME - n);
            }
        } else {
            unsafe { copy_nonoverlapping(b.as_ptr(), self.0.as_mut_ptr(), ShortName::SIZE_NAME - 2) };
            // NOTE(sf): We don't really know the "number" of files
            //           contained, like we could read the dir first before
            //           setting this last bit (or after setting it).
            // NOTE(sf): ^ Would be a lot of work tbh.
            self.0.write_u8(ShortName::SIZE_NAME - 2, b'~');
            self.0.write_u8(ShortName::SIZE_NAME - 1, b'1');
        }
        // Flatten any spaces to '_'. We don't flatten spaces in extensions.
        // This only goes the length of the original string, so the empty
        // bytes stay as spaces.
        for i in 0..n.min(ShortName::SIZE_NAME) {
            let v = unsafe { self.0.get_unchecked_mut(i) };
            if *v == 0x20 {
                *v = b'_';
            }
        }
        // Last, make sure every a-z is capitalized.
        for i in unsafe { self.0.get_unchecked_mut(0..ShortName::SIZE_NAME) }.iter_mut() {
            *i = transform(*i)
        }
    }
    #[inline]
    fn extension_len(&self) -> usize {
        match self.0 {
            [.., 0x20, 0x20, 0x20] => 0,
            [.., 0x20, 0x20] => 1,
            [.., 0x20] => 2,
            _ => 3,
        }
    }
}
impl VolumeName {
    pub const SIZE: usize = 11usize;

    #[inline]
    pub const fn empty() -> VolumeName {
        VolumeName([0x20u8; VolumeName::SIZE])
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.0
            .iter()
            .position(|&v| v == 0 || v == 0x20)
            .unwrap_or(VolumeName::SIZE)
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        unsafe { from_utf8_unchecked(self.0.get_unchecked(0..self.len())) }
    }
    #[inline]
    pub fn as_raw(&self) -> &[u8] {
        &self.0
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        matches!(self.0.read_u8(0), 0 | 0x20)
    }
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        unsafe { self.0.get_unchecked(0..self.len()) }
    }

    #[inline]
    pub(super) fn new(f: bool, b: &[u8]) -> VolumeName {
        let mut v = VolumeName::empty();
        unsafe {
            copy_nonoverlapping(
                b.as_ptr().add(if f { 0x47 } else { 0x2B }),
                v.0.as_mut_ptr(),
                VolumeName::SIZE,
            )
        };
        v
    }
}

impl Clone for LongName {
    #[inline]
    fn clone(&self) -> LongName {
        LongName(self.0.clone())
    }
}
impl Deref for LongName {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &[u8] {
        &self.0
    }
}
impl Default for LongName {
    #[inline]
    fn default() -> LongName {
        LongName::empty()
    }
}
impl AsRef<str> for LongName {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
impl AsRef<[u8]> for LongName {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Clone for ShortName {
    #[inline]
    fn clone(&self) -> ShortName {
        ShortName(self.0.clone())
    }
}
impl Deref for ShortName {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &[u8] {
        &self.0
    }
}
impl Default for ShortName {
    #[inline]
    fn default() -> ShortName {
        ShortName::empty()
    }
}
impl AsRef<str> for ShortName {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
impl AsRef<[u8]> for ShortName {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Clone for VolumeName {
    #[inline]
    fn clone(&self) -> VolumeName {
        VolumeName(self.0.clone())
    }
}
impl Deref for VolumeName {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &[u8] {
        &self.0
    }
}
impl Default for VolumeName {
    #[inline]
    fn default() -> VolumeName {
        VolumeName::empty()
    }
}
impl AsRef<str> for VolumeName {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
impl AsRef<[u8]> for VolumeName {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl PartialEq for ShortName {
    #[inline]
    fn eq(&self, other: &ShortName) -> bool {
        self.0.eq(&other.0)
    }
}
impl PartialEq<str> for ShortName {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self.eq(other.as_bytes())
    }
}
impl PartialEq<[u8]> for ShortName {
    #[inline]
    fn eq(&self, other: &[u8]) -> bool {
        self.0.eq(&ShortName::from_slice(other).0)
    }
}

impl PartialEq for LongName {
    #[inline]
    fn eq(&self, other: &LongName) -> bool {
        self.0.eq(&other.0)
    }
}
impl PartialEq<str> for LongName {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self.eq(other.as_bytes())
    }
}
impl PartialEq<[u8]> for LongName {
    #[inline]
    fn eq(&self, other: &[u8]) -> bool {
        self.as_bytes().eq(other)
    }
}

#[inline]
pub(super) fn to_lfn(v: usize) -> usize {
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
        _ => unsafe { unreachable_unchecked() },
    }
}

#[inline]
fn transform(v: u8) -> u8 {
    match v {
        0x00..=0x1F | 0x7F..=0xFF | 0x3A..=0x3F | 0x20 | 0x22 | 0x2A | 0x2B | 0x2C | 0x2F | 0x5B | 0x5C | 0x5D | 0x7C => b'_',
        v @ 0x61..=0x7A => v - 0x20, // a - z
        v => v,
    }
}

#[cfg(feature = "debug")]
mod display {
    extern crate core;

    use core::fmt::{Debug, Display, Formatter, Result, Write};
    use core::iter::Iterator;
    use core::result::Result::Ok;

    use crate::fs::{LongName, ShortName, VolumeName};

    impl Debug for LongName {
        #[inline]
        fn fmt(&self, f: &mut Formatter<'_>) -> Result {
            f.write_str(self.as_str())
        }
    }
    impl Display for LongName {
        #[inline]
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
        #[inline]
        fn fmt(&self, f: &mut Formatter<'_>) -> Result {
            f.write_str(self.as_str())
        }
    }
    impl Display for VolumeName {
        #[inline]
        fn fmt(&self, f: &mut Formatter<'_>) -> Result {
            f.write_str(self.as_str())
        }
    }
}
