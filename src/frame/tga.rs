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

use core::clone::Clone;
use core::convert::From;
use core::fmt::{self, Debug, Formatter};
use core::iter::{IntoIterator, Iterator};
use core::marker::{Copy, PhantomData};
use core::ops::Deref;
use core::option::Option::{self, None, Some};
use core::result::Result::{self, Err, Ok};
use core::unreachable;

use rpsp::io::{Error, Read, Seek, SeekFrom};

use crate::frame::RGB;
use crate::fs::DeviceError;

const ATTRS_NONE: u8 = 0u8;
const ATTRS_TOP_LEFT: u8 = 0x08u8;
const ATTRS_GRAYSCALE: u8 = 0x01u8;
const ATTRS_TOP_RIGHT: u8 = 0x10u8;
const ATTRS_TRUE_COLOR: u8 = 0x02u8;
const ATTRS_BOTTOM_LEFT: u8 = 0x20u8;
const ATTRS_BOTTOM_RIGHT: u8 = 0x40u8;
const ATTRS_MAPPED_COLOR: u8 = 0x04u8;
const ATTRS_IS_COMPRESSED: u8 = 0x80u8;

pub enum ImageError {
    InvalidType(u8),
    Io(Error<DeviceError>),
    Empty,
    NotTGA,
    InvalidImage,
    InvalidColorMap,
}
pub enum Pixels<'a, R: Reader> {
    Raw(Raw<'a, R>),
    Compressed(Compressed<'a, R>),
}

pub struct Pixel {
    pub pos:   Point,
    pub color: u32,
}
pub struct Point {
    pub x: i32,
    pub y: i32,
}
pub struct Header {
    map:    Option<ColorMap>,
    bits:   u8,
    attrs:  u8,
    width:  u16,
    alpha:  u8,
    height: u16,
    origin: Point,
}
pub struct Raw<'a, R: Reader> {
    pos:   Point,
    image: TgaParser<'a, R>,
}
pub struct TgaParser<'a, R: Reader> {
    buf:    [u8; 0xFF],
    pos:    usize,
    avail:  usize,
    reader: &'a mut R,
    header: Header,
    _p:     PhantomData<*const ()>,
}
pub struct Compressed<'a, R: Reader> {
    cur:   u32,
    pos:   Point,
    skip:  u8,
    count: u8,
    image: TgaParser<'a, R>,
}

pub trait Reader: Read<DeviceError> + Seek<DeviceError> {}

struct ColorMap {
    len:   u16,
    pos:   u16,
    buf:   [u8; 4],
    bits:  u8,
    last:  Option<u32>,
    index: u16,
}

impl Pixel {
    #[inline(always)]
    pub fn rgb(&self) -> RGB {
        RGB::raw(self.color)
    }
    #[inline(always)]
    pub fn is_solid(&self) -> bool {
        (self.color >> 24) & 0xFF == 0xFF
    }
    #[inline(always)]
    pub fn is_transparent(&self) -> bool {
        (self.color >> 24) & 0xFF == 0
    }
}
impl Point {
    #[inline(always)]
    fn new(x: i32, y: i32) -> Point {
        Point { x, y }
    }

    fn next(&mut self, h: &Header) -> Option<Point> {
        if self.y < 0 || self.y >= h.height as i32 {
            return None;
        }
        let p = *self;
        self.x = self.x.saturating_add(1);
        if self.x >= h.width as i32 {
            self.y = if h.is_flipped() { self.y.saturating_sub(1) } else { self.y.saturating_add(1) };
            self.x = 0;
        }
        Some(p)
    }
}
impl Header {
    fn new(r: &mut impl Reader) -> Result<Header, ImageError> {
        let mut b = [0u8; 18];
        r.read_exact(&mut b)?;
        match b[16] {
            8 | 16 | 24 | 32 => (),
            _ => return Err(ImageError::InvalidImage),
        }
        if b[0] > 0 {
            r.seek(SeekFrom::Current(b[0] as i64))?;
        }
        Ok(Header {
            map:    ColorMap::new(&b)?,
            bits:   b[16] / 0x8,
            attrs:  attrs(b[2], b[17])?,
            alpha:  b[17] & 0xF,
            width:  le_u16(&b[12..]),
            height: le_u16(&b[14..]),
            origin: Point::new(le_u16(&b[8..]) as i32, le_u16(&b[10..]) as i32),
        })
    }

