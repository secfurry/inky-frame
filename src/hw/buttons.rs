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
use core::fmt::{Debug, Display, Formatter, Result};
use core::marker::Copy;
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
    /// Returns the cached state as an [`u8`].
    #[inline]
    pub const fn raw(&self) -> u8 {
        self.state
    }

    #[inline]
    pub fn clear(&mut self) {
        self.state = 0
    }
    /// Check cached state and returns a [`Button`] for any button was pressed
    /// or a trigger occurred.
    ///
    /// If nothing was pressed or triggered, this function returns
    /// [`Button`]::None.
    ///
    /// This returns physical button values first before checking triggers.
    /// Use [`Buttons::is_pressed`] if you need to read a specifc trigger.
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
    /// Check cached state and returns a [`Button`] for any button was pressed
    /// or a trigger occurred. This will update the cached state to remove the
    /// returned Button. This can be used to poll all current [`Button`]s
    /// pressed.
    ///
    /// If nothing was pressed or triggered, this function returns
    /// [`Button::None`].
    ///
    /// This returns physical button values first before checking triggers.
    /// Use [`Buttons::is_pressed`] if you need to read a specifc trigger.
    #[inline]
    pub fn take(&mut self) -> Button {
        let (m, v) = match self.state {
            _ if self.state & BUTTON_A != 0 => (BUTTON_A, Button::ButtonA),
            _ if self.state & BUTTON_B != 0 => (BUTTON_B, Button::ButtonB),
            _ if self.state & BUTTON_C != 0 => (BUTTON_C, Button::ButtonC),
            _ if self.state & BUTTON_D != 0 => (BUTTON_D, Button::ButtonD),
            _ if self.state & BUTTON_E != 0 => (BUTTON_E, Button::ButtonE),
            _ if self.state & BUTTON_RTC != 0 => (BUTTON_RTC, Button::RTC),
            _ if self.state & BUTTON_EXTERNAL != 0 => (BUTTON_EXTERNAL, Button::External),
            _ => return Button::None,
        };
        self.state &= !m; // Remove button.
        v
    }
    /// Check cached state and returns `true` if any button was pressed or
    /// a trigger occurred.
    #[inline]
    pub fn any_pressed(&self) -> bool {
        self.state != 0
    }
    /// Update the [`Buttons`] state.
    ///
    /// Returns itself for convince.
    pub fn read(&mut self) -> &mut Buttons {
        self.state = self.sr.read() & 0x7F; // Cut out the "DISPLAY_BUSY" bit.
        self
    }
    /// Set the cached state to the specific [`Button`] value. This will replace
    /// the current state.
    ///
    /// To add a button state, use the [`AddAssign`] operation (`+=`) to with a
    /// [`Button`] on the [`Buttons`] struct.
    #[inline]
    pub fn set_pressed(&mut self, v: Button) {
        match v {
            Button::None => self.state = 0u8,
            Button::RTC => self.state = BUTTON_RTC,
            Button::ButtonA => self.state = BUTTON_A,
            Button::ButtonB => self.state = BUTTON_B,
            Button::ButtonC => self.state = BUTTON_C,
            Button::ButtonD => self.state = BUTTON_D,
            Button::ButtonE => self.state = BUTTON_E,
            Button::External => self.state = BUTTON_EXTERNAL,
        }
    }
    /// Check cached state and returns `true` the supplied button/trigger was
    /// pressed or occurred.
    #[inline]
    pub fn is_pressed(&self, v: Button) -> bool {
        match v {
            Button::RTC if self.state & BUTTON_RTC != 0 => true,
            Button::ButtonA if self.state & BUTTON_A != 0 => true,
            Button::ButtonB if self.state & BUTTON_B != 0 => true,
            Button::ButtonC if self.state & BUTTON_C != 0 => true,
            Button::ButtonD if self.state & BUTTON_D != 0 => true,
            Button::ButtonE if self.state & BUTTON_E != 0 => true,
            Button::External if self.state & BUTTON_EXTERNAL != 0 => true,
            _ => false,
        }
    }
    #[inline]
    pub fn shift_register(&self) -> &ShiftRegister {
        &self.sr
    }

    #[inline]
    pub(crate) fn new(state: u8, sr: ShiftRegister) -> Buttons {
        Buttons { sr, state }
    }
}
impl ButtonsPtr {
    #[inline]
    pub(crate) fn new(i: &mut Buttons) -> ButtonsPtr {
        ButtonsPtr(unsafe { NonNull::new_unchecked(i) })
    }
}
impl WakeReason {
    #[inline]
    pub fn is_none(&self) -> bool {
        match self {
            WakeReason::None => true,
            _ => false,
        }
    }
    #[inline]
    pub fn is_some(&self) -> bool {
        match self {
            WakeReason::None => false,
            _ => true,
        }
    }
    #[inline]
    pub fn is_button(&self) -> bool {
        match self {
            WakeReason::ButtonA | WakeReason::ButtonB | WakeReason::ButtonC | WakeReason::ButtonD | WakeReason::ButtonE => true,
            _ => false,
        }
    }
    #[inline]
    pub fn is_trigger(&self) -> bool {
        match self {
            WakeReason::RTC | WakeReason::External => true,
            _ => false,
        }
    }
    #[inline]
    pub fn wake_from_ext(&self) -> bool {
        match self {
            WakeReason::External => true,
            _ => false,
        }
    }
    #[inline]
    pub fn wake_from_rtc(&self) -> bool {
        match self {
            WakeReason::RTC => true,
            _ => false,
        }
    }
    #[inline]
    pub fn wake_from_button(&self) -> bool {
        self.is_button()
    }
}

