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
use core::convert::{From, Into};
use core::fmt::{self, Debug, Formatter};
use core::matches;
use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;
use core::result::Result::{self, Err, Ok};

use rpsp::Pico;
use rpsp::clock::Timer;
use rpsp::pin::gpio::Output;
use rpsp::pin::{Pin, PinID};
use rpsp::spi::{Spi, SpiBus, SpiIO};

use crate::fs::{Block, BlockDevice, DeviceError};

const CMD0: u8 = 0x00u8;
const CMD8: u8 = 0x08u8;
const CMD9: u8 = 0x09u8;
const CMD12: u8 = 0x0Cu8;
const CMD13: u8 = 0x0Du8;
const CMD17: u8 = 0x11u8;
const CMD18: u8 = 0x12u8;
const CMD24: u8 = 0x18u8;
const CMD25: u8 = 0x19u8;
const CMD55: u8 = 0x37u8;
const CMD58: u8 = 0x3Au8;
const CMD59: u8 = 0x3Bu8;
const CMDA23: u8 = 0x17u8;
const CMDA41: u8 = 0x29u8;

pub enum CardType {
    None,
    SD1,
    SD2,
    SDHC,
}
#[repr(u8)]
pub enum CardError {
    Timeout         = 0x0,
    ReadError       = 0x1,
    WriteError      = 0x2,
    InitFailed      = 0xA,
    InvalidDevice   = 0xB,
    InvalidOptions  = 0xC,
    InvalidResponse = 0xD,
    InvalidChecksum = 0xE,
}

pub struct CardInfo {
    v2:  bool,
    buf: [u8; 16],
}
pub struct Card<'a> {
    cs:       Pin<Output>,
    spi:      SpiBus<'a>,
    timer:    Timer,
    ver:      CardType,
    crc:      bool,
    attempts: u16,
}

struct Counter {
    t:   NonNull<Timer>,
    cur: u16,
    hit: u16,
}

