pub mod nav;
pub use stern_macros::{adopter, route};
/// Re-exports external crates' types.
pub mod re_exports {
    pub use strum::{EnumDiscriminants, EnumIter, IntoEnumIterator};
}

mod worker;
pub use worker::WorkerThread;

mod property;
pub use property::PropertyHandle as MappedPropertyHandle;
/// Type alias for [`MappedPropertyHandle`].
pub type PropertyHandle<T> = property::PropertyHandle<T, T>;

/// Extension method on `XxxAdopter`.
pub trait GlobalExt<VM> {
    /// Registers viewmodel at `XxxAdopter` i.e. set `XxxViewModelTrait`'s method as the callback handler.
    fn register_viewmodel(&self, vm: std::rc::Rc<core::cell::RefCell<VM>>);
}
