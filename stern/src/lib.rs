use futures_util::{StreamExt as _, stream::FusedStream};
use std::{fmt::Debug, rc::Rc};
use thiserror::Error;

pub mod nav;
pub use stern_macros::{adopter, route};
/// Re-exports external crates' types.
pub mod re_exports {
    pub use strum::{EnumDiscriminants, EnumIter, IntoEnumIterator};
}
mod worker;
pub use worker::WorkerThread;

/// Extension method on `XxxAdopter`.
pub trait GlobalExt<VM> {
    /// Registers viewmodel at `XxxAdopter` i.e. set `XxxViewModelTrait`'s method as the callback handler.
    fn register_viewmodel(&self, vm: std::rc::Rc<core::cell::RefCell<VM>>);
}

/// A handle to slint's property.
///
/// # Reference Cycle
/// [`PropertyHandle`] is usually owned by `XxxStates`, and `XxxStates` is owned by `XXxViewModelTrait`'s implementor,
/// and `XxxViewModelTrait`'s implementor is passed to `InnerXxxAdopter` via `XxxAdopter::on_click()`.
///
/// Finally `InnerXxxAdopter` has strong reference to [`PropertyHandle`], but [`PropertyHandle`] also references `InnerXxxAdopter` in order
/// to access internal `slint::private_unstable_api::Property`.
/// If [`PropertyHandle`] have strong reference to `InnerXxxAdopter`, it causes reference cycle.
///
/// Therefore, closures given to [`PropertyHandle`] must have weak reference to `InnerXxxAdopter` (or `XxxAdopter`) and must not have strong reference.
#[derive(Clone)]
pub struct PropertyHandle<T> {
    getter: Rc<dyn Fn() -> T>,
    setter: Rc<dyn Fn(T)>,
}

impl<T> PropertyHandle<T> {
    pub fn new(getter: impl Fn() -> T + 'static, setter: impl Fn(T) + 'static) -> Self {
        Self {
            getter: Rc::new(getter),
            setter: Rc::new(setter),
        }
    }

    pub fn get(&self) -> T {
        (self.getter)()
    }

    pub fn set(&self, val: T) {
        (self.setter)(val)
    }

    /// Binds `input_stream` to adopter i.e. watches `input_stream` and sets latest value
    /// via [`setter`][Self::setter].
    ///
    /// Once property are bound to stream, [`PropertyHandle`] is consumed and cannot be changed manually.
    pub async fn bind<S>(self, mut input_stream: S) -> Result<(), StreamTerminated>
    where
        S: FusedStream<Item = T> + Unpin,
    {
        loop {
            if let Some(val) = input_stream.next().await {
                (self.setter)(val);
            } else {
                return Err(StreamTerminated(()));
            }
        }
    }

    /// Run [`PropertyHandle::bind()`] in the slint event loop using [`slint::spawn_local()`].
    ///
    /// Returns handle to the spawned task.
    pub fn bind_detached<S>(self, input_stream: S) -> slint::JoinHandle<()>
    where
        S: FusedStream<Item = T> + Unpin + 'static,
        T: 'static,
    {
        slint::spawn_local(async move {
            self.bind(input_stream).await.expect("stream terminated");
        })
        .unwrap()
    }
}

/// Returned from [`PropertyHandle::bind()`]
#[derive(Debug, Error)]
#[error("stream has been terminated")]
pub struct StreamTerminated(());

impl<T> Debug for PropertyHandle<T>
where
    T: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PropertyHandle")
            .field("value", &(self.getter)())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use tokio::sync::watch;
    use tokio_stream::StreamExt;
    use tokio_stream::wrappers::WatchStream;

    #[test]
    fn property_handle_bind_returns_err_when_stream_ends() {
        let val = Cell::new(0);
        let prop = PropertyHandle {
            getter: Rc::new(|| val.get()),
            setter: Rc::new(|v| {
                val.set(v);
            }),
        };

        let (tx, rx) = watch::channel(0);
        slint::spawn_local(async move {
            prop.bind(WatchStream::new(rx).fuse())
                .await
                .expect_err("should return error");
        })
        .unwrap();
        tx.send(1).unwrap();
        assert_eq!(val.get(), 1);
        tx.send(2).unwrap();
        assert_eq!(val.get(), 2);
        drop(tx);
    }
}
