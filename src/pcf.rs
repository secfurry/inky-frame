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

use core::cmp::Ord;
use core::convert::{From, Into};
use core::marker::Send;
use core::option::Option::{self, None, Some};
use core::result::Result::{self, Err, Ok};

use rpsp::clock::{AlarmConfig, RtcError, TimeSource};
use rpsp::i2c::mode::Controller;
use rpsp::i2c::{I2c, I2cAddress, I2cBus};
use rpsp::int::Acknowledge;
use rpsp::time::{Month, Time, Weekday};

use crate::Slice;

const CMD_SAVED: u8 = 0x03u8;
const CMD_CTRL_1: u8 = 0x00u8;
const CMD_CTRL_2: u8 = 0x01u8;
const CMD_ALARM2: u8 = 0x0Bu8;
const CMD_STATUS: u8 = 0x04u8;
const CMD_TIMER_MODE: u8 = 0x11u8;
const CMD_TIMER_VALUE: u8 = 0x10u8;

const ADDR: I2cAddress = I2cAddress::new_7bit(0x51u8);

#[repr(u8)]
pub enum PcfTick {
    Speed4kHz = 0u8,
    Speed64Hz = 1u8,
    Speed1Hz  = 2u8,
    Slow      = 3u8,
}
#[repr(u8)]
pub enum PcfOutput {
    Rate32kHz = 0u8,
    Rate16kHz = 1u8,
    Rate8kHz  = 2u8,
    Rate4kHz  = 3u8,
    Rate2kHz  = 4u8,
    Rate1kHz  = 5u8,
    Rate1Hz   = 6u8,
    Off       = 7u8,
}

pub struct PcfRtc<'a> {
    i2c: I2cBus<'a, Controller>,
}

