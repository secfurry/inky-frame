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
use core::convert::{Into, TryInto};
use core::option::Option::{self, None, Some};
use core::result::Result::{self, Ok};

use rpsp::Board;
use rpsp::clock::Timer;
use rpsp::pin::gpio::{Input, Output};
use rpsp::pin::{Pin, PinID};
use rpsp::spi::{Spi, SpiBus, SpiConfig, SpiError, SpiFormat, SpiIO, SpiPhase, SpiPolarity};

use crate::frame::ShiftRegister;

const BUSY: u8 = 7u8;
const BAUDRATE: u32 = 3_000_000u32;

pub enum BusySignal {
    Pin(Pin<Input>),
    SR(ShiftRegister),
}

pub struct Display<'a, const W: u16, const H: u16> {
    bs:    BusySignal,
    cs:    Pin<Output>,
    rst:   Pin<Output>,
    spi:   SpiBus<'a>,
    data:  Pin<Output>,
    timer: Timer,
}

impl BusySignal {
    #[inline]
    fn is_ready(&self) -> bool {
        match self {
            BusySignal::Pin(v) => v.is_high(),
            BusySignal::SR(v) => v.is_set(BUSY),
        }
    }
}
impl<'a, const W: u16, const H: u16> Display<'a, W, H> {
    #[inline]
    pub fn new(p: &'a Board, spi: SpiBus<'a>, cs: PinID, rst: PinID, data: PinID, bs: BusySignal) -> Display<'a, W, H> {
        Display {
            bs,
            spi,
            cs: p.pin(cs).output_high(),
            rst: p.pin(rst).output_high(),
            data: p.pin(data),
            timer: p.timer().clone(),
        }
    }
    pub fn create(p: &'a Board, tx: PinID, sck: PinID, cs: PinID, rst: PinID, data: PinID, bs: BusySignal) -> Result<Display<'a, W, H>, SpiError> {
        Ok(Display {
            bs,
            cs: p.pin(cs).output_high(),
            rst: p.pin(rst).output_high(),
            spi: Spi::new(
                p,
                BAUDRATE,
                SpiConfig::new()
                    .bits(8)
                    .format(SpiFormat::Motorola)
                    .phase(SpiPhase::First)
                    .polarity(SpiPolarity::Low)
                    .primary(true),
                (tx, sck).try_into()?,
            )?
            .into(),
            data: p.pin(data),
            timer: p.timer().clone(),
        })
    }

    #[inline]
    pub const fn width(&self) -> u16 {
        W
    }
    #[inline]
    pub const fn height(&self) -> u16 {
        H
    }
    #[inline]
    pub const fn shift_register(&self) -> Option<&ShiftRegister> {
        match &self.bs {
            BusySignal::Pin(_) => None,
            BusySignal::SR(v) => Some(v),
        }
    }

    #[inline]
    pub fn off(&mut self) {
        self.wait();
        self.cmd(0x2) // POF
    }
    #[inline]
    pub fn sleep(&mut self) {
        self.wait();
        self.cmd(0xA5) // ???
    }
    #[inline]
    pub fn refresh(&mut self) {
        self.setup();
        self.cmd(0x4);
        self.wait();
        self.cmd(0x12);
        self.wait();
    }
    #[inline]
    pub fn is_busy(&self) -> bool {
        !self.bs.is_ready()
    }
    #[inline]
    pub fn is_ready(&self) -> bool {
        self.bs.is_ready()
    }
    #[inline]
    pub fn update(&mut self, b: &[u8]) {
        self.setup();
        self.cmd_data(0x10, b); // DTM1
        self.wait();
        self.cmd(0x4); // PON
        self.wait();
        self.cmd(0x12); // DRF
        self.wait();
        self.cmd(0x2); // POF
    }
    #[inline]
    pub fn spi_bus(&mut self) -> &mut Spi {
        &mut self.spi
    }

    /// Returns immediately, the user must issue a POF command using the 'off'
    /// function once the display refresh is complete.
    pub unsafe fn update_async(&mut self, b: &[u8]) {
        self.setup();
        self.cmd_data(0x10, b); // DTM1
        self.wait();
        self.cmd(0x4); // PON
        self.wait();
        self.cmd(0x12); // DRF
    }

    #[inline]
    fn wait(&self) {
        while !self.bs.is_ready() {
            self.timer.sleep_ms(10);
        }
    }
    #[inline]
    fn reset(&self) {
        self.rst.low();
        self.timer.sleep_ms(10);
        self.rst.high();
        self.timer.sleep_ms(10);
        self.wait();
    }
    fn setup(&mut self) {
        self.reset();
        self.cmd_data(0x0, &[0xAF | if W == 600 { 0x40 } else { 0 }, 0x8]); // PSR
        self.cmd_data(0x1, &[0x37, 0, 0x23, 0x23]); // PWR
        self.cmd_data(0x3, &[0]); // PFS
        self.cmd_data(0x6, &[0xC7, 0xC7, 0x1D]); // BTST
        self.cmd_data(0x30, &[0x3C]); // PLL
        self.cmd_data(0x40, &[0]); // TSC
        self.cmd_data(0x50, &[0x37]); // CDI
        self.cmd_data(0x60, &[0x22]); // TCON
        unsafe {
            self.cmd_data(0x61, &[
                W.unchecked_shr(8) as u8,
                W as u8,
                H.unchecked_shr(8) as u8,
                H as u8,
            ]); // TRES
        }
        self.cmd_data(0xE3, &[0xAA]); // PWS
        self.timer.sleep_ms(100);
        self.cmd_data(0x50, &[0x37]) // CDI
    }
    #[inline]
    fn cmd(&mut self, v: u8) {
        self.cs.low();
        self.data.low();
        self.spi.write_single(v);
        self.cs.high();
    }
    #[inline]
    fn cmd_data(&mut self, v: u8, b: &[u8]) {
        self.cs.low();
        self.data.low();
        self.spi.write_single(v);
        self.data.high();
        self.spi.write(b);
        self.cs.high();
    }
}
