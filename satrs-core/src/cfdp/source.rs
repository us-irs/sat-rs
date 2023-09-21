use spacepackets::util::UnsignedByteField;

pub struct SourceHandler {
    id: UnsignedByteField,
}

impl SourceHandler {
    pub fn new(id: impl Into<UnsignedByteField>) -> Self {
        Self { id: id.into() }
    }
}

#[cfg(test)]
mod tests {}
