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

use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;

use rpsp::Pico;
use rpsp::pin::PinID;
use rpsp::pin::led::LedPwm;

pub struct Leds {
    pub a:        LedPwm,
    pub b:        LedPwm,
    pub c:        LedPwm,
    pub d:        LedPwm,
    pub e:        LedPwm,
    pub network:  LedPwm,
    pub activity: LedPwm,
}
pub struct LedsPtr(NonNull<Leds>);

impl Leds {
    #[inline(always)]
    pub fn all_on(&self) {
        self.set_all(true);
    }
    #[inline(always)]
    pub fn all_off(&self) {
        self.set_all(false);
    }
    #[inline]
    pub fn set_all(&self, en: bool) {
        self.a.set_on(en);
        self.b.set_on(en);
        self.c.set_on(en);
        self.d.set_on(en);
        self.e.set_on(en);
        self.network.set_on(en);
        self.activity.set_on(en);
    }
    #[inline]
    pub fn all_brightness(&self, v: u8) {
        self.a.brightness(v);
        self.b.brightness(v);
        self.c.brightness(v);
        self.d.brightness(v);
        self.e.brightness(v);
        self.network.brightness(v);
        self.activity.brightness(v);
    }

    #[inline]
    pub(crate) fn new(p: &Pico) -> Leds {
        Leds {
            a:        LedPwm::get(p, PinID::Pin11),
            b:        LedPwm::get(p, PinID::Pin12),
            c:        LedPwm::get(p, PinID::Pin13),
            d:        LedPwm::get(p, PinID::Pin14),
            e:        LedPwm::get(p, PinID::Pin15),
            network:  LedPwm::get(p, PinID::Pin7),
            activity: LedPwm::get(p, PinID::Pin6),
        }
    }
}
impl LedsPtr {
    #[inline(always)]
    pub(crate) fn new(i: &mut Leds) -> LedsPtr {
        LedsPtr(unsafe { NonNull::new_unchecked(i) })
    }
}

impl Deref for LedsPtr {
    type Target = Leds;

    #[inline(always)]
    fn deref(&self) -> &Leds {
        unsafe { self.0.as_ref() }
    }
}
impl DerefMut for LedsPtr {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Leds {
        unsafe { self.0.as_mut() }
    }
}
