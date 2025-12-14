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

use core::cmp::{Ord, PartialEq};
use core::convert::From;
use core::marker::Send;
use core::mem::MaybeUninit;
use core::ops::Deref;
use core::option::Option::{self};
use core::ptr::NonNull;
use core::result::Result::{self, Ok};

use rpsp::atomic::{Mutex, with};
use rpsp::clock::{AlarmConfig, RtcError};
use rpsp::i2c::I2cController;
use rpsp::pin::gpio::Output;
use rpsp::pin::{Pin, PinID};
use rpsp::spi::{Spi, SpiConfig, SpiDev, SpiFormat, SpiPhase, SpiPolarity};
use rpsp::time::Time;
use rpsp::{Board, static_instance};

use crate::frame::ShiftRegister;
use crate::fs::Storage;
use crate::hw::{Buttons, ButtonsPtr, Leds, LedsPtr, WakeReason};
use crate::pcf::PcfRtc;
use crate::sd::Card;

#[cfg_attr(rustfmt, rustfmt_skip)]
pub use rpsp::{pin, pwm, sleep, sleep_us, ticks, ticks_ms};

const PFC_RTC_HZ: u32 = 400_000u32;

static_instance!(INSTANCE, MaybeUninit<Inner>, Inner::new());

pub struct InkyBoard<'a> {
    i: NonNull<Inner<'a>>,
    p: Board,
}

struct Inner<'a> {
    pwr:     Pin<Output>,
    rtc:     PcfRtc<'a>,
    spi:     Option<Spi>,
    leds:    Leds,
    wake:    WakeReason,
    buttons: Buttons,
}

