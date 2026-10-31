use uuid::Uuid;

/// Whom an upload is for (`?for=item:{id}` or `?for=assignment:{id}`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    /// An item whose body will name the file: needs `EditContent`.
    Item(Uuid),
    /// An assignment the caller will submit the file to: needs `SubmitWork`.
    Assignment(Uuid),
}

impl Owner {
    /// `item:<uuid>` or `assignment:<uuid>`; anything else is `None`.
    pub fn parse(text: &str) -> Option<Self> {
        let (kind, id) = text.split_once(':')?;
        let id = Uuid::parse_str(id).ok()?;
        match kind {
            "item" => Some(Self::Item(id)),
            "assignment" => Some(Self::Assignment(id)),
            _ => None,
        }
    }
}

/// The row that references files: an item (either of its copies) or a submission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageOwner {
    Item(Uuid),
    Submission(Uuid),
}

/// A row of `content.item_files`.
pub struct ItemReference {
    pub item_id: Uuid,
    pub published: bool,
}
