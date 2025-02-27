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

use core::convert::{From, Into};
use core::fmt::{self, Debug, Formatter};
use core::iter::IntoIterator;
use core::ops::FnOnce;
use core::option::Option::{self, None, Some};
use core::result::Result::{self, Err, Ok};

use rpsp::Pico;
use rpsp::pin::{Pin, PinID};
use rpsp::spi::{Spi, SpiBus, SpiError};

use self::tga::Pixel;

mod color;
mod display;
mod shift;
pub mod tga;

#[cfg_attr(rustfmt, rustfmt_skip)]
pub use self::color::*;
pub use self::display::*;
pub use self::shift::*;

const DEFAULT_CLEAR: u8 = 0x11u8;

pub enum InkyError {
    Spi(SpiError),
    InvalidBusyPins,
}
#[repr(u8)]
pub enum InkyRotation {
    Rotate0   = 0u8,
    Rotate90  = 1u8,
    Rotate180 = 2u8,
    Rotate270 = 3u8,
}

pub struct InkyPins {
    pub tx:       PinID,
    pub sck:      PinID,
    pub cs:       PinID,
    pub rst:      PinID,
    pub data:     PinID,
    pub busy_pin: Option<PinID>,
    pub sr_data:  Option<PinID>,
    pub sr_clock: Option<PinID>,
    pub sr_latch: Option<PinID>,
}
pub struct Inky<'a, const B: usize, const W: u16, const H: u16> {
    dis: Display<'a, W, H>,
    buf: [u8; B],
    rot: InkyRotation,
}

pub type Inky4<'a> = Inky<'a, 128_000, 640u16, 400u16>;
pub type Inky5<'a> = Inky<'a, 134_400, 600u16, 448u16>;
// NOTE(sf): ^This one might not not be correct, don't have the hardware to
//           test!

impl InkyPins {
    #[inline(always)]
    pub const fn inky_frame4() -> InkyPins {
        InkyPins {
            tx:       PinID::Pin19,
            sck:      PinID::Pin18,
            cs:       PinID::Pin17,
            rst:      PinID::Pin27,
            data:     PinID::Pin28,
            busy_pin: None,
            sr_data:  Some(PinID::Pin10),
            sr_clock: Some(PinID::Pin8),
            sr_latch: Some(PinID::Pin9),
        }
    }

    #[inline]
    pub const fn reset(mut self, p: PinID) -> InkyPins {
        self.rst = p;
        self
    }
    #[inline]
    pub const fn spi_tx(mut self, p: PinID) -> InkyPins {
        self.tx = p;
        self
    }
    #[inline]
    pub const fn spi_cs(mut self, p: PinID) -> InkyPins {
        self.cs = p;
        self
    }
    #[inline]
    pub const fn spi_sck(mut self, p: PinID) -> InkyPins {
        self.sck = p;
        self
    }
    #[inline]
    pub const fn busy_pin(mut self, p: PinID) -> InkyPins {
        self.busy_pin = Some(p);
        self
    }
    #[inline]
    pub const fn shift_register(mut self, clock: PinID, latch: PinID, data: PinID) -> InkyPins {
        self.sr_data = Some(data);
        self.sr_clock = Some(clock);
        self.sr_latch = Some(latch);
        self
    }

