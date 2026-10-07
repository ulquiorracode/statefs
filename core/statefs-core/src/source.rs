//! SPI state source contract for pluggable configuration and state ingestion.

use crate::error::StoreError;
use crate::store::Store;

/// Trait representing an external source capable of populating a [`Store`].
pub trait StateSource {
    /// Applies this source's state entries into the provided store.
    fn apply_to_store<S: Store>(&self, store: &mut S) -> Result<(), StoreError>;
}

impl<F> StateSource for F
where
    F: Fn(
        &mut dyn FnMut(&crate::Path, crate::Value) -> Result<Option<crate::Node>, StoreError>,
    ) -> Result<(), StoreError>,
{
    fn apply_to_store<S: Store>(&self, _store: &mut S) -> Result<(), StoreError> {
        // Fallback for simple closures if needed
        Ok(())
    }
}
