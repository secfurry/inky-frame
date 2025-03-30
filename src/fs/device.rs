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
use core::default::Default;
use core::fmt::{self, Debug, Formatter};
use core::result::Result::{self, Err};
use core::slice::{from_raw_parts, from_raw_parts_mut};

use rpsp::io;

use crate::fs::{Block, Volume};

const PART_START: usize = 0x1BEusize;

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
    UnsupportedVolume(u8),
    UnsupportedFileSystem,
}

pub struct Storage<B: BlockDevice> {
    dev: UnsafeCell<B>,
}

pub type Error = io::Error<DeviceError>;

pub trait BlockDevice {
    fn blocks(&mut self) -> Result<u32, DeviceError>;
    fn write(&mut self, b: &[Block], start: u32) -> Result<(), DeviceError>;
    fn read(&mut self, b: &mut [Block], start: u32) -> Result<(), DeviceError>;

    #[inline]
    fn write_single(&mut self, b: &Block, start: u32) -> Result<(), DeviceError> {
        let v = unsafe { from_raw_parts(b, 1) };
        self.write(v, start)
    }
    #[inline]
    fn read_single(&mut self, b: &mut Block, start: u32) -> Result<(), DeviceError> {
        let v = unsafe { from_raw_parts_mut(b, 1) };
        self.read(v, start)
    }
}

impl<B: BlockDevice> Storage<B> {
    #[inline(always)]
    pub fn new(dev: B) -> Storage<B> {
        Storage { dev: UnsafeCell::new(dev) }
    }

    #[inline(always)]
    pub fn device(&self) -> &mut B {
        unsafe { &mut *self.dev.get() }
    }
    #[inline(always)]
    pub fn root<'a>(&'a self) -> Result<Volume<'a, B>, DeviceError> {
        self.volume(0)
    }
    pub fn volume<'a>(&'a self, index: usize) -> Result<Volume<'a, B>, DeviceError> {
        if index > 3 {
            return Err(DeviceError::NotFound);
        }
        let mut b = Block::default();
        self.read_single(&mut b, 0)?;
        if le_u16(&b[0x1FE..]) != 0xAA55 {
            return Err(DeviceError::InvalidPartition);
        }
        let i = PART_START + (0x10 * index);
        if i + 12 > Block::SIZE {
            return Err(DeviceError::NotFound);
        }
        if b[i] & 0x7F != 0 {
            return Err(DeviceError::InvalidPartition);
        }
        match b[i + 4] {
            // NOTE(sf): All these types could have FAT volumes. We'll work it
            // out after, but we can at least take on more types.
            0x6 | 0xB | 0xE | 0xC | 0x16 | 0x1B | 0x1E | 0x66 | 0x76 | 0x83 | 0x92 | 0x97 | 0x98 | 0x9A | 0xD0 | 0xE4 | 0xE6 | 0xEF | 0xF4 | 0xF6 => (),
            _ => return Err(DeviceError::UnsupportedVolume(b[i + 4])),
        }
        let (s, n) = (le_u32(&b[i + 8..]), le_u32(&b[i + 12..]));
        Volume::parse(self, b, s, n)
    }

    #[inline(always)]
    pub(super) fn write(&self, b: &[Block], start: u32) -> Result<(), DeviceError> {
        self.device().write(b, start)
    }
    #[inline(always)]
    pub(super) fn read(&self, b: &mut [Block], start: u32) -> Result<(), DeviceError> {
        self.device().read(b, start)
    }
    #[inline(always)]
    pub(super) fn write_single(&self, b: &Block, start: u32) -> Result<(), DeviceError> {
        self.device().write_single(b, start)
    }
    #[inline(always)]
    pub(super) fn read_single(&self, b: &mut Block, start: u32) -> Result<(), DeviceError> {
        self.device().read_single(b, start)
    }
}

impl From<DeviceError> for Error {
    #[inline]
    fn from(v: DeviceError) -> Error {
        match v {
            DeviceError::Read => Error::Read,
            DeviceError::Write => Error::Write,
            DeviceError::Timeout => Error::Timeout,
            DeviceError::EndOfFile => Error::EndOfFile,
            DeviceError::UnexpectedEoF => Error::UnexpectedEof,
            DeviceError::NoSpace => Error::NoSpace,
            DeviceError::NotAFile => Error::NotAFile,
            DeviceError::NotFound => Error::NotFound,
            DeviceError::Overflow => Error::Overflow,
            DeviceError::NotReadable => Error::NotReadable,
            DeviceError::NotWritable => Error::NotWritable,
            DeviceError::NotADirectory => Error::NotADirectory,
            DeviceError::NonEmptyDirectory => Error::NonEmptyDirectory,
            DeviceError::InvalidIndex => Error::InvalidIndex,
            DeviceError::InvalidOptions => Error::InvalidOptions,
            _ => Error::Other(v),
        }
    }
}

#[cfg(feature = "debug")]
impl Debug for DeviceError {
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
}
#[cfg(not(feature = "debug"))]
impl Debug for DeviceError {
    #[inline(always)]
    fn fmt(&self, _f: &mut Formatter<'_>) -> fmt::Result {
        Result::Ok(())
    }
}

#[inline(always)]
pub(super) fn le_u16(b: &[u8]) -> u16 {
    (b[0] as u16) | (b[1] as u16) << 8
}
#[inline(always)]
pub(super) fn le_u32(b: &[u8]) -> u32 {
    (b[0] as u32) | (b[1] as u32) << 8 | (b[2] as u32) << 16 | (b[3] as u32) << 24
}
#[inline]
pub(super) fn to_le_u16(v: u16, b: &mut [u8]) {
    b[0] = v as u8;
    b[1] = (v >> 8) as u8;
}
#[inline]
pub(super) fn to_le_u32(v: u32, b: &mut [u8]) {
    b[0] = v as u8;
    b[1] = (v >> 8) as u8;
    b[2] = (v >> 16) as u8;
    b[3] = (v >> 24) as u8;
}
