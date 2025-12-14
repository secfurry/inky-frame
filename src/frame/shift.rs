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
use core::cmp::Ord;
use core::convert::From;

use rpsp::Board;
use rpsp::clock::Timer;
use rpsp::pin::gpio::{Input, Output};
use rpsp::pin::{Pin, PinID};

use crate::hw::WakeReason;

pub struct ShiftRegister {
    lat:   Pin<Output>,
    data:  Pin<Input>,
    clock: Pin<Output>,
    timer: Timer,
}

impl ShiftRegister {
    #[inline]
    pub fn new(p: &Board, clock: PinID, lat: PinID, data: PinID) -> ShiftRegister {
        ShiftRegister {
            lat:   p.pin(lat).output_high(),
            clock: p.pin(clock).output_high(),
            data:  p.pin(data).into_input(),
            timer: p.timer().clone(),
        }
    }

    pub fn read(&self) -> u8 {
        self.lat.low();
        self.timer.sleep_us(2);
        self.lat.high();
        self.timer.sleep_us(2);
        let (mut r, mut b) = (0u8, 8u8);
        while b > 0 {
            b -= 1;
            r = unsafe { r.unchecked_shl(1) };
            if self.data.is_high() {
                r |= 1
            } else {
                r |= 0
            }
            self.clock.low();
            self.timer.sleep_us(2);
            self.clock.high();
            self.timer.sleep_us(2);
        }
        r
    }
    #[inline]
    pub fn is_set(&self, v: u8) -> bool {
        unsafe { self.read() & 1u8.unchecked_shl((v as u32).min(7)) != 0 }
    }
    #[inline]
    pub fn read_wake(&self) -> WakeReason {
        WakeReason::from(self.read())
    }
}

impl Clone for ShiftRegister {
    #[inline]
    fn clone(&self) -> ShiftRegister {
        ShiftRegister {
            lat:   self.lat.clone(),
            data:  self.data.clone(),
            clock: self.clock.clone(),
            timer: self.timer.clone(),
        }
    }
}