impl Counter {
    #[inline(always)]
    fn reset(&mut self) {
        self.cur = 0;
    }
    #[inline]
    fn wait(&mut self) -> Result<(), CardError> {
        if self.cur >= self.hit {
            return Err(CardError::Timeout);
        }
        unsafe { (&*self.t.as_ptr()).sleep_us(10) };
        self.cur += 1;
        Ok(())
    }
}
impl Card<'_> {
    #[inline(always)]
    pub fn new<'a>(p: &Pico, cs: PinID, spi: impl Into<SpiBus<'a>>) -> Card<'a> {
        Card::new_crc(p, cs, spi, true)
    }
    #[inline]
    pub fn new_crc<'a>(p: &Pico, cs: PinID, spi: impl Into<SpiBus<'a>>, crc: bool) -> Card<'a> {
        Card {
            crc,
            cs: p.pin(cs).output_high(),
            spi: spi.into(),
            ver: CardType::None,
            timer: p.timer().clone(),
            attempts: 0xFFFu16,
        }
    }

    #[inline(always)]
    pub fn bus(&self) -> &Spi {
        &self.spi
    }
    #[inline(always)]
    pub fn blocks(&mut self) -> Result<u32, CardError> {
        Ok(self.info()?.blocks())
    }
    pub fn info(&mut self) -> Result<CardInfo, CardError> {
        if matches!(self.ver, CardType::None) {
            self.init()?;
        }
        let mut i = match self.ver {
            CardType::None => return Err(CardError::InitFailed),
            CardType::SD2 | CardType::SDHC => CardInfo::new(true),
            CardType::SD1 => CardInfo::new(false),
        };
        self.cs.low();
        let r = match self.cmd(CMD9, 0) {
            Ok(v) if v == 0 => self.read(&mut i.buf),
            Ok(_) => Err(CardError::InvalidResponse),
            Err(e) => Err(e),
        };
        self.cs.high();
        r.map(|_| i)
    }
    #[inline]
    pub fn write_block(&mut self, b: &Block, start: u32) -> Result<(), CardError> {
        if matches!(self.ver, CardType::None) {
            self.init()?;
        }
        self.cs.low();
        let r = self._write_block(b, self.index(start)?);
        self.cs.high();
        r
    }
    #[inline]
    pub fn read_block(&mut self, b: &mut Block, start: u32) -> Result<(), CardError> {
        if matches!(self.ver, CardType::None) {
            self.init()?;
        }
        self.cs.low();
        let r = self._read_block(b, self.index(start)?);
        self.cs.high();
        r
    }
    #[inline]
    pub fn write_blocks(&mut self, b: &[Block], start: u32) -> Result<(), CardError> {
        if matches!(self.ver, CardType::None) {
            self.init()?;
        }
        self.cs.low();
        let r = self._write_blocks(b, self.index(start)?);
        self.cs.high();
        r
    }
    #[inline]
    pub fn read_blocks(&mut self, b: &mut [Block], start: u32) -> Result<(), CardError> {
        if matches!(self.ver, CardType::None) {
            self.init()?;
        }
        self.cs.low();
        let r = self._read_blocks(b, self.index(start)?);
        self.cs.high();
        r
    }

    #[inline(always)]
    fn read_byte(&mut self) -> u8 {
        self.spi.transfer_single(0xFFu8)
    }
    #[inline(always)]
    fn counter(&mut self) -> Counter {
        Counter {
            t:   unsafe { NonNull::new_unchecked(&mut self.timer) },
            cur: 0,
            hit: self.attempts,
        }
    }
    #[inline]
    fn init(&mut self) -> Result<(), CardError> {
        self.cs.low();
        let r = self._init();
        self.cs.high();
        r
    }
    fn _init(&mut self) -> Result<(), CardError> {
        let mut c = self.counter();
        loop {
            match self.cmd(CMD0, 0) {
                Err(CardError::Timeout) => {
                    for _ in 0..0xFF {
                        self.spi.write_single(0xFFu8);
                    }
                },
                Err(e) => return Err(e),
                Ok(0x1) => break,
                Ok(_) => continue,
            }
            c.wait()?;
        }
        if self.crc && self.cmd(CMD59, 0x1)? != 0x1 {
            return Err(CardError::InvalidOptions);
        }
        c.reset();
        let a = loop {
            if self.cmd(CMD8, 0x1AA)? == 0x5 {
                self.ver = CardType::SD1;
                break 0;
            }
            let mut b = [0xFFu8, 0xFFu8, 0xFFu8, 0xFFu8];
            self.spi.transfer_in_place(&mut b);
            if b[3] == 0xAA {
                self.ver = CardType::SD2;
                break 0x40000000;
            }
            c.wait()?;
        };
        c.reset();
        while self.cmd_app(CMDA41, a)? != 0 {
            c.wait()?;
        }
        if !matches!(self.ver, CardType::SD2) {
            return Ok(());
        }
        if self.cmd(CMD58, 0)? != 0 {
            return Err(CardError::InvalidResponse);
        }
        let mut b = [0xFFu8, 0xFFu8, 0xFFu8, 0xFFu8];
        self.spi.transfer_in_place(&mut b);
        if (b[0] & 0xC0) == 0xC0 {
            self.ver = CardType::SDHC;
        }
        let _ = self.read_byte();
        Ok(())
    }
    #[inline]
    fn wait_busy(&mut self) -> Result<(), CardError> {
        let mut c = self.counter();
        loop {
            if self.read_byte() == 0xFF {
                return Ok(());
            }
            c.wait()?;
        }
    }
    #[inline(always)]
    fn index(&self, s: u32) -> Result<u32, CardError> {
        match self.ver {
            CardType::None => Err(CardError::InitFailed),
            CardType::SDHC => Ok(s),
            CardType::SD1 | CardType::SD2 => Ok(s * 0x200),
        }
    }
    fn read(&mut self, b: &mut [u8]) -> Result<(), CardError> {
        let mut c = self.counter();
        loop {
            match self.read_byte() {
                0xFF => (),
                0xFE => break,
                _ => return Err(CardError::ReadError),
            }
            c.wait()?;
        }
        self.spi.read_with(0xFFu8, b);
        let mut v = [0xFFu8, 0xFFu8];
        self.spi.transfer_in_place(&mut v);
        if !self.crc {
            return Ok(());
        }
        let c = u16::from_be_bytes(v);
        let a = crc_v16(b);
        if a != c { Err(CardError::InvalidChecksum) } else { Ok(()) }
    }
    fn cmd(&mut self, x: u8, arg: u32) -> Result<u8, CardError> {
        if x != CMD0 && x != CMD12 {
            self.wait_busy()?;
        }
        let mut b = [
            0x40u8 | x,
            (arg >> 24) as u8,
            (arg >> 16) as u8,
            (arg >> 8) as u8,
            arg as u8,
            0,
        ];
        b[5] = crc_v7(&b[0..5]);
        self.spi.write(&b);
        if x == CMD12 {
            let _ = self.read_byte();
        }
        let mut c = self.counter();
        loop {
            let v = self.read_byte();
            if (v & 0x80) == 0 {
                return Ok(v);
            }
            c.wait()?;
        }
    }
    fn write(&mut self, t: u8, b: &[u8]) -> Result<(), CardError> {
        self.spi.write_single(t);
        self.spi.write(b);
        let c = if self.crc { crc_v16(b).to_be_bytes() } else { [0xFFu8, 0xFFu8] };
        self.spi.write(&c);
        if self.read_byte() & 0x1F != 0x5 { Err(CardError::WriteError) } else { Ok(()) }
    }
    #[inline(always)]
    fn cmd_app(&mut self, x: u8, arg: u32) -> Result<u8, CardError> {
        self.cmd(CMD55, 0)?;
        self.cmd(x, arg)
    }
    fn _write_block(&mut self, b: &Block, i: u32) -> Result<(), CardError> {
        self.cmd(CMD24, i)?;
        self.write(0xFE, &b)?;
        self.wait_busy()?;
        if self.cmd(CMD13, 0)? != 0 {
            return Err(CardError::WriteError);
        }
        if self.read_byte() != 0 {
            return Err(CardError::WriteError);
        }
        Ok(())
    }
    #[inline(always)]
    fn _read_block(&mut self, b: &mut Block, i: u32) -> Result<(), CardError> {
        self.cmd(CMD17, i)?;
        self.read(b)
    }
    fn _write_blocks(&mut self, b: &[Block], i: u32) -> Result<(), CardError> {
        if b.len() == 1 {
            return self._write_block(&b[0], i);
        }
        self.cmd_app(CMDA23, b.len() as u32)?;
        self.wait_busy()?;
        self.cmd(CMD25, i)?;
        for v in b.iter() {
            self.wait_busy()?;
            self.write(0xFC, v)?;
        }
        self.wait_busy()?;
        self.spi.write_single(0xFDu8);
        Ok(())
    }
    fn _read_blocks(&mut self, b: &mut [Block], i: u32) -> Result<(), CardError> {
        if b.len() == 1 {
            return self._read_block(&mut b[0], i);
        }
        self.cmd(CMD18, i)?;
        for v in b.iter_mut() {
            self.read(v)?;
        }
        self.cmd(CMD12, 0)?;
        Ok(())
    }
}
impl CardInfo {
    #[inline(always)]
    fn new(v2: bool) -> CardInfo {
        CardInfo { v2, buf: [0u8; 16] }
    }

