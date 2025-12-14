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

#![no_std]
#![no_main]
#![no_implicit_prelude]
#![allow(internal_features)]
#![feature(core_intrinsics, panic_internals, unchecked_shifts)]

extern crate core;

use core::ptr::copy_nonoverlapping;
use core::slice::from_raw_parts;

pub mod frame;
pub mod fs;
pub mod hw;
mod inky;
pub mod pcf;
pub mod sd;

pub use self::inky::*;

trait Slice {
    fn as_ptr(&self) -> *const u8;

    #[inline]
    fn read_u8(&self, i: usize) -> u8 {
        unsafe { *self.as_ptr().add(i) }
    }
    #[inline]
    fn read_u16(&self, i: usize) -> u16 {
        unsafe { (*self.as_ptr().add(i) as u16) | (*self.as_ptr().add(i + 1) as u16).unchecked_shl(8) }
    }
    #[inline]
    fn read_u32(&self, i: usize) -> u32 {
        unsafe { (*self.as_ptr().add(i) as u32) | (*self.as_ptr().add(i + 1) as u32).unchecked_shl(8) | (*self.as_ptr().add(i + 2) as u32).unchecked_shl(16) | (*self.as_ptr().add(i + 3) as u32).unchecked_shl(24) }
    }
    fn read_slice(&self, i: usize, len: usize) -> &[u8] {
        unsafe { from_raw_parts(self.as_ptr().add(i), len) }
    }
}
trait SliceMut {
    fn as_mut_ptr(&mut self) -> *mut u8;

    #[inline]
    fn write_u8(&mut self, i: usize, v: u8) {
        unsafe { *self.as_mut_ptr().add(i) = v }
    }
    #[inline]
    fn write_u16(&mut self, i: usize, v: u16) {
        unsafe {
            *self.as_mut_ptr().add(i) = v as u8;
            *self.as_mut_ptr().add(i + 1) = v.unchecked_shr(8) as u8;
        }
    }
    #[inline]
    fn write_u32(&mut self, i: usize, v: u32) {
        unsafe {
            *self.as_mut_ptr().add(i) = v as u8;
            *self.as_mut_ptr().add(i + 1) = v.unchecked_shr(8) as u8;
            *self.as_mut_ptr().add(i + 2) = v.unchecked_shr(16) as u8;
            *self.as_mut_ptr().add(i + 3) = v.unchecked_shr(24) as u8;
        }
    }
    #[inline]
    fn write_from(&mut self, i: usize, v: &[u8]) {
        unsafe { copy_nonoverlapping(v.as_ptr(), self.as_mut_ptr().add(i), v.len()) };
    }
}

impl Slice for &[u8] {
    #[inline]
    fn as_ptr(&self) -> *const u8 {
        *self as *const [u8] as *const u8
    }
}

impl Slice for &mut [u8] {
    #[inline]
    fn as_ptr(&self) -> *const u8 {
        *self as *const [u8] as *const u8
    }
}
impl SliceMut for &mut [u8] {
    #[inline]
    fn as_mut_ptr(&mut self) -> *mut u8 {
        *self as *mut [u8] as *mut u8
    }
}

impl<const N: usize> Slice for [u8; N] {
    #[inline]
    fn as_ptr(&self) -> *const u8 {
        self as *const [u8] as *const u8
    }
}
impl<const N: usize> SliceMut for [u8; N] {
    #[inline]
    fn as_mut_ptr(&mut self) -> *mut u8 {
        self as *mut [u8] as *mut u8
    }
}
