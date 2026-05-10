use std::borrow::Cow;

use gix_ref::{Category, FullNameRef};

use crate::{
    Reference,
    bstr::ByteSlice,
    remote,
    repository::{branch_remote_ref_name, branch_remote_tracking_ref_name},
};

/// Remotes
impl<'repo> Reference<'repo> {
    /// Find the name of our remote for `direction` as configured in `branch.<name>.remote|pushRemote` respectively.
    /// Return `None` if no remote is configured.
    ///
    /// See also [`Repository::branch_remote_name()`](crate::Repository::branch_remote_name()) for more details.
    pub fn remote_name(&self, direction: remote::Direction) -> Option<remote::Name<'_>> {
        let (category, shortname) = self.name().category_and_short_name()?;
        match category {
            Category::RemoteBranch => {
                if shortname.find_iter("/").take(2).count() == 1 {
                    let slash_pos = shortname.find_byte(b'/').expect("it was just found");
                    shortname[..slash_pos]
                        .as_bstr()
                        .to_str()
                        .ok()
                        .map(|n| remote::Name::Symbol(n.into()))
                } else {
                    let remotes = self.repo.remote_names();
                    for slash_pos in shortname.rfind_iter("/") {
                        let candidate = shortname[..slash_pos].as_bstr();
                        if remotes.contains(candidate) {
                            return candidate.to_str().ok().map(|n| remote::Name::Symbol(n.into()));
                        }
                    }
                    None
                }
            }
            Category::LocalBranch => self.repo.branch_remote_name(shortname, direction),
            _ => None,
        }
    }

    /// Find the remote along with all configuration associated with it suitable for handling this reference.
    ///
    /// See also [`Repository::branch_remote()`](crate::Repository::branch_remote()) for more details.
    pub fn remote(
        &self,
        direction: remote::Direction,
    ) -> Option<Result<crate::Remote<'repo>, remote::find::existing::Error>> {
        let name = self.remote_name(direction)?;
        let found = self
            .repo
            .try_find_remote(name.as_bstr())
            .map(|res| res.map_err(Into::into))
            .or_else(|| match name {
                remote::Name::Url(url) => gix_url::parse(url.as_ref())
                    .map_err(Into::into)
                    .and_then(|url| {
                        self.repo
                            .remote_at(url)
                            .map_err(|err| remote::find::existing::Error::Find(remote::find::Error::Init(err)))
                    })
                    .into(),
                remote::Name::Symbol(_) => None,
            });
        Some(found?.map(|mut remote| {
            if direction == remote::Direction::Fetch && remote.refspecs(direction).is_empty() {
                if let Some(Ok(remote_ref)) = self.remote_ref_name(direction) {
                    remote
                        .replace_refspecs([remote_ref.as_bstr()], direction)
                        .expect("full ref names are valid fetch refspec sources");
                }
            }
            remote
        }))
    }

    /// Return the name of this reference on the remote side.
    ///
    /// See [`Repository::branch_remote_ref_name()`](crate::Repository::branch_remote_ref_name()) for details.
    #[doc(alias = "upstream", alias = "git2")]
    pub fn remote_ref_name(
        &self,
        direction: remote::Direction,
    ) -> Option<Result<Cow<'_, FullNameRef>, branch_remote_ref_name::Error>> {
        self.repo.branch_remote_ref_name(self.name(), direction)
    }

    /// Return the name of the reference that tracks this reference on the remote side.
    ///
    /// See [`Repository::branch_remote_tracking_ref_name()`](crate::Repository::branch_remote_tracking_ref_name()) for details.
    #[doc(alias = "upstream", alias = "git2")]
    pub fn remote_tracking_ref_name(
        &self,
        direction: remote::Direction,
    ) -> Option<Result<Cow<'_, FullNameRef>, branch_remote_tracking_ref_name::Error>> {
        self.repo.branch_remote_tracking_ref_name(self.name(), direction)
    }
}
