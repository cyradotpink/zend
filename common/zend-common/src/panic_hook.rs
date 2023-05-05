#[macro_export]
macro_rules! set_panic_hook {
    () => {{
        thread_local!(static HOOK_SET: ::std::cell::Cell<bool> = ::std::cell::Cell::new(false));
        HOOK_SET.with(|is_set| {
            if !is_set.get() {
                ::std::panic::set_hook(::std::boxed::Box::new(|v: &::std::panic::PanicInfo| {
                    $crate::log!("Rust panicked qwq\n{}", v);
                }));
                is_set.set(true);
            }
        });
    }};
}