impl<'a> Inner<'a> {
    #[inline]
    const fn new() -> MaybeUninit<Inner<'a>> {
        MaybeUninit::zeroed()
    }

    #[inline]
    fn is_ready(&self) -> bool {
        PinID::Pin0.ne(self.pwr.id())
    }
    #[inline]
    fn setup(&mut self, p: &Board) {
        // NOTE(sf): Ensure that VSYS_HOLD is enabled so we stay on during boot.
        self.pwr = Pin::get(&p, PinID::Pin2).output_high();
        let s = ShiftRegister::new(&p, PinID::Pin8, PinID::Pin9, PinID::Pin10);
        let w = s.read();
        self.wake = WakeReason::from(w);
        self.buttons = Buttons::new(w, s);
        // NOTE(sf): 'unwrap_unchecked' is used as this call can only return
        //           'InvalidFrequency' or 'InvalidPins', which are both impossible
        //           due to the const configuration.
        self.rtc = PcfRtc::new(unsafe { I2cController::new(&p, PinID::Pin4, PinID::Pin5, PFC_RTC_HZ).unwrap_unchecked() });
        // Don't care if we can't clear the PFC state, we're already online.
        let _ = self.rtc.alarm_clear_state();
        let _ = self.rtc.alarm_disable();
        let _ = self.rtc.set_timer_interrupt(false, false);
        self.leds = Leds::new(p);
    }
}
impl<'a> InkyBoard<'a> {
    #[inline]
    pub fn get() -> InkyBoard<'a> {
        let p = Board::get();
        InkyBoard {
            i: with(|x| {
                // Workaround for the compiler identifying that the I2C Rtc cannot be
                // zero. So we do this, it's zeroed out so it's valid memory.
                let f = unsafe { &mut *INSTANCE.borrow_mut(x).assume_init_mut() };
                if !f.is_ready() {
                    f.setup(&p);
                }
                unsafe { NonNull::new_unchecked(f) }
            }),
            p,
        }
    }

    #[inline]
    pub fn leds(&self) -> &Leds {
        &self.ptr().leds
    }
    #[inline]
    pub fn spi_bus(&self) -> &Spi {
        // Lazy make the SPI bus.
        self.ptr().spi.get_or_insert_with(|| unsafe {
            // NOTE(sf): 'unwrap_unchecked' is used as the 'SPI::new' call can
            //           only return 'InvalidFrequency' or 'InvalidPins', which
            //           are both impossible due to the const configuration.
            // NOTE(sf): 'unwrap_unchecked is used as the 'SpiDev::new_rx' call
            //           can only return 'InvalidPins', which is not possible
            //           with the const pin configuration.
            Spi::new(
                &self.p,
                SpiConfig::DEFAULT_BAUD_RATE,
                SpiConfig::new()
                    .bits(8)
                    .format(SpiFormat::Motorola)
                    .phase(SpiPhase::First)
                    .polarity(SpiPolarity::Low)
                    .primary(true),
                SpiDev::new_rx(PinID::Pin19, PinID::Pin18, PinID::Pin16).unwrap_unchecked(),
            )
            .unwrap_unchecked()
        })
    }
    #[inline]
    pub fn sync_pcf_to_rtc(&self) {
        let _ = self.p.rtc().set_time_from(self.pcf());
    }
    #[inline]
    pub fn sync_rtc_to_pcf(&self) {
        let _ = self.pcf().set_time_from(self.p.rtc());
    }
    #[inline]
    pub fn pcf(&self) -> &mut PcfRtc<'a> {
        &mut self.ptr().rtc
    }
    #[inline]
    pub fn buttons(&self) -> &mut Buttons {
        &mut self.ptr().buttons
    }
    #[inline]
    pub fn set_rtc_and_pcf(&self, v: Time) {
        let _ = self.p.rtc().set_time(v);
        let _ = self.pcf().set_time(v);
    }
    #[inline]
    pub fn wake_reason(&self) -> WakeReason {
        self.ptr().wake
    }
    #[inline]
    pub fn i2c_bus(&self) -> &I2cController {
        self.ptr().rtc.i2c_bus()
    }
    #[inline]
    pub fn sd_card(&self) -> Storage<Card<'_>> {
        Storage::new(Card::new(&self.p, PinID::Pin22, self.spi_bus()))
    }
    #[inline]
    pub fn shift_register(&self) -> &ShiftRegister {
        self.ptr().buttons.shift_register()
    }
    /// Returns wait period in milliseconds.
    pub fn set_rtc_wake(&self, secs: u32) -> Result<u32, RtcError> {
        let d = self.pcf();
        let mut v = d.now()?.add_seconds((secs as i64).min(0x24EA00));
        if v.secs >= 55 && v.mins <= 58 {
            // Account for a bug in the RTC, from MicroPython.
            (v.secs, v.mins) = (5, v.mins + 1);
        }
        let _ = d.alarm_clear_state()?;
        let _ = d.set_alarm(
            AlarmConfig::new()
                .month(v.month)
                .day(v.day)
                .hours(v.hours)
                .mins(v.mins)
                .secs(v.secs),
        )?;
        let _ = d.set_alarm_interrupt(true)?;
        Ok(secs.saturating_mul(1_000))
    }

    /// SAFETY: This is unsafe as this will immediately power off the
    ///         device if it's on LIPO/Battery power. Make sure to sync
    ///         and finish all work beforehand.
    ///
    /// This has no affect if powered by USB or External (non-Battery) and
    /// cannot be '!'.
    #[inline]
    pub unsafe fn power_off(&self) {
        self.ptr().pwr.low();
        // wait for a couple secs for power off
        self.sleep(1_500);
    }
    /// SAFETY: This is unsafe as this will immediately power off the
    ///         device if it's on LIPO/Battery power. Make sure to sync
    ///         and finish all work beforehand.
    ///
    /// The device will wake up after the specified number of seconds. If
    /// powered externally by USB or External (non-Battery), the device will
    /// sleep for the period of time instead.
    #[inline]
    pub unsafe fn deep_sleep(&self, secs: u32) -> Result<(), RtcError> {
        let v = self.set_rtc_wake(secs)?;
        unsafe { self.power_off() };
        // On battery power, the next lines will NOT run.
        self.p.sleep(v);
        let d: &mut PcfRtc<'_> = self.pcf();
        // Ignore reset errors, as these only affect calls that don't
        // need the PFC to wake them up.
        let _ = d.alarm_clear_state();
        let _ = d.alarm_disable();
        let _ = d.set_timer_interrupt(false, false);
        Ok(())
    }

    #[inline]
    fn ptr(&self) -> &mut Inner<'a> {
        unsafe { &mut *self.i.as_ptr() }
    }
}

impl Deref for InkyBoard<'_> {
    type Target = Board;

    #[inline]
    fn deref(&self) -> &Board {
        &self.p
    }
}

unsafe impl Send for Inner<'_> {}

#[inline]
pub fn leds() -> LedsPtr {
    LedsPtr::new(&mut InkyBoard::get().ptr().leds)
}
#[inline]
pub fn buttons() -> ButtonsPtr {
    ButtonsPtr::new(InkyBoard::get().buttons())
}
