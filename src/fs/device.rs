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
use core::convert::From;
use core::fmt::{self, Debug, Formatter};
use core::result::Result::{self, Err};
use core::slice::{from_raw_parts, from_raw_parts_mut};

use rpsp::io;

use crate::Slice;
use crate::fs::{Block, Cache, Volume};

pub enum DeviceError {
    // Standard IO Errors
    Timeout,
    NotAFile,
    NotFound,
    EndOfFile,
    NotReadable,
    NotWritable,
    UnexpectedEoF,
    NotADirectory,
    NonEmptyDirectory,

    // Hardware-ish/Io Errors
    Read,
    Write,
    BadData,
    NoSpace,
    Hardware(u8),

    // Validation Errors
    Overflow,
    InvalidIndex,
    InvalidOptions,

    // FileSystem Errors
    NameTooLong,
    InvalidChain,
    InvalidVolume,
    InvalidCluster,
    InvalidChecksum,
    InvalidPartition,
    InvalidFileSystem,
    UnsupportedFileSystem,
    UnsupportedVolume(u8),
}

pub struct Storage<B: BlockDevice> {
    dev: UnsafeCell<B>,
}

pub trait BlockDevice {
    fn blocks(&mut self) -> DevResult<u32>;
    fn write(&mut self, b: &[Block], start: u32) -> DevResult<()>;
    fn read(&mut self, b: &mut [Block], start: u32) -> DevResult<()>;

    #[inline]
    fn write_single(&mut self, b: &Block, start: u32) -> DevResult<()> {
        let v = unsafe { from_raw_parts(b, 1) };
        self.write(v, start)
    }
    #[inline]
    fn read_single(&mut self, b: &mut Block, start: u32) -> DevResult<()> {
        let v = unsafe { from_raw_parts_mut(b, 1) };
        self.read(v, start)
    }
}

pub type Error = io::Error<DeviceError>;

pub type DevError = io::Error<DeviceError>;
pub type DevResult<T> = Result<T, DeviceError>;

impl<B: BlockDevice> Storage<B> {
    #[inline]
    pub const fn new(dev: B) -> Storage<B> {
        Storage { dev: UnsafeCell::new(dev) }
    }

    #[inline]
    pub fn device(&self) -> &mut B {
        unsafe { &mut *self.dev.get() }
    }
    #[inline]
    pub fn root<'a>(&'a self) -> DevResult<Volume<'a, B>> {
        self.volume(0)
    }
    #[inline]
    pub fn write(&self, b: &[Block], start: u32) -> DevResult<()> {
        self.device().write(b, start)
    }
    #[inline]
    pub fn read(&self, b: &mut [Block], start: u32) -> DevResult<()> {
        self.device().read(b, start)
    }
    #[inline]
    pub fn write_single(&self, b: &Block, start: u32) -> DevResult<()> {
        self.device().write_single(b, start)
    }
    #[inline]
    pub fn read_single(&self, b: &mut Block, start: u32) -> DevResult<()> {
        self.device().read_single(b, start)
    }
    #[inline]
    pub fn volume<'a>(&'a self, index: usize) -> DevResult<Volume<'a, B>> {
        if index > 3 {
            return Err(DeviceError::NotFound);
        }
        let mut b = Cache::block_a();
        let _ = self.read_single(&mut b, 0)?;
        if b.read_u16(510) != 0xAA55 {
            return Err(DeviceError::InvalidPartition);
        }
        let i = 0x1BE + (16 * index);
        if i + 16 > Block::SIZE {
            return Err(DeviceError::NotFound);
        }
        if b.read_u8(i) & 0x7F != 0 {
            return Err(DeviceError::InvalidPartition);
        }
        match b.read_u8(i + 4) {
            // NOTE(sf): All these types could have FAT volumes. We'll work it
            //           out after, but we can at least take on more types.
            //           These are also the "parition type" flag when using parted/fdisk.
            0x6 | 0xB | 0xE | 0xC | 0x16 | 0x1B | 0x1E | 0x66 | 0x76 | 0x83 | 0x92 | 0x97 | 0x98 | 0x9A | 0xD0 | 0xE4 | 0xE6 | 0xEF | 0xF4 | 0xF6 => (),
            v => return Err(DeviceError::UnsupportedVolume(v)),
        }
        let (s, n) = (b.read_u32(i + 8), b.read_u32(i + 12));
        Volume::new(self, &mut b, s, n)
    }
}