impl Clone for Buttons {
    #[inline]
    fn clone(&self) -> Buttons {
        Buttons {
            sr:    self.sr.clone(),
            state: self.state.clone(),
        }
    }
}

impl Deref for ButtonsPtr {
    type Target = Buttons;

    #[inline]
    fn deref(&self) -> &Buttons {
        unsafe { self.0.as_ref() }
    }
}
impl DerefMut for ButtonsPtr {
    #[inline]
    fn deref_mut(&mut self) -> &mut Buttons {
        unsafe { self.0.as_mut() }
    }
}

impl Eq for WakeReason {}
impl Copy for WakeReason {}
impl Clone for WakeReason {
    #[inline]
    fn clone(&self) -> WakeReason {
        *self
    }
}
impl From<u8> for WakeReason {
    #[inline]
    fn from(v: u8) -> WakeReason {
        // Returns the first state if sees, in-case multiple exist.
        match v {
            _ if v & 0x01 != 0 => WakeReason::ButtonA,
            _ if v & 0x02 != 0 => WakeReason::ButtonB,
            _ if v & 0x04 != 0 => WakeReason::ButtonC,
            _ if v & 0x08 != 0 => WakeReason::ButtonD,
            _ if v & 0x10 != 0 => WakeReason::ButtonE,
            _ if v & 0x20 != 0 => WakeReason::RTC,
            _ if v & 0x40 != 0 => WakeReason::External,
            _ => WakeReason::None,
        }
    }
}
impl PartialEq for WakeReason {
    #[inline]
    fn eq(&self, other: &WakeReason) -> bool {
        discriminant(self) == discriminant(other)
    }
}

impl Debug for WakeReason {
    #[cfg(feature = "debug")]
    #[inline]
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            WakeReason::RTC => f.write_str("RTC"),
            WakeReason::None => f.write_str("None"),
            WakeReason::ButtonA => f.write_str("ButtonA"),
            WakeReason::ButtonB => f.write_str("ButtonB"),
            WakeReason::ButtonC => f.write_str("ButtonC"),
            WakeReason::ButtonD => f.write_str("ButtonD"),
            WakeReason::ButtonE => f.write_str("ButtonE"),
            WakeReason::External => f.write_str("External"),
        }
    }
    #[cfg(not(feature = "debug"))]
    #[inline]
    fn fmt(&self, _f: &mut Formatter<'_>) -> Result {
        core::result::Result::Ok(())
    }
}
impl Display for WakeReason {
    #[inline]
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        Debug::fmt(self, f)
    }
}
