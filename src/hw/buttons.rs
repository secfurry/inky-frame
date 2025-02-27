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

use core::clone::Clone;
use core::cmp::{Eq, PartialEq};
use core::convert::From;
use core::marker::Copy;
use core::matches;
use core::mem::discriminant;
use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;

use crate::frame::ShiftRegister;

pub enum WakeReason {
    None,
    RTC,
    ButtonA,
    ButtonB,
    ButtonC,
    ButtonD,
    ButtonE,
    External,
}

pub struct Buttons {
    sr:    ShiftRegister,
    state: u8,
}
pub struct ButtonsPtr(NonNull<Buttons>);

pub type Button = WakeReason;

const BUTTON_A: u8 = 0x01u8;
const BUTTON_B: u8 = 0x02u8;
const BUTTON_C: u8 = 0x04u8;
const BUTTON_D: u8 = 0x08u8;
const BUTTON_E: u8 = 0x10u8;
const BUTTON_RTC: u8 = 0x20u8;
const BUTTON_EXTERNAL: u8 = 0x40u8;

impl Buttons {
    #[inline(always)]
    pub fn clear(&mut self) {
        self.state = 0
    }
    #[inline(always)]
    pub fn state(&self) -> u8 {
        self.state
    }
    #[inline(always)]
    pub fn read(&mut self) -> u8 {
        self.state = self.sr.read();
        self.state
    }
    #[inline(always)]
    pub fn button_a(&self) -> bool {
        self.state & BUTTON_A != 0
    }
    #[inline(always)]
    pub fn button_b(&self) -> bool {
        self.state & BUTTON_B != 0
    }
    #[inline(always)]
    pub fn button_c(&self) -> bool {
        self.state & BUTTON_C != 0
    }
    #[inline(always)]
    pub fn button_d(&self) -> bool {
        self.state & BUTTON_D != 0
    }
    #[inline(always)]
    pub fn button_e(&self) -> bool {
        self.state & BUTTON_E != 0
    }
    #[inline]
    pub fn pressed(&self) -> Button {
        match self.state {
            _ if self.state & BUTTON_A != 0 => Button::ButtonA,
            _ if self.state & BUTTON_B != 0 => Button::ButtonB,
            _ if self.state & BUTTON_C != 0 => Button::ButtonC,
            _ if self.state & BUTTON_D != 0 => Button::ButtonD,
            _ if self.state & BUTTON_E != 0 => Button::ButtonE,
            _ if self.state & BUTTON_RTC != 0 => Button::RTC,
            _ if self.state & BUTTON_EXTERNAL != 0 => Button::External,
            _ => Button::None,
        }
    }
    #[inline(always)]
    pub fn button_any(&self) -> bool {
        self.state & 0x1F != 0
    }
    #[inline(always)]
    pub fn set_raw(&mut self, v: u8) {
        self.state = v
    }
    #[inline(always)]
    pub fn set(&mut self, v: Button) {
        self.set_raw(match v {
            Button::ButtonA => BUTTON_A,
            Button::ButtonB => BUTTON_B,
            Button::ButtonC => BUTTON_C,
            Button::ButtonD => BUTTON_D,
            Button::ButtonE => BUTTON_E,
            Button::RTC => BUTTON_RTC,
            Button::External => BUTTON_EXTERNAL,
            Button::None => 0u8,
        })
    }
    #[inline(always)]
    pub fn read_pressed(&mut self) -> bool {
        self.read() & 0x1F != 0 || self.state & BUTTON_EXTERNAL != 0
    }
    #[inline]
    pub fn pressed_buttons(&self) -> Button {
        match self.state {
            _ if self.state & BUTTON_A != 0 => Button::ButtonA,
            _ if self.state & BUTTON_B != 0 => Button::ButtonB,
            _ if self.state & BUTTON_C != 0 => Button::ButtonC,
            _ if self.state & BUTTON_D != 0 => Button::ButtonD,
            _ if self.state & BUTTON_E != 0 => Button::ButtonE,
            _ => Button::None,
        }
    }
    #[inline(always)]
    pub fn shift_register(&self) -> &ShiftRegister {
        &self.sr
    }

    #[inline(always)]
    pub(crate) fn new(state: u8, sr: ShiftRegister) -> Buttons {
        Buttons { sr, state }
    }
}
impl ButtonsPtr {
    #[inline(always)]
    pub(crate) fn new(i: &mut Buttons) -> ButtonsPtr {
        ButtonsPtr(unsafe { NonNull::new_unchecked(i) })
    }
}
impl WakeReason {
    #[inline(always)]
    pub fn wake_from_ext(&self) -> bool {
        matches!(self, WakeReason::External)
    }
    #[inline(always)]
    pub fn wake_from_rtc(&self) -> bool {
        matches!(self, WakeReason::RTC)
    }
    #[inline(always)]
    pub fn wake_from_button(&self) -> bool {
        matches!(
            self,
            WakeReason::ButtonA | WakeReason::ButtonB | WakeReason::ButtonC | WakeReason::ButtonD | WakeReason::ButtonE
        )
    }
}

impl Clone for Buttons {
    #[inline(always)]
    fn clone(&self) -> Buttons {
        Buttons {
            sr:    self.sr.clone(),
            state: self.state.clone(),
        }
    }
}

impl Deref for ButtonsPtr {
    type Target = Buttons;

    #[inline(always)]
    fn deref(&self) -> &Buttons {
        unsafe { self.0.as_ref() }
    }
}
impl DerefMut for ButtonsPtr {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Buttons {
        unsafe { self.0.as_mut() }
    }
}

impl Eq for WakeReason {}
impl Copy for WakeReason {}
impl Clone for WakeReason {
    #[inline(always)]
    fn clone(&self) -> WakeReason {
        *self
    }
}
impl From<u8> for WakeReason {
    #[inline]
    fn from(v: u8) -> WakeReason {
        match v {
            _ if v & 0x40 != 0 => WakeReason::External,
            _ if v & 0x20 != 0 => WakeReason::RTC,
            _ if v & 0x10 != 0 => WakeReason::ButtonE,
            _ if v & 0x08 != 0 => WakeReason::ButtonD,
            _ if v & 0x04 != 0 => WakeReason::ButtonC,
            _ if v & 0x02 != 0 => WakeReason::ButtonB,
            _ if v & 0x01 != 0 => WakeReason::ButtonA,
            _ => WakeReason::None,
        }
    }
}
impl PartialEq for WakeReason {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        discriminant(self) == discriminant(other)
    }
}