    #[inline(always)]
    pub fn alpha(&self) -> u8 {
        self.alpha
    }
    #[inline(always)]
    pub fn width(&self) -> i32 {
        self.width as i32
    }
    #[inline(always)]
    pub fn height(&self) -> i32 {
        self.height as i32
    }
    #[inline(always)]
    pub fn origin(&self) -> Point {
        self.origin
    }
    #[inline(always)]
    pub fn pixel_size(&self) -> u8 {
        self.bits
    }
    #[inline]
    pub fn image_start(&self) -> u64 {
        self.map
            .as_ref()
            .map(|m| m.pos as u64 + (m.len * m.bits as u16) as u64)
            .unwrap_or(0)
    }
    #[inline(always)]
    pub fn is_flipped(&self) -> bool {
        self.attrs & ATTRS_BOTTOM_LEFT != 0 || self.attrs & ATTRS_BOTTOM_RIGHT != 0
    }
    #[inline(always)]
    pub fn is_compressed(&self) -> bool {
        self.attrs & ATTRS_IS_COMPRESSED != 0
    }
}
impl ColorMap {
    fn new(b: &[u8]) -> Result<Option<ColorMap>, ImageError> {
        match b[1] {
            1 => (),
            0 => return Ok(None),
            _ => return Err(ImageError::InvalidColorMap),
        }
        let n = le_u16(&b[5..]);
        if n == 0 {
            return Ok(None);
        }
        let p = le_u16(&b[3..]);
        Ok(Some(ColorMap {
            len:   n,
            pos:   if p == 0 { 0x12u16 + b[0] as u16 } else { p },
            buf:   [0u8; 4],
            bits:  b[7] / 0x8,
            last:  None,
            index: 0u16,
        }))
    }

