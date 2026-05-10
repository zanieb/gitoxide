use std::{borrow::Cow, collections::BTreeSet};

use crate::{
    bstr::{BStr, ByteSlice},
    config::tree::{Remote, Section},
    remote,
};

/// Query configuration related to remotes.
impl crate::Repository {
    /// Returns a sorted list unique of symbolic names of remotes that
    /// we deem [trustworthy][crate::open::Options::filter_config_section()].
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    /// # mod doctest { include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/doctest.rs")); }
    /// # let repo = doctest::open_repo(doctest::remote_repo_dir("clone")?)?;
    /// let remote_names: Vec<_> = repo.remote_names().into_iter().map(|name| name.to_string()).collect();
    ///
    /// assert_eq!(remote_names, vec!["myself".to_owned(), "origin".to_owned()]);
    /// # Ok(()) }
    /// ```
    pub fn remote_names(&self) -> remote::Names<'_> {
        self.config
            .resolved
            .sections_by_name(Remote.name())
            .map(|it| {
                let filter = self.filter_config_section();
                it.filter(move |s| filter(s.meta()))
                    .filter_map(|section| section.header().subsection_name().map(Cow::Borrowed))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Return a set of unique names of remote groups configured as `remotes.<group>`,
    /// if we deem their sections [trustworthy][crate::open::Options::filter_config_section()].
    ///
    /// The group names are configuration key names and thus always valid UTF-8.
    #[doc(alias = "remotes.<group>")]
    pub fn remote_group_names(&self) -> BTreeSet<&str> {
        self.config
            .resolved
            .sections_by_name("remotes")
            .map(|it| {
                let filter = self.filter_config_section();
                it.filter(move |s| filter(s.meta()))
                    .flat_map(|section| section.body().value_names().map(std::convert::AsRef::as_ref))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Return remote names configured for `remotes.<group>`, preserving configuration order.
    ///
    /// Each value is split like C Git does, using ASCII space, tab and newline as separators.
    /// Empty values yield an empty list.
    #[doc(alias = "remotes.<group>")]
    pub fn remote_names_by_group(&self, group: impl AsRef<str>) -> Option<Vec<Cow<'_, BStr>>> {
        self.config
            .resolved
            .strings_filter_by("remotes", None, group.as_ref(), &mut self.filter_config_section())
            .map(|values| {
                values
                    .into_iter()
                    .flat_map(|value| {
                        value
                            .as_ref()
                            .split(|b| matches!(*b, b' ' | b'\t' | b'\n'))
                            .filter(|name| !name.is_empty())
                            .map(|name| Cow::Owned(name.as_bstr().to_owned()))
                            .collect::<Vec<_>>()
                    })
                    .collect()
            })
    }

    /// Obtain the branch-independent name for a remote for use in the given `direction`, or `None` if it could not be determined.
    ///
    /// For _fetching_, use the only configured remote, or default to `origin` if it exists.
    /// For _pushing_, use the `remote.pushDefault` trusted configuration key, or fall back to the rules for _fetching_.
    ///
    /// # Notes
    ///
    /// It's up to the caller to determine what to do if the current `head` is unborn or detached.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    /// # mod doctest { include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/doctest.rs")); }
    /// # let repo = doctest::open_repo(doctest::remote_repo_dir("clone")?)?;
    /// assert_eq!(
    ///     repo.remote_default_name(gix::remote::Direction::Fetch)
    ///         .expect("configured")
    ///         .as_ref(),
    ///     "origin"
    /// );
    /// assert_eq!(
    ///     repo.remote_default_name(gix::remote::Direction::Push)
    ///         .expect("configured")
    ///         .as_ref(),
    ///     "origin"
    /// );
    /// # Ok(()) }
    /// ```
    pub fn remote_default_name(&self, direction: remote::Direction) -> Option<Cow<'_, BStr>> {
        let name = (direction == remote::Direction::Push)
            .then(|| {
                self.config
                    .resolved
                    .string_filter(Remote::PUSH_DEFAULT, &mut self.filter_config_section())
            })
            .flatten();
        name.or_else(|| {
            let names = self.remote_names();
            match names.len() {
                0 => None,
                1 => names.into_iter().next(),
                _more_than_one => {
                    let origin = Cow::Borrowed("origin".into());
                    names.contains(&origin).then_some(origin)
                }
            }
        })
    }
}