    #[inline]
    fn signal(self, p: &Pico) -> Result<BusySignal, InkyError> {
        if let Some(v) = self.busy_pin {
            return Ok(BusySignal::Pin(Pin::get(p, v).into_input()));
        }
        match (self.sr_clock, self.sr_latch, self.sr_data) {
            (Some(c), Some(l), Some(d)) => Ok(BusySignal::SR(ShiftRegister::new(p, c, l, d))),
            _ => Err(InkyError::InvalidBusyPins),
        }
    }
}
impl<const B: usize, const W: u16, const H: u16> Inky<'_, B, W, H> {
    #[inline]
    pub fn create(p: &Pico, cfg: InkyPins) -> Result<Inky<B, W, H>, InkyError> {
        Ok(Inky {
            dis: Display::create(
                p,
                cfg.tx,
                cfg.sck,
                cfg.cs,
                cfg.rst,
                cfg.data,
                cfg.signal(p)?,
            )
            .map_err(InkyError::Spi)?,
            buf: [DEFAULT_CLEAR; B],
            rot: InkyRotation::Rotate0,
        })
    }
    #[inline]
    pub fn new<'a>(p: &Pico, spi: impl Into<SpiBus<'a>>, cfg: InkyPins) -> Result<Inky<'a, B, W, H>, InkyError> {
        Ok(Inky {
            dis: Display::new(p, spi.into(), cfg.cs, cfg.rst, cfg.data, cfg.signal(p)?),
            buf: [DEFAULT_CLEAR; B],
            rot: InkyRotation::Rotate0,
        })
    }

    #[inline(always)]
    pub fn off(&mut self) {
        self.dis.off();
    }
    #[inline]
    pub fn clear(&mut self) {
        self.buf.fill(DEFAULT_CLEAR);
        self.dis.update(&self.buf);
    }
    #[inline(always)]
    pub fn update(&mut self) {
        self.dis.update(&self.buf)
    }
    #[inline(always)]
    pub fn width(&self) -> u16 {
        self.dis.width()
    }
    #[inline(always)]
    pub fn height(&self) -> u16 {
        self.dis.height()
    }
    #[inline(always)]
    pub fn is_busy(&self) -> bool {
        self.dis.is_busy()
    }
    #[inline(always)]
    pub fn is_ready(&self) -> bool {
        self.dis.is_ready()
    }
    #[inline(always)]
    pub fn set_fill(&mut self, c: Color) {
        self.buf.fill(c as u8);
    }
    #[inline(always)]
    pub fn spi_bus(&mut self) -> &mut Spi {
        self.dis.spi_bus()
    }
    #[inline(always)]
    pub fn set_rotation(&mut self, r: InkyRotation) {
        self.rot = r;
    }
    pub fn set_pixel(&mut self, x: u16, y: u16, c: Color) {
        if !self.in_bounds(x, y) {
            return;
        }
        let (i, v) = self.index(x, y);
        if let Some(p) = self.buf.get_mut(i) {
            *p = (*p & if v { 0xF } else { 0xF0 }) | if v { (c as u8) << 4 } else { c as u8 };
        }
    }
    #[inline(always)]
    pub fn shift_register(&self) -> Option<&ShiftRegister> {
        self.dis.shift_register()
    }
    pub fn set_pixel_raw(&mut self, x: u16, y: u16, c: u32) {
        if !self.in_bounds(x, y) {
            return;
        }
        let d = dither(x, y, c);
        let (i, v) = self.index(x, y);
        if let Some(p) = self.buf.get_mut(i) {
            *p = (*p & if v { 0xF } else { 0xF0 }) | if v { d << 4 } else { d };
        }
    }
    #[inline(always)]
    pub fn set_pixel_color(&mut self, x: u16, y: u16, c: RGB) {
        self.set_pixel_raw(x, y, c.uint());
    }
    #[inline(always)]
    pub fn set_with<E>(&mut self, func: impl FnOnce(&mut Inky<'_, B, W, H>) -> Result<(), E>) -> Result<(), E> {
        func(self)
    }
    pub fn set_image<E>(&mut self, x: i32, y: i32, image: impl IntoIterator<Item = Result<Pixel, E>>) -> Result<(), E> {
        // NOTE(sf): We don't bounds check here as we could offset images into
        //            non-visible space to only show a part of them. It won't get
        //            rendered anyway.
        for e in image {
            let r = e?;
            if r.is_transparent() {
                continue;
            }
            let (j, k) = (x + r.x, y + r.y);
            if (j < 0) || (k < 0) {
                continue;
            }
            let (f, g) = (j as u16, k as u16);
            // NOTE(sf): Bounds check is down here.
            if !self.in_bounds(f, g) {
                continue;
            }
            let d = dither(f, g, r.color);
            let (i, v) = self.index(f, g);
            if let Some(p) = self.buf.get_mut(i) {
                *p = (*p & if v { 0xF } else { 0xF0 }) | if v { d << 4 } else { d };
            }
        }
        Ok(())
    }

    /// Returns immediately, the user must issue a
    /// POF command using the 'off' function once
    /// the display refresh is complete.
    #[inline(always)]
    pub unsafe fn update_async(&mut self) {
        unsafe { self.dis.update_async(&self.buf) }
    }

    #[inline(always)]
    fn in_bounds(&self, x: u16, y: u16) -> bool {
        match self.rot {
            InkyRotation::Rotate0 | InkyRotation::Rotate180 if x >= W || y >= H => false,
            InkyRotation::Rotate90 | InkyRotation::Rotate270 if y >= W || x >= H => false,
            _ => true,
        }
    }
    #[inline]
    fn index(&self, x: u16, y: u16) -> (usize, bool) {
        let (q, w) = match self.rot {
            InkyRotation::Rotate0 => (x, y),
            InkyRotation::Rotate90 => (W - 1 - y, x),
            InkyRotation::Rotate180 => (W - 1 - x, H - 1 - y),
            InkyRotation::Rotate270 => (y, H - 1 - x),
        };
        (q as usize / 2 + (W as usize / 2) * w as usize, q & 0x1 != 0)
    }
}

impl From<u8> for InkyRotation {
    #[inline(always)]
    fn from(v: u8) -> InkyRotation {
        match v {
            1 => InkyRotation::Rotate180,
            2 => InkyRotation::Rotate180,
            3 => InkyRotation::Rotate180,
            _ => InkyRotation::Rotate0,
        }
    }
}

#[cfg(feature = "debug")]
impl Debug for InkyError {
    #[inline]
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            InkyError::Spi(v) => f.debug_tuple("Spi").field(v).finish(),
            InkyError::InvalidBusyPins => f.write_str("InvalidBusyPins"),
        }
    }
}
#[cfg(not(feature = "debug"))]
impl Debug for InkyError {
    #[inline(always)]
    fn fmt(&self, _f: &mut Formatter<'_>) -> fmt::Result {
        Ok(())
    }
}
