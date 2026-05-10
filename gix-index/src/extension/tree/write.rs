use crate::extension::{Tree, tree};

impl Tree {
    /// Serialize this instance to `out`.
    pub fn write_to(&self, mut out: impl std::io::Write) -> Result<(), std::io::Error> {
        self.write_to_inner(None, &mut out)
    }

    pub(crate) fn write_to_with_hash(
        &self,
        object_hash: gix_hash::Kind,
        mut out: impl std::io::Write,
    ) -> Result<(), std::io::Error> {
        self.write_to_inner(Some(object_hash), &mut out)
    }

    fn write_to_inner(
        &self,
        object_hash: Option<gix_hash::Kind>,
        mut out: impl std::io::Write,
    ) -> Result<(), std::io::Error> {
        fn tree_entry(
            out: &mut impl std::io::Write,
            tree: &Tree,
            object_hash: Option<gix_hash::Kind>,
        ) -> Result<(), std::io::Error> {
            let mut buf = itoa::Buffer::new();
            let num_entries = match tree.num_entries {
                Some(num_entries) => buf.format(num_entries),
                None => buf.format(-1),
            };

            out.write_all(tree.name.as_slice())?;
            out.write_all(b"\0")?;
            out.write_all(num_entries.as_bytes())?;
            out.write_all(b" ")?;
            let num_children = buf.format(tree.children.len());
            out.write_all(num_children.as_bytes())?;
            out.write_all(b"\n")?;
            if tree.num_entries.is_some() {
                if object_hash.is_some_and(|object_hash| tree.id.kind() != object_hash) {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "tree-cache object id length does not match index object hash",
                    ));
                }
                out.write_all(tree.id.as_bytes())?;
            }

            for child in &tree.children {
                tree_entry(out, child, object_hash)?;
            }

            Ok(())
        }

        let signature = tree::SIGNATURE;

        let mut entries = Vec::<u8>::new();
        tree_entry(&mut entries, self, object_hash)?;

        out.write_all(&signature)?;
        out.write_all(
            &u32::try_from(entries.len())
                .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "tree extension exceeds 4GB"))?
                .to_be_bytes(),
        )?;
        out.write_all(&entries)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::extension::Tree;

    #[test]
    fn write_to_with_hash_rejects_object_hash_mismatch() {
        let tree = Tree {
            name: Default::default(),
            id: gix_hash::ObjectId::from_bytes_or_panic(&[0; 20]),
            num_entries: Some(1),
            children: Vec::new(),
        };
        let mut out = Vec::new();

        let err = tree.write_to_with_hash(gix_hash::Kind::Sha256, &mut out).unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }
}