impl<'a> PcfRtc<'a> {
    #[inline]
    pub fn new(i2c: impl Into<I2cBus<'a, Controller>>) -> PcfRtc<'a> {
        PcfRtc { i2c: i2c.into() }
    }

    #[inline]
    pub fn i2c_bus(&self) -> &I2c<Controller> {
        &self.i2c
    }
    #[inline]
    pub fn reset(&mut self) -> Result<(), RtcError> {
        self.i2c.write(ADDR, &[CMD_CTRL_1, 0x58])?;
        loop {
            self.i2c.write(ADDR, &[CMD_STATUS, 0])?;
            if self.is_stable()? {
                break;
            }
        }
        Ok(())
    }
    #[inline]
    pub fn now(&mut self) -> Result<Time, RtcError> {
        self.now_inner()
    }
    #[inline]
    pub fn get_byte(&mut self) -> Result<u8, RtcError> {
        self.read_reg(CMD_SAVED)
    }
    #[inline]
    pub fn is_24hr(&mut self) -> Result<bool, RtcError> {
        Ok(self.read_reg(CMD_CTRL_1)? & 0x2 == 0)
    }
    #[inline]
    pub fn is_stable(&mut self) -> Result<bool, RtcError> {
        Ok(self.read_reg(CMD_STATUS)? & 0x80 == 0)
    }
    #[inline]
    pub fn alarm_disable(&mut self) -> Result<(), RtcError> {
        // Set the register values to disable instead of just clearing them, as bit 7
        // (0x80) as zero means it's enabled.
        self.i2c.write(ADDR, &[CMD_ALARM2, 0x80, 0x80, 0x80, 0x80, 0x80])?;
        Ok(())
    }
    #[inline]
    pub fn timer_disable(&mut self) -> Result<(), RtcError> {
        let v = self.read_reg(CMD_TIMER_MODE)?;
        // 0x6 - (0b110)
        // - Clear TE:    Timer Enable
        // - Clear TIE:   Timer Interrupt Enable
        // - Clear TI_TP: Timer Interrupt Mode
        //
        // Clear the bottom 3 bits of the timer flags so the timer won't do
        // anything weird.
        self.i2c.write(ADDR, &[CMD_TIMER_MODE, v & 0xF8])?;
        Ok(())
    }
    #[inline]
    pub fn alarm_state(&mut self) -> Result<bool, RtcError> {
        Ok(self.read_reg(CMD_CTRL_2)? & 0x40 != 0)
    }
    #[inline]
    pub fn timer_state(&mut self) -> Result<bool, RtcError> {
        Ok(self.read_reg(CMD_CTRL_2)? & 0x8 != 0)
    }
    #[inline]
    pub fn set_byte(&mut self, v: u8) -> Result<(), RtcError> {
        self.i2c.write(ADDR, &[CMD_SAVED, v])?;
        Ok(())
    }
    pub fn set_time(&mut self, v: Time) -> Result<(), RtcError> {
        if !v.is_valid() {
            return Err(RtcError::InvalidTime);
        }
        let r = self.read_reg(CMD_CTRL_1)?;
        // Reset the 12_24 register to 0, to avoid any 12/24 hours confusion.
        self.i2c.write(ADDR, &[CMD_CTRL_1, r & 0xFD])?;
        self.i2c.write(ADDR, &[
            CMD_STATUS,
            encode(v.secs),
            encode(v.mins),
            encode(v.hours),
            encode(v.day),
            encode(v.weekday as u8),
            encode(v.month as u8),
            encode(v.year.saturating_sub(0x7D0) as u8),
        ])?;
        Ok(())
    }
    #[inline]
    pub fn set_24hr(&mut self, en: bool) -> Result<(), RtcError> {
        let v = self.read_reg(CMD_CTRL_1)?;
        self.i2c.write(ADDR, &[CMD_CTRL_1, if en { v & 0xFD } else { v | 0x2 }])?;
        Ok(())
    }
    #[inline]
    pub fn alarm_clear_state(&mut self) -> Result<bool, RtcError> {
        let v = self.read_reg(CMD_CTRL_2)?;
        self.i2c.write(ADDR, &[CMD_CTRL_2, v & 0xBF])?;
        Ok(v & 0x40 != 0)
    }
    #[inline]
    pub fn timer_clear_state(&mut self) -> Result<bool, RtcError> {
        let v = self.read_reg(CMD_CTRL_2)?;
        self.i2c.write(ADDR, &[CMD_CTRL_2, v & 0xF7])?;
        Ok(v & 0x8 != 0)
    }
    #[inline]
    pub fn set_timer_ms(&mut self, ms: u32) -> Result<(), RtcError> {
        let (t, s) = ms_to_ticks(ms).ok_or(RtcError::ValueTooLarge)?;
        self.set_timer(t, s)
    }
    pub fn set_alarm(&mut self, v: AlarmConfig) -> Result<(), RtcError> {
        if v.is_empty() {
            return self.alarm_disable();
        }
        if !v.is_valid() {
            return Err(RtcError::InvalidTime);
        }
        let r = self.read_reg(CMD_CTRL_1)?;
        // Reset the 12_24 register to 0, to avoid any 12/24 hours confusion.
        self.i2c.write(ADDR, &[CMD_CTRL_1, r & 0xFD])?;
        self.i2c.write(ADDR, &[
            CMD_ALARM2,
            v.secs.map_or(0x80, encode),
            v.mins.map_or(0x80, encode),
            v.hours.map_or(0x80, encode),
            v.day.map_or(0x80, |i| encode(i.get())),
            v.weekday.map_or(0x80, |i| encode(i as u8)),
        ])?;
        Ok(())
    }
    #[inline]
    pub fn set_alarm_interrupt(&mut self, en: bool) -> Result<(), RtcError> {
        let v = self.read_reg(CMD_CTRL_2)?;
        self.i2c.write(ADDR, &[
            CMD_CTRL_2,
            (if en { v | 0x80 } else { v & 0x7F }) & 0xBF,
            // Clear the Alarm interrupt register
        ])?;
        Ok(())
    }
    /// See the bottom of page 26 in the documentation for the PCF85063A IC to
    /// determine what ticks and speed relate to.
    ///
    /// Documentation at: <https://www.nxp.com/docs/en/data-sheet/PCF85063A.pdf>
    #[inline]
    pub fn set_timer(&mut self, ticks: u8, speed: PcfTick) -> Result<(), RtcError> {
        let v = self.read_reg(CMD_TIMER_MODE)?;
        self.i2c.write(ADDR, &[CMD_TIMER_VALUE, ticks, unsafe {
            (v & 0xE7) | ((speed as u8) & 0x3).unchecked_shl(3) | 0x4
        }])?;
        Ok(())
    }
    #[inline]
    pub fn set_time_from(&mut self, mut v: impl TimeSource) -> Result<(), RtcError> {
        self.set_time(v.now().map_err(|e| e.into())?)
    }
    #[inline]
    pub fn set_timer_interrupt(&mut self, en: bool, pulse: bool) -> Result<(), RtcError> {
        self.timer_clear_state()?; // Clear the Timer Interrupt flag.
        let v = self.read_reg(CMD_TIMER_MODE)?;
        self.i2c.write(ADDR, &[
            CMD_TIMER_MODE,
            (v & 0xFC) | if en { 0x2 } else { 0 } | if pulse { 0x1 } else { 0 },
        ])?;
        Ok(())
    }

    #[inline]
    fn now_inner(&mut self) -> Result<Time, RtcError> {
        let mut b: [u8; 7] = [0u8; 7];
        self.i2c.write_single_then_read(ADDR, CMD_STATUS, &mut b)?;
        let mut d = Time::new(
            decode(b.read_u8(6)) as u16 + 0x7D0,
            Month::from(decode(b.read_u8(5))),
            decode(b.read_u8(3)),
            decode(b.read_u8(2)),
            decode(b.read_u8(1)),
            decode(b.read_u8(0) & 0x7F),
            Weekday::from(decode(b.read_u8(4))),
        );
        if d.hours >= 24 {
            // Correct 12hr to 24hr skew.
            // Hours in 12hr
            // - AM/PM: Bit 5.
            // - Hours (10's place [1 or 0]): 4
            // - Hours (1's place [1 - 9]): 3-0 (in BCD)
            //
            // To 24hr:
            // 1. Decode min unit.
            // 2. If Bit 4 is 1, add 10.
            // 3. If Bit 5 is 1, add 12 (it's PM).
            let h = b.read_u8(2);
            d.hours = decode(h & 0x7).max(9) + if h & 0x10 != 0 { 10 } else { 0 } + if h & 0x20 != 0 { 12 } else { 0 };
        }
        if !d.is_valid() { Err(RtcError::InvalidTime) } else { Ok(d) }
    }
    #[inline]
    fn read_reg(&mut self, r: u8) -> Result<u8, RtcError> {
        self.i2c.write_single(ADDR, r)?;
        Ok(self.i2c.read_single(ADDR)?)
    }
}

impl TimeSource for PcfRtc<'_> {
    type Error = RtcError;

