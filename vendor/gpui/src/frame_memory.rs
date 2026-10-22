use std::cell::Cell;

thread_local! {
    static RECLAIMED: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn did_reclaim() {
    RECLAIMED.set(true);
}

pub(crate) fn release_reclaimed() {
    if !RECLAIMED.replace(false) {
        return;
    }

    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal: usize) -> usize;
        }

        // SAFETY: Apple documents NULL as all of this process's malloc zones
        // and zero as reclaiming as much unused storage as possible. This does
        // not invalidate live allocations. Invoke once at the frame boundary,
        // only after oversized containers actually released their storage.
        let started = std::time::Instant::now();
        let released = unsafe { malloc_zone_pressure_relief(std::ptr::null_mut(), 0) };
        log::debug!(
            "released {released} allocator bytes in {:?}",
            started.elapsed()
        );
    }
}