impl Debug for DeviceError {
    #[cfg(feature = "debug")]
    #[inline]
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            DeviceError::Read => f.write_str("Read"),
            DeviceError::Write => f.write_str("Write"),
            DeviceError::Timeout => f.write_str("Timeout"),
            DeviceError::NotAFile => f.write_str("NotAFile"),
            DeviceError::NotFound => f.write_str("NotFound"),
            DeviceError::EndOfFile => f.write_str("EndOfFile"),
            DeviceError::NotReadable => f.write_str("NotReadable"),
            DeviceError::NotWritable => f.write_str("NotWritable"),
            DeviceError::UnexpectedEoF => f.write_str("UnexpectedEoF"),
            DeviceError::NotADirectory => f.write_str("NotADirectory"),
            DeviceError::NonEmptyDirectory => f.write_str("NonEmptyDirectory"),
            DeviceError::BadData => f.write_str("BadData"),
            DeviceError::NoSpace => f.write_str("NoSpace"),
            DeviceError::Hardware(v) => f.debug_tuple("Hardware").field(v).finish(),
            DeviceError::Overflow => f.write_str("Overflow"),
            DeviceError::InvalidIndex => f.write_str("InvalidIndex"),
            DeviceError::InvalidOptions => f.write_str("InvalidOptions"),
            DeviceError::NameTooLong => f.write_str("NameTooLong"),
            DeviceError::InvalidChain => f.write_str("InvalidChain"),
            DeviceError::InvalidVolume => f.write_str("InvalidVolume"),
            DeviceError::InvalidCluster => f.write_str("InvalidCluster"),
            DeviceError::InvalidChecksum => f.write_str("InvalidChecksum"),
            DeviceError::InvalidPartition => f.write_str("InvalidPartition"),
            DeviceError::InvalidFileSystem => f.write_str("InvalidFileSystem"),
            DeviceError::UnsupportedVolume(v) => f.debug_tuple("UnsupportedVolume").field(v).finish(),
            DeviceError::UnsupportedFileSystem => f.write_str("UnsupportedFileSystem"),
        }
    }
    #[cfg(not(feature = "debug"))]
    #[inline]
    fn fmt(&self, _f: &mut Formatter<'_>) -> fmt::Result {
        Result::Ok(())
    }
}

impl From<DeviceError> for DevError {
    #[inline]
    fn from(v: DeviceError) -> DevError {
        match v {
            DeviceError::Read => Error::Read,
            DeviceError::Write => Error::Write,
            DeviceError::NoSpace => Error::NoSpace,
            DeviceError::Timeout => Error::Timeout,
            DeviceError::Overflow => Error::Overflow,
            DeviceError::NotAFile => Error::NotAFile,
            DeviceError::NotFound => Error::NotFound,
            DeviceError::EndOfFile => Error::EndOfFile,
            DeviceError::NotReadable => Error::NotReadable,
            DeviceError::NotWritable => Error::NotWritable,
            DeviceError::InvalidIndex => Error::InvalidIndex,
            DeviceError::NotADirectory => Error::NotADirectory,
            DeviceError::UnexpectedEoF => Error::UnexpectedEof,
            DeviceError::InvalidOptions => Error::InvalidOptions,
            DeviceError::NonEmptyDirectory => Error::NonEmptyDirectory,
            _ => Error::Other(v),
        }
    }
}
