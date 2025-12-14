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
use core::ops::Deref;
use core::ptr::{NonNull, write_bytes};
use core::result::Result::{self, Err, Ok};
use core::{matches, unreachable};

use rpsp::Board;
use rpsp::clock::Timer;
use rpsp::pin::gpio::Output;
use rpsp::pin::{Pin, PinID};
use rpsp::spi::{Spi, SpiBus, SpiIO};

use crate::fs::{Block, BlockDevice, DevResult, DeviceError};
use crate::{Slice, SliceMut};

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
    Read            = 0x1,
    Write           = 0x2,
    Timeout         = 0x0,
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
    cs:  Pin<Output>,
    clk: Timer,
    crc: bool,
    spi: SpiBus<'a>,
    ver: CardType,
}

struct Counter {
    c: u16,
    t: NonNull<Timer>,
}

impl Counter {
    const ATTEMPTS: u16 = 0x5FFFu16;

    #[inline]
    fn new(v: &mut Card<'_>) -> Counter {
        Counter {
            c: Counter::ATTEMPTS,
            t: unsafe { NonNull::new_unchecked(&mut v.clk) },
        }
    }

    #[inline]
    fn reset(&mut self) {
        self.c = Counter::ATTEMPTS;
    }
    #[inline]
    fn wait(&mut self) -> Result<(), CardError> {
        if self.c == 0 {
            return Err(CardError::Timeout);
        }
        unsafe { (&*self.t.as_ptr()).sleep_us(10) };
        self.c = self.c.saturating_sub(1);
        Ok(())
    }
}
impl CardInfo {
    #[inline]
    fn new(v2: bool) -> CardInfo {
        CardInfo { v2, buf: [0u8; 16] }
    }