    #[inline(always)]
    pub fn crc(&self) -> u8 {
        self.buf[0xF] & 0xFF
    }
    #[inline]
    pub fn size(&self) -> u64 {
        if self.v2 {
            (self.device_size() as u64 + 1) * 0x200 * 0x400
        } else {
            (self.device_size() as u64 + 1) << (self.device_size_multiplier() as u64 * self.block_length() as u64 + 2)
        }
    }
    #[inline]
    pub fn blocks(&self) -> u32 {
        if self.v2 {
            (self.device_size() + 1) * 0x400
        } else {
            (self.device_size() + 1) << (self.device_size_multiplier() as u32 * self.block_length() as u32 + 7)
        }
    }
    #[inline(always)]
    pub fn is_v2(&self) -> bool {
        self.v2
    }
    #[inline(always)]
    pub fn block_length(&self) -> u8 {
        self.buf[0x5] & 0xF
    }
    #[inline(always)]
    pub fn device_size(&self) -> u32 {
        if self.v2 {
            (((self.buf[0x7] & 0x3F) as u32) << 8) | (((self.buf[0x8] & 0xFF) as u32) << 8) | ((self.buf[0x9] & 0xFF) as u32)
        } else {
            (((self.buf[0x6] & 0x3) as u32) << 8) | (((self.buf[0x7] & 0xFF) as u32) << 8) | (((self.buf[0x8] >> 0x6) & 0x3) as u32)
        }
    }
    #[inline(always)]
    pub fn device_size_multiplier(&self) -> u8 {
        if self.v2 { ((self.buf[0x9] & 0x3) << 1) | (self.buf[0xA] >> 0x7) } else { 0u8 }
    }
}