    #[inline]
    fn now(&mut self) -> Result<Time, RtcError> {
        self.now_inner()
    }
}
impl Acknowledge for PcfRtc<'_> {
    #[inline]
    fn ack_interrupt(&mut self) -> bool {
        self.alarm_clear_state()
            .or_else(|_| self.timer_clear_state())
            .unwrap_or(false)
    }
}

unsafe impl Send for PcfRtc<'_> {}

#[inline]
fn encode(v: u8) -> u8 {
    let i = v / 10;
    unsafe { (v - (i * 10)) | i.unchecked_shl(4) }
}
#[inline]
fn decode(v: u8) -> u8 {
    unsafe { (v & 0xF) + (v.unchecked_shr(4) & 0xF).wrapping_mul(10) }
}
#[inline]
fn ms_to_ticks(v: u32) -> Option<(u8, PcfTick)> {
    match v {
        0..=62 => Some(((v * 1_000 / 244) as u8, PcfTick::Speed4kHz)),
        0..=3_800 => Some(((v / 15) as u8, PcfTick::Speed64Hz)),
        0..=255_000 => Some(((v / 1_000) as u8, PcfTick::Speed1Hz)),
        0..=15_300_000 => Some((((v / 1_000) / 60) as u8, PcfTick::Slow)),
        _ => None,
    }
}
