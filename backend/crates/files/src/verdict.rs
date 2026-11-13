use std::collections::BTreeMap;

use uuid::Uuid;

/// The answer of the check "may this owner name these files", as plain data. The check is policy
/// that reads the owners' references, so it lives in `learning`; the owners (`content`,
/// `assessment`) only need its answer, and this crate sits below all of them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileVerdict {
    /// The first named file the owner may not use (held by another owner, or unknown).
    pub unusable: Option<Uuid>,
    /// The first file named as an image whose stored type is not an image.
    pub not_image: Option<Uuid>,
    /// The stored name of every named file that exists.
    pub names: BTreeMap<Uuid, String>,
}
