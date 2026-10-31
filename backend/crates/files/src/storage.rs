use std::path::Path;
use std::sync::Arc;

use object_store::ObjectStore;
use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjectPath;
use uuid::Uuid;

use crate::FilesError;

/// Where the bytes live: a directory, or any `object_store` backend. Cheap to clone.
#[derive(Clone)]
pub struct Storage {
    store: Arc<dyn ObjectStore>,
}

impl Storage {
    /// A directory on local disk; it is created if it does not exist.
    pub fn local(dir: &Path) -> Result<Self, FilesError> {
        std::fs::create_dir_all(dir).map_err(|error| object_store::Error::Generic {
            store: "LocalFileSystem",
            source: Box::new(error),
        })?;
        let store = LocalFileSystem::new_with_prefix(dir)?;
        Ok(Self::from_store(Arc::new(store)))
    }

    pub fn from_store(store: Arc<dyn ObjectStore>) -> Self {
        Self { store }
    }

    pub(crate) fn store(&self) -> &dyn ObjectStore {
        self.store.as_ref()
    }
}

/// `ab/cd/<id>`: derived from the id only, never from the name of the file.
pub(crate) fn key(id: Uuid) -> ObjectPath {
    let id = id.simple().to_string();
    ObjectPath::from(format!("{}/{}/{id}", &id[0..2], &id[2..4]))
}