    fn index(&mut self, v: u32, r: &mut impl Reader) -> Result<Option<u32>, ImageError> {
        if v as u16 == self.index && self.last.is_some() {
            return Ok(self.last);
        }
        if v as u16 >= self.len {
            return Ok(None);
        }
        let i = v as u64 * self.bits as u64;
        if i > 0xFFFF {
            return Ok(None);
        }
        let l = r.stream_position()?;
        r.seek(SeekFrom::Start(i + self.pos as u64))?;
        let n = r.read(&mut self.buf)?;
        r.seek(SeekFrom::Start(l))?;
        let c = match self.bits {
            1 if n >= 1 => self.buf[0] as u32,
            2 if n >= 2 => le_u16(&self.buf) as u32,
            3 if n >= 3 => le_u24(&self.buf),
            4 if n >= 4 => le_u32(&self.buf),
            _ => return Ok(None),
        };
        self.index = v as u16;
        self.last.replace(c);
        Ok(Some(c))
    }
}
impl<R: Reader> TgaParser<'_, R> {
    #[inline]
    pub fn new<'a>(reader: &'a mut R) -> Result<TgaParser<'a, R>, ImageError> {
        let h = Header::new(reader)?;
        let s = h.image_start();
        if s > 0 {
            reader.seek(SeekFrom::Start(s))?;
        }
        Ok(TgaParser {
            reader,
            buf: [0u8; 0xFF],
            pos: 0usize,
            avail: 0usize,
            header: h,
            _p: PhantomData,
        })
    }

    #[inline(always)]
    pub fn header(&self) -> &Header {
        &self.header
    }

    #[inline]
    fn fix_alpha(&self, v: u32) -> u32 {
        if (v >> 24) & 0xFF > 0 {
            return v;
        } else if self.header.alpha == 0 {
            v | 0xFF000000
        } else {
            v
        }
    }
    #[inline]
    fn next(&mut self) -> Result<u32, ImageError> {
        match self.header.bits {
            1 => Ok(self.read(1)?[0] as u32),
            2 => Ok(le_u16(self.read(2)?) as u32),
            3 => Ok(le_u24(self.read(3)?)),
            4 => Ok(le_u32(self.read(4)?)),
            _ => unreachable!(),
        }
    }
    fn map(&mut self, c: u32) -> Result<u32, ImageError> {
        let n = match self.header.map.as_mut() {
            Some(m) => m.index(c, self.reader)?.unwrap_or(c),
            None => c,
        };
        // NOTE(sf): Reformat to "AARRGGBB" form.
        match self.header.bits {
            _ if self.header.attrs & 0x7 == ATTRS_NONE => Ok(n),
            // Replicate Grayscale into "FFVVVVVV" format where the V is the 0-255
            // value of Gray, since all hex colors matching all six are Gray colors.
            1 if self.header.attrs & ATTRS_GRAYSCALE != 0 => Ok((n & 0xFF) << 16 | (n & 0xFF) << 8 | n & 0xFF | 0xFF000000),
            // Expand the A-5-5-5 value. The first is the alpha, 1 or 0, so we can just multiply it.
            // Next we extract 5 bits and reposition them into the "AARRGGBB" format.
            2 => Ok((0xFF * ((n >> 15) & 0x1)) | (((n >> 10) & 0x1F) << 16) | (((n >> 5) << 0x1F) << 8) | (n & 0x1F)),
            3 => Ok(n | 0xFF000000), // Add FF alpha channel, since 3bit doesn't have one.
            _ => Ok(n),              // AAARRGGBB
        }
    }
    #[inline]
    fn read(&mut self, want: usize) -> Result<&[u8], ImageError> {
        self.refill(want)?;
        if self.avail.saturating_sub(self.pos) < want {
            Err(ImageError::Empty)
        } else {
            let n = self.pos;
            self.pos += want;
            Ok(&self.buf[n..n + want])
        }
    }
    fn refill(&mut self, want: usize) -> Result<usize, ImageError> {
        while self.avail.saturating_sub(self.pos) < want {
            if self.pos > 0 {
                self.buf.copy_within(self.pos.., 0);
                self.avail -= self.pos;
                self.pos = 0;
            }
            let n = self.reader.read(&mut self.buf[self.avail..])?;
            if n == 0 {
                break;
            }
            self.avail += n;
        }
        Ok(self.avail)
    }
}
impl<R: Reader> Compressed<'_, R> {
    fn decompress(&mut self) -> Option<Result<u32, ImageError>> {
        if self.count > 0 {
            self.count -= 1;
            return Some(Ok(self.cur));
        }
        if self.skip > 0 {
            self.skip -= 1;
            return Some(self.image.next().and_then(|v| self.image.map(v)));
        }
        let v = match self.image.read(1) {
            Err(e) => return Some(Err(e)),
            Ok(v) => v[0],
        };
        if v & 0x80 != 0 {
            self.count = (v & 0x7F) + 1;
            self.cur = match self.image.next() {
                Err(e) => return Some(Err(e)),
                Ok(v) => v,
            };
        } else {
            self.skip = (v & 0x7F) + 1;
        }
        self.decompress()
    }
}

impl<'a, R: Reader> IntoIterator for TgaParser<'a, R> {
    type IntoIter = Pixels<'a, R>;
    type Item = Result<Pixel, ImageError>;

    fn into_iter(self) -> Pixels<'a, R> {
        let y = if self.header.is_flipped() { self.header.height.saturating_sub(1) } else { 0 };
        if self.header.attrs & ATTRS_IS_COMPRESSED != 0 {
            Pixels::Compressed(Compressed {
                pos:   Point::new(0, y as i32),
                cur:   0u32,
                skip:  0u8,
                image: self,
                count: 0u8,
            })
        } else {
            Pixels::Raw(Raw {
                pos:   Point::new(0, y as i32),
                image: self,
            })
        }
    }
}

impl<R: Reader> Iterator for Raw<'_, R> {
    type Item = Result<Pixel, ImageError>;

    fn next(&mut self) -> Option<Result<Pixel, ImageError>> {
        let p = self.pos.next(&self.image.header)?;
        match self.image.next() {
            Ok(c) => Some(Ok(Pixel {
                pos:   p,
                color: match self.image.map(c) {
                    Err(e) => return Some(Err(e)),
                    Ok(v) => self.image.fix_alpha(v),
                },
            })),
            Err(ImageError::Empty) => None,
            Err(e) => Some(Err(e)),
        }
    }
}
impl<R: Reader> Iterator for Pixels<'_, R> {
    type Item = Result<Pixel, ImageError>;

    #[inline(always)]
    fn next(&mut self) -> Option<Result<Pixel, ImageError>> {
        match self {
            Pixels::Raw(v) => v.next(),
            Pixels::Compressed(v) => v.next(),
        }
    }
}
impl<R: Reader> Iterator for Compressed<'_, R> {
    type Item = Result<Pixel, ImageError>;

    fn next(&mut self) -> Option<Result<Pixel, ImageError>> {
        let p = self.pos.next(&self.image.header)?;
        match self.decompress()? {
            Ok(c) => Some(Ok(Pixel {
                pos:   p,
                color: self.image.fix_alpha(c),
            })),
            Err(ImageError::Empty) => None,
            Err(e) => Some(Err(e)),
        }
    }
}

