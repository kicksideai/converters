// SPDX-License-Identifier: MIT
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static LIVE_BYTES: Cell<isize> = const { Cell::new(0) };
}

struct Allocator;

#[global_allocator]
static ALLOCATOR: Allocator = Allocator;

unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            let _ = LIVE_BYTES.try_with(|n| n.set(n.get() + layout.size() as isize));
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let _ = LIVE_BYTES.try_with(|n| n.set(n.get() - layout.size() as isize));
        System.dealloc(ptr, layout);
    }
}

pub fn live_bytes() -> isize {
    LIVE_BYTES.with(Cell::get)
}
