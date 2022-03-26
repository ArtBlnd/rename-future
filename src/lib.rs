extern crate proc_macro;
pub use proc_macro::rename_future;

pub use elain::Align;

pub struct PhantomUnsend(std::marker::PhantomData<*const std::ffi::c_void>);
unsafe impl Sync for PhantomUnsend {}