impl Copy for Point {}
impl Clone for Point {
    #[inline(always)]
    fn clone(&self) -> Point {
        Point { x: self.x, y: self.y }
    }
}

impl Copy for Pixel {}
impl Clone for Pixel {
    #[inline(always)]
    fn clone(&self) -> Pixel {
        Pixel {
            pos:   self.pos.clone(),
            color: self.color,
        }
    }
}
impl Deref for Pixel {
    type Target = Point;

    #[inline(always)]
    fn deref(&self) -> &Point {
        &self.pos
    }
}

impl From<DeviceError> for ImageError {
    #[inline(always)]
    fn from(v: DeviceError) -> ImageError {
        ImageError::Io(Error::Other(v))
    }
}
impl From<Error<DeviceError>> for ImageError {
    #[inline(always)]
    fn from(v: Error<DeviceError>) -> ImageError {
        ImageError::Io(v)
    }
}

impl<D: Read<DeviceError> + Seek<DeviceError>> Reader for D {}

#[cfg(feature = "debug")]
impl Debug for ImageError {
    #[inline]
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ImageError::Io(v) => f.debug_tuple("Io").field(v).finish(),
            ImageError::InvalidType(v) => f.debug_tuple("InvalidType").field(v).finish(),
            ImageError::Empty => f.write_str("Empty"),
            ImageError::NotTGA => f.write_str("NotTGA"),
            ImageError::InvalidImage => f.write_str("InvalidImage"),
            ImageError::InvalidColorMap => f.write_str("InvalidColorMap"),
        }
    }
}
#[cfg(not(feature = "debug"))]
impl Debug for ImageError {
    #[inline(always)]
    fn fmt(&self, _f: &mut Formatter<'_>) -> fmt::Result {
        Ok(())
    }
}

#[inline]
fn le_u16(b: &[u8]) -> u16 {
    (b[0] as u16) | (b[1] as u16) << 8
}
#[inline]
fn le_u24(b: &[u8]) -> u32 {
    (b[0] as u32) | (b[1] as u32) << 8 | (b[2] as u32) << 16
}
#[inline]
fn le_u32(b: &[u8]) -> u32 {
    (b[0] as u32) | (b[1] as u32) << 8 | (b[2] as u32) << 16 | (b[3] as u32) << 24
}
#[inline]
fn attrs(a: u8, p: u8) -> Result<u8, ImageError> {
    let r = match a {
        _ if a & !0xB != 0 => return Err(ImageError::InvalidType(a)),
        0 => return Err(ImageError::NotTGA),
        1 if a & 0x8 != 0 => ATTRS_MAPPED_COLOR | ATTRS_IS_COMPRESSED,
        2 if a & 0x8 != 0 => ATTRS_TRUE_COLOR | ATTRS_IS_COMPRESSED,
        3 if a & 0x8 != 0 => ATTRS_GRAYSCALE | ATTRS_IS_COMPRESSED,
        _ if a & 0x8 != 0 => ATTRS_NONE | ATTRS_IS_COMPRESSED,
        1 => ATTRS_MAPPED_COLOR,
        2 => ATTRS_TRUE_COLOR,
        3 => ATTRS_GRAYSCALE,
        _ => ATTRS_NONE,
    };
    match (p & 0x30) >> 4 {
        0 => Ok(r | ATTRS_BOTTOM_LEFT),
        1 => Ok(r | ATTRS_BOTTOM_RIGHT),
        2 => Ok(r | ATTRS_TOP_LEFT),
        _ => Ok(r | ATTRS_TOP_RIGHT),
    }
}
