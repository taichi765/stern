use futures_util::{StreamExt as _, stream::FusedStream};
use std::{cell::RefCell, fmt::Debug, rc::Rc};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

/// A handle to slint's property.
///
/// `T` is a domain type (e.g. [`String`] or [`u32`]) and `U` is a slint type (e.g. [`slint::SharedString`] or [`i32`]).
///
/// Note that [`PropertyHandle`] holds the current value as a field, so if you modify the underlying value (e.g. `AppWindow::set_count()`),
/// [`PropertyHandle::get()`] and some other methods may return a value which is not the same as the actual value.
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
pub struct PropertyHandle<T, U> {
    value: Rc<RefCell<T>>,
    setter: Rc<dyn Fn(U)>,
    mapper: Rc<dyn Fn(T) -> U>,
}

impl<T> PropertyHandle<T, T> {
    /// Creates new [`PropertyHandle`] where domain type (`T`) and slint type (`U`) are same.
    pub fn new(setter: impl Fn(T) + 'static) -> Self
    where
        T: Default,
    {
        Self {
            value: Default::default(),
            setter: Rc::new(setter),
            mapper: Rc::new(|v| v),
        }
    }
}

impl<T, U> PropertyHandle<T, U> {
    /// Creates new [`PropertyHandle`] with a mapper.
    pub fn new_mapped(setter: impl Fn(U) + 'static, mapper: impl Fn(T) -> U + 'static) -> Self
    where
        T: Default,
    {
        Self {
            value: Default::default(),
            setter: Rc::new(setter),
            mapper: Rc::new(mapper),
        }
    }

    /// Gets the current value.
    pub fn get(&self) -> T
    where
        T: Copy,
    {
        *self.value.borrow()
    }

    /// Gets a reference to the value.
    pub fn get_ref(&self) -> std::cell::Ref<'_, T> {
        self.value.borrow()
    }
}

impl<T, U> PropertyHandle<T, U>
where
    T: Clone,
{
    /// Sets the value to underlying property.
    pub fn set(&self, val: T) {
        {
            let mut guard = self.value.borrow_mut();
            *guard = val.clone()
        }
        let mapped = (self.mapper)(val);
        (self.setter)(mapped);
    }

    /// Watches `input_stream` and sets latest value
    /// via [`setter`][Self::setter].
    ///
    /// Once property are bound to stream, [`PropertyHandle`] is consumed and cannot be changed manually.
    ///
    /// Note that if `setter` configured in [`PropertyHandle::new()`] calls any slint function, this function
    /// can't be called from the thread not running slint event loop.
    pub async fn watch<S>(self, mut input_stream: S) -> Result<(), StreamTerminated>
    where
        S: FusedStream<Item = T> + Unpin,
    {
        loop {
            if let Some(val) = input_stream.next().await {
                self.set(val);
            } else {
                return Err(StreamTerminated(()));
            }
        }
    }

    /// Run [`PropertyHandle::bind()`] in the slint event loop using [`slint::spawn_local()`].
    ///
    /// Returns handle to the spawned task.
    pub fn bind<S>(self, input_stream: S) -> CancellationToken
    where
        S: FusedStream<Item = T> + Unpin + 'static,
        T: 'static,
        U: 'static,
    {
        let tok = CancellationToken::new();
        let _join = slint::spawn_local({
            let tok = tok.clone();
            async move {
                tokio::select! {
                    _ = tok.cancelled() => (),
                    // An error means that input stream has ended before cancelled by caller.
                    // There's no problem so we ignore error here.
                    _ = self.watch(input_stream) => (),
                }
            }
        })
        .unwrap();
        tok
    }
}

/// Returned from [`PropertyHandle::bind()`]
#[derive(Debug, Error)]
#[error("stream has been terminated")]
pub struct StreamTerminated(());

impl<T, U> Debug for PropertyHandle<T, U>
where
    T: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PropertyHandle")
            .field("value", &self.value)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use tokio::sync::watch;
    use tokio_stream::wrappers::WatchStream;

    #[test]
    fn property_handle_bind_returns_err_when_stream_ends() {
        let val = Cell::new(0);
        let prop = PropertyHandle::new({
            let val = val.clone();
            move |v| {
                val.set(v);
            }
        });

        let (tx, rx) = watch::channel(0);
        slint::spawn_local(async move {
            prop.watch(WatchStream::new(rx).fuse())
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
