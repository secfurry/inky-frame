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

mod block;
mod device;
mod volume;

use core::cell::UnsafeCell;
use core::marker::Sync;
use core::mem::forget;
use core::ops::{Deref, DerefMut, Drop};
use core::ptr::NonNull;

use rpsp::locks::{Spinlock28, Spinlock29, Spinlock30};

pub use self::block::*;
pub use self::device::*;
pub use self::volume::*;

// Shared References are annoying, but this is the best way to handle this as
// the Pico does not operate properly when creating large objects.
//
// The Cache object allows for keeping a couple Blocks and a LFN that can be
// locked individually for usage.
//
// The 'lfn' is only used for Directory search functions, 'block_a' is used for
// small File read/writes. This may be kept locked when a File is converted to
// a Reader and will be released when the Reader is dropped. 'block_b' is only
// used for the Directory search cache.
static CACHE: Cache = Cache::new();

pub struct BlockPtr(NonNull<Block>);

struct CacheInner {
    lfn:     LongName,
    block_a: Block,
    block_b: Block,
}
struct Cache(UnsafeCell<CacheInner>);
struct LongNamePtr(NonNull<LongName>);

impl Cache {
    #[inline(always)]
    const fn new() -> Cache {
        Cache(UnsafeCell::new(CacheInner {
            lfn:     LongName::empty(),
            block_a: Block::new(),
            block_b: Block::new(),
        }))
    }

    #[inline(always)]
    fn lfn() -> LongNamePtr {
        let c = Spinlock30::claim();
        forget(c);
        LongNamePtr(unsafe { NonNull::new_unchecked(&mut (&mut *CACHE.0.get()).lfn) })
    }
    #[inline(always)]
    fn block_a() -> BlockPtr {
        let c = Spinlock29::claim();
        forget(c);
        BlockPtr(unsafe { NonNull::new_unchecked(&mut (&mut *CACHE.0.get()).block_a) })
    }
    #[inline(always)]
    fn block_b() -> BlockPtr {
        let c = Spinlock28::claim();
        forget(c);
        BlockPtr(unsafe { NonNull::new_unchecked(&mut (&mut *CACHE.0.get()).block_b) })
    }

    #[inline(always)]
    unsafe fn block_a_nolock() -> BlockPtr {
        BlockPtr(unsafe { NonNull::new_unchecked(&mut (&mut *CACHE.0.get()).block_a) })
    }
}

impl Drop for BlockPtr {
    #[inline(always)]
    fn drop(&mut self) {
        if unsafe { self.0.as_ref() }.as_ptr() == unsafe { (*CACHE.0.get()).block_a.as_ptr() } {
            unsafe { Spinlock29::free() }
        } else {
            unsafe { Spinlock28::free() }
        }
    }
}
impl Deref for BlockPtr {
    type Target = Block;

    #[inline(always)]
    fn deref(&self) -> &Block {
        unsafe { self.0.as_ref() }
    }
}
impl DerefMut for BlockPtr {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Block {
        unsafe { &mut *self.0.as_ptr() }
    }
}

impl Drop for LongNamePtr {
    #[inline(always)]
    fn drop(&mut self) {
        unsafe { Spinlock30::free() }
    }
}
impl Deref for LongNamePtr {
    type Target = LongName;

    #[inline(always)]
    fn deref(&self) -> &LongName {
        unsafe { self.0.as_ref() }
    }
}
impl DerefMut for LongNamePtr {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut LongName {
        unsafe { &mut *self.0.as_ptr() }
    }
}

unsafe impl Sync for Cache {}
