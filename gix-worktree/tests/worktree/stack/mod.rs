use crate::Result;
use gix_glob::pattern::Case;

mod create_directory;

mod attributes;
mod ignore;

fn probe_case() -> Result<Case> {
    let dir = gix_testtools::tempfile::tempdir_in(std::env::current_dir()?)?;
    Ok(if gix_fs::Capabilities::probe_dir(dir.path()).ignore_case {
        Case::Fold
    } else {
        Case::Sensitive
    })
}