impl Deref for CardInfo {
    type Target = [u8];

    #[inline(always)]
    fn deref(&self) -> &[u8] {
        &self.buf
    }
}
impl DerefMut for CardInfo {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut [u8] {
        &mut self.buf
    }
}

impl BlockDevice for Card<'_> {
    #[inline(always)]
    fn blocks(&mut self) -> Result<u32, DeviceError> {
        Ok(self.blocks()?)
    }
    #[inline(always)]
    fn write(&mut self, b: &[Block], start: u32) -> Result<(), DeviceError> {
        Ok(self.write_blocks(b, start)?)
    }
    #[inline(always)]
    fn read(&mut self, b: &mut [Block], start: u32) -> Result<(), DeviceError> {
        Ok(self.read_blocks(b, start)?)
    }
    #[inline(always)]
    fn write_single(&mut self, b: &Block, start: u32) -> Result<(), DeviceError> {
        Ok(self.write_block(b, start)?)
    }
    #[inline(always)]
    fn read_single(&mut self, b: &mut Block, start: u32) -> Result<(), DeviceError> {
        Ok(self.read_block(b, start)?)
    }
}

impl From<CardError> for DeviceError {
    #[inline(always)]
    fn from(v: CardError) -> DeviceError {
        match v {
            CardError::Timeout => DeviceError::Timeout,
            CardError::ReadError => DeviceError::ReadError,
            CardError::WriteError => DeviceError::WriteError,
            CardError::InvalidChecksum => DeviceError::BadData,
            CardError::InvalidOptions => DeviceError::InvalidOptions,
            _ => DeviceError::Hardware(v as u8),
        }
    }
}

#[cfg(feature = "debug")]
impl Debug for CardError {
    #[inline]
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            CardError::Timeout => f.write_str("Timeout"),
            CardError::ReadError => f.write_str("ReadError"),
            CardError::WriteError => f.write_str("WriteError"),
            CardError::InitFailed => f.write_str("InitFailed"),
            CardError::InvalidDevice => f.write_str("InvalidDevice"),
            CardError::InvalidOptions => f.write_str("InvalidOptions"),
            CardError::InvalidResponse => f.write_str("InvalidResponse"),
            CardError::InvalidChecksum => f.write_str("InvalidChecksum"),
        }
    }
}
#[cfg(not(feature = "debug"))]
impl Debug for CardError {
    #[inline(always)]
    fn fmt(&self, _f: &mut Formatter<'_>) -> fmt::Result {
        Ok(())
    }
}

fn crc_v7(b: &[u8]) -> u8 {
    let mut r = 0u8;
    for i in b {
        let mut v = *i;
        for _ in 0..8 {
            r <<= 1;
            if ((v & 0x80) ^ (r & 0x80)) != 0 {
                r ^= 0x9;
            }
            v <<= 1;
        }
    }
    (r << 1) | 1
}
fn crc_v16(b: &[u8]) -> u16 {
    let mut r = 0u16;
    for i in b {
        r = ((r >> 8) & 0xFF) | (r << 8);
        r ^= *i as u16;
        r ^= (r & 0xFF) >> 4;
        r ^= r << 12;
        r ^= (r & 0xFF) << 5;
    }
    r
}