    #[inline]
    pub fn crc(&self) -> u8 {
        self.buf.read_u8(15)
    }
    #[inline]
    pub fn size(&self) -> u64 {
        if self.v2 {
            (self.device_size() as u64 + 1) * 0x200 * 0x400
        } else {
            unsafe { (self.device_size() as u64 + 1).unchecked_shl((self.device_size_multiplier() as u64 * self.block_length() as u64 + 2) as u32) }
        }
    }
    #[inline]
    pub fn blocks(&self) -> u32 {
        if self.v2 {
            (self.device_size() + 1) * 0x400
        } else {
            unsafe { (self.device_size() + 1).unchecked_shl(self.device_size_multiplier() as u32 * self.block_length() as u32 + 7) }
        }
    }
    #[inline]
    pub fn is_v2(&self) -> bool {
        self.v2
    }
    #[inline]
    pub fn block_length(&self) -> u8 {
        self.buf.read_u8(5) & 0xF
    }
    #[inline]
    pub fn device_size(&self) -> u32 {
        if self.v2 {
            unsafe { ((self.buf.read_u8(7) & 0x3F) as u32).unchecked_shl(8) | (self.buf.read_u8(8) as u32).unchecked_shl(8) | (self.buf.read_u8(9) as u32) }
        } else {
            unsafe { ((self.buf.read_u8(6) & 0x03) as u32).unchecked_shl(8) | (self.buf.read_u8(7) as u32).unchecked_shl(8) | (self.buf.read_u8(8).unchecked_shr(6) & 0x3) as u32 }
        }
    }
    #[inline]
    pub fn device_size_multiplier(&self) -> u8 {
        if self.v2 {
            unsafe { (self.buf.read_u8(9) & 0x3).unchecked_shl(1) | self.buf.read_u8(10).unchecked_shr(7) }
        } else {
            0
        }
    }
}
impl<'a> Card<'a> {
    #[inline]
    pub fn new(p: &Board, cs: PinID, spi: impl Into<SpiBus<'a>>) -> Card<'a> {
        Card::new_crc(p, cs, spi, true)
    }
    #[inline]
    pub fn new_crc(p: &Board, cs: PinID, spi: impl Into<SpiBus<'a>>, crc: bool) -> Card<'a> {
        Card {
            crc,
            cs: p.pin(cs).output_high(),
            clk: p.timer().clone(),
            spi: spi.into(),
            ver: CardType::None,
        }
    }

    #[inline]
    pub fn bus(&self) -> &Spi {
        &self.spi
    }
    #[inline]
    pub fn blocks(&mut self) -> Result<u32, CardError> {
        Ok(self.info()?.blocks())
    }
    pub fn info(&mut self) -> Result<CardInfo, CardError> {
        if let CardType::None = self.ver {
            let _ = self.init()?;
        }
        let mut i = match self.ver {
            CardType::None => return Err(CardError::InitFailed),
            CardType::SD2 | CardType::SDHC => CardInfo::new(true),
            CardType::SD1 => CardInfo::new(false),
        };
        self.cs.low();
        let r = match self.cmd(CMD9, 0) {
            Ok(v) if v == 0 => {
                self.read(&mut i.buf)?;
                Ok(i)
            },
            Ok(_) => Err(CardError::InvalidResponse),
            Err(e) => Err(e),
        };
        self.cs.high();
        r
    }
    #[inline]
    pub fn write_block(&mut self, b: &Block, start: u32) -> Result<(), CardError> {
        if let CardType::None = self.ver {
            let _ = self.init()?;
        }
        self.cs.low();
        let r = self._write_block(b, self.index(start));
        self.cs.high();
        r
    }
    #[inline]
    pub fn read_block(&mut self, b: &mut Block, start: u32) -> Result<(), CardError> {
        if let CardType::None = self.ver {
            let _ = self.init()?;
        }
        self.cs.low();
        let r = self._read_block(b, self.index(start));
        self.cs.high();
        r
    }
    #[inline]
    pub fn write_blocks(&mut self, b: &[Block], start: u32) -> Result<(), CardError> {
        if let CardType::None = self.ver {
            let _ = self.init()?;
        }
        self.cs.low();
        let r = self._write_blocks(b, self.index(start));
        self.cs.high();
        r
    }
    #[inline]
    pub fn read_blocks(&mut self, b: &mut [Block], start: u32) -> Result<(), CardError> {
        if let CardType::None = self.ver {
            let _ = self.init()?;
        }
        self.cs.low();
        let r = self._read_blocks(b, self.index(start));
        self.cs.high();
        r
    }

    #[inline]
    fn read_byte(&mut self) -> u8 {
        self.spi.transfer_single(0xFFu8)
    }
    #[inline]
    fn index(&self, s: u32) -> u32 {
        match self.ver {
            CardType::SDHC => s,
            CardType::SD1 | CardType::SD2 => s * 0x200,
            CardType::None => unreachable!(), // Shouldn't be able to happen.
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
        let (mut c, mut e, mut o) = (Counter::new(self), 0x40u8, 0xFFu8);
        loop {
            // 0xF9F9F9F9 - Switch to SDR12 mode. Forces fast-boot on all cards
            //              so they startup properly.
            match self.cmd(CMD0, 0xF9F9F9F9) {
                Err(CardError::Timeout) if e == 0 => return Err(CardError::Timeout),
                Err(CardError::Timeout) => {
                    for _ in 0..0xFF {
                        self.spi.write_single(0xFFu8);
                    }
                    e = e.saturating_sub(1); // Timeout 64 times before giving up.
                    c.reset(); // Reset Counter
                },
                Err(e) => return Err(e),
                // NOTE(sf): Some SDCards do not respond with 'R1_IDLE_STATE' (1)
                //           when asked to go to IDLE. Instead, they respond with
                //           'R1_READY_STATE' (0) or 'UNKNOWN' (63) which I don't
                //           know what it means. But! If we keep going from here
                //           the SDCard works perfectly!! Fucking cheap shit..
                //
                //           Anyway, we don't break immediately on this, we let
                //           it cycle for 255 times before, just to be 1000% sure
                //           that it's one of those weird cards first.
                Ok(0x0 | 0x3F) if o == 0 => break,
                Ok(0x0 | 0x3F) => o = o.saturating_sub(1),
                Ok(0x1) => break,
                Ok(_) => (),
            }
            c.wait()?;
        }
        c.reset();
        if self.crc && self.cmd(CMD59, 0x1)? != 0x1 {
            return Err(CardError::InvalidOptions);
        }
        let mut b = [0xFFu8, 0xFFu8, 0xFFu8, 0xFFu8];
        let a = loop {
            if self.cmd(CMD8, 0x1AA)? == 0x5 {
                self.ver = CardType::SD1;
                break 0;
            }
            self.spi.transfer_in_place(&mut b);
            if b.read_u8(3) == 0xAA {
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
        // Reset the buffer
        unsafe { write_bytes(b.as_mut_ptr(), 0xFF, 4) };
        self.spi.transfer_in_place(&mut b);
        if b.read_u8(0) & 0xC0 == 0xC0 {
            self.ver = CardType::SDHC;
        }
        let _ = self.read_byte();
        Ok(())
    }
    #[inline]
    fn wait_busy(&mut self) -> Result<(), CardError> {
        let mut c = Counter::new(self);
        loop {
            if self.read_byte() == 0xFF {
                return Ok(());
            }
            c.wait()?;
        }
    }
    fn read(&mut self, b: &mut [u8]) -> Result<(), CardError> {
        let mut c = Counter::new(self);
        loop {
            match self.read_byte() {
                0xFF => (),
                0xFE => break,
                _ => return Err(CardError::Read),
            }
            c.wait()?;
        }
        self.spi.read_with(0xFFu8, b);
        let mut v = [0xFFu8, 0xFFu8];
        self.spi.transfer_in_place(&mut v);
        if !self.crc {
            return Ok(());
        }
        if u16::from_be_bytes(v) != crc_v16(b) { Err(CardError::InvalidChecksum) } else { Ok(()) }
    }
    fn cmd(&mut self, x: u8, arg: u32) -> Result<u8, CardError> {
        if x != CMD0 && x != CMD12 {
            self.wait_busy()?;
        }
        let mut b = unsafe {
            [
                0x40 | x,
                (arg.unchecked_shr(24)) as u8,
                (arg.unchecked_shr(16)) as u8,
                (arg.unchecked_shr(8)) as u8,
                arg as u8,
                0,
            ]
        };
        b.write_u8(5, crc_v7(b.read_slice(0, 5)));
        self.spi.write(&b);
        if x == CMD12 {
            let _ = self.read_byte();
        }
        let mut c = Counter::new(self);
        loop {
            let v = self.read_byte();
            if v & 0x80 == 0 {
                return Ok(v);
            }
            c.wait()?;
        }
    }
    #[inline]
    fn write(&mut self, t: u8, b: &[u8]) -> Result<(), CardError> {
        self.spi.write_single(t);
        self.spi.write(b);
        let c = if self.crc { crc_v16(b).to_be_bytes() } else { [0xFFu8, 0xFFu8] };
        self.spi.write(&c);
        if self.read_byte() & 0x1F != 0x5 { Err(CardError::Write) } else { Ok(()) }
    }
    #[inline]
    fn cmd_app(&mut self, x: u8, arg: u32) -> Result<u8, CardError> {
        self.cmd(CMD55, 0)?;
        self.cmd(x, arg)
    }
    fn _write_block(&mut self, b: &Block, i: u32) -> Result<(), CardError> {
        self.cmd(CMD24, i)?;
        self.write(0xFE, &b)?;
        self.wait_busy()?;
        if self.cmd(CMD13, 0)? != 0 {
            return Err(CardError::Write);
        }
        if self.read_byte() != 0 {
            return Err(CardError::Write);
        }
        Ok(())
    }
    #[inline]
    fn _read_block(&mut self, b: &mut Block, i: u32) -> Result<(), CardError> {
        self.cmd(CMD17, i)?;
        self.read(b)
    }
    fn _write_blocks(&mut self, b: &[Block], i: u32) -> Result<(), CardError> {
        if b.len() == 1 {
            return self._write_block(unsafe { b.get_unchecked(0) }, i);
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
            return self._read_block(unsafe { b.get_unchecked_mut(0) }, i);
        }
        self.cmd(CMD18, i)?;
        for v in b.iter_mut() {
            self.read(v)?;
        }
        self.cmd(CMD12, 0)?;
        Ok(())
    }
}

impl Deref for CardInfo {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &[u8] {
        &self.buf
    }
}

impl BlockDevice for Card<'_> {
    #[inline]
    fn blocks(&mut self) -> DevResult<u32> {
        Ok(self.blocks()?)
    }
    #[inline]
    fn write(&mut self, b: &[Block], start: u32) -> DevResult<()> {
        Ok(self.write_blocks(b, start)?)
    }
    #[inline]
    fn read(&mut self, b: &mut [Block], start: u32) -> DevResult<()> {
        Ok(self.read_blocks(b, start)?)
    }
    #[inline]
    fn write_single(&mut self, b: &Block, start: u32) -> DevResult<()> {
        Ok(self.write_block(b, start)?)
    }
    #[inline]
    fn read_single(&mut self, b: &mut Block, start: u32) -> DevResult<()> {
        Ok(self.read_block(b, start)?)
    }
}

impl From<CardError> for DeviceError {
    #[inline]
    fn from(v: CardError) -> DeviceError {
        match v {
            CardError::Read => DeviceError::Read,
            CardError::Write => DeviceError::Write,
            CardError::Timeout => DeviceError::Timeout,
            CardError::InvalidChecksum => DeviceError::BadData,
            CardError::InvalidOptions => DeviceError::InvalidOptions,
            _ => DeviceError::Hardware(v as u8),
        }
    }
}

impl Debug for CardError {
    #[cfg(feature = "debug")]
    #[inline]
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            CardError::Read => f.write_str("Read"),
            CardError::Write => f.write_str("Write"),
            CardError::Timeout => f.write_str("Timeout"),
            CardError::InitFailed => f.write_str("InitFailed"),
            CardError::InvalidDevice => f.write_str("InvalidDevice"),
            CardError::InvalidOptions => f.write_str("InvalidOptions"),
            CardError::InvalidResponse => f.write_str("InvalidResponse"),
            CardError::InvalidChecksum => f.write_str("InvalidChecksum"),
        }
    }
    #[cfg(not(feature = "debug"))]
    #[inline]
    fn fmt(&self, _f: &mut Formatter<'_>) -> fmt::Result {
        Ok(())
    }
}

fn crc_v7(b: &[u8]) -> u8 {
    let mut r = 0u8;
    for i in b.iter() {
        let mut v = *i;
        for _ in 0..8 {
            r = unsafe { r.unchecked_shl(1) };
            if ((v & 0x80) ^ (r & 0x80)) != 0 {
                r ^= 0x9;
            }
            v = unsafe { v.unchecked_shl(1) };
        }
    }
    unsafe { r.unchecked_shl(1) | 1 }
}
fn crc_v16(b: &[u8]) -> u16 {
    let mut r = 0u16;
    for i in b.iter() {
        unsafe {
            r = (r.unchecked_shr(8) & 0xFF) | r.unchecked_shl(8);
            r ^= *i as u16;
            r ^= (r & 0xFF).unchecked_shr(4);
            r ^= r.unchecked_shl(12);
            r ^= (r & 0xFF).unchecked_shl(5);
        }
    }
    r
}
