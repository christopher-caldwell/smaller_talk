// Exercise the exact vendored arena source without GPUI's platform-wide test
// feature set. Its upstream tests run alongside the retention regressions.
#[allow(dead_code)]
#[path = "../../../vendor/gpui/src/arena.rs"]
mod arena;

use arena::Arena;
use std::{cell::Cell, rc::Rc};

#[test]
fn a_small_frame_releases_large_frame_capacity_after_dropping_values() {
    let mut arena = Arena::new(1024);
    let dropped = Rc::new(Cell::new(0));
    struct LargeElement([u8; 900], Rc<Cell<usize>>);
    impl Drop for LargeElement {
        fn drop(&mut self) {
            assert_eq!(self.0[0], 42);
            self.1.set(self.1.get() + 1);
        }
    }
    for _ in 0..32 {
        arena.alloc(|| LargeElement([42; 900], dropped.clone()));
    }
    let peak = arena.capacity();
    arena.clear();
    assert_eq!(dropped.get(), 32);
    assert_eq!(
        arena.capacity(),
        peak,
        "reuse capacity for equally large frames"
    );

    let stale = arena.alloc(|| 7u64);
    arena.clear();
    assert_eq!(arena.capacity(), 1024);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| *stale)).is_err());
    assert_eq!(*arena.alloc(|| 9u64), 9);
}

#[test]
fn equally_large_frames_keep_their_reusable_capacity() {
    let mut arena = Arena::new(1024);
    for _ in 0..3 {
        for i in 0..32u8 {
            assert_eq!(arena.alloc(|| [i; 900])[0], i);
        }
        arena.clear();
        assert_eq!(arena.capacity(), 32 * 1024);
    }
}

#[test]
fn nested_draw_defers_drops_and_capacity_release_until_the_outer_scope_finishes() {
    let mut arena = Arena::new(1024);
    for _ in 0..32 {
        arena.alloc(|| [0u8; 900]);
    }
    arena.clear();
    let peak = arena.capacity();
    arena.begin_scope();
    let live = arena.alloc(|| 42u64);
    arena.begin_scope();
    arena.clear();
    assert_eq!(*live, 42);
    assert_eq!(arena.capacity(), peak);
    arena.end_scope();
    arena.clear();
    assert_eq!(*live, 42);
    assert_eq!(arena.capacity(), peak);
    arena.end_scope();
    arena.clear();
    assert_eq!(arena.capacity(), 1024);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| *live)).is_err());
}
