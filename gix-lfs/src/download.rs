use std::{
    collections::BTreeMap,
    io::{Read, Write},
};

use gix_error::{Result, ResultExt, bail, message};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::Pointer;

/// The media type of a Git LFS Batch API request or response.
pub const MEDIA_TYPE: &str = "application/vnd.git-lfs+json";

/// Encode a Batch API request for extension-free objects using the basic transfer.
pub fn request(objects: &[Pointer<'_>]) -> Result<Vec<u8>> {
    #[derive(Serialize)]
    struct Request<'a, 'oid> {
        operation: &'static str,
        transfers: [&'static str; 1],
        objects: &'a [Pointer<'oid>],
    }
    serde_json::to_vec(&Request {
        operation: "download",
        transfers: ["basic"],
        objects,
    })
    .or_raise(|| message("Could not encode Git LFS batch request"))
}

/// A download URL and the headers the server supplied for that URL.
#[derive(Deserialize)]
pub struct Action {
    /// The URL supplied by the server for the object contents.
    pub href: String,
    /// Headers for this action. Their values may contain credentials.
    #[serde(default)]
    pub header: BTreeMap<String, String>,
}

/// Decode a basic download response and match each returned object ID and size to the requested objects.
pub fn actions(requested: &[Pointer<'_>], response: &[u8]) -> Result<BTreeMap<String, Action>> {
    #[derive(Deserialize)]
    struct Response {
        transfer: Option<String>,
        objects: Vec<Object>,
    }
    #[derive(Deserialize)]
    struct Object {
        oid: String,
        size: u64,
        actions: Option<Actions>,
        error: Option<ObjectError>,
    }
    #[derive(Deserialize)]
    struct Actions {
        download: Action,
    }
    #[derive(Deserialize)]
    struct ObjectError {
        code: u16,
    }

    let response: Response = serde_json::from_slice(response).or_raise(|| message("Invalid Git LFS batch response"))?;
    if response.transfer.as_deref().is_some_and(|name| name != "basic") {
        bail!(gix_error::unsupported(
            "The Git LFS server selected an unsupported transfer"
        ));
    }
    let mut expected: BTreeMap<_, _> = requested.iter().map(|p| (p.oid, p.size)).collect();
    let mut actions = BTreeMap::new();
    for object in response.objects {
        if expected.remove(object.oid.as_str()) != Some(object.size) {
            bail!(gix_error::validation(
                "Git LFS response contains an unexpected object or size"
            ));
        }
        if let Some(error) = object.error {
            bail!(
                "The Git LFS server returned status {} for object {}",
                error.code,
                object.oid
            );
        }
        let Some(action) = object.actions else {
            bail!("The Git LFS server returned no download action for {}", object.oid);
        };
        actions.insert(object.oid, action.download);
    }
    if !expected.is_empty() {
        bail!(gix_error::validation("The Git LFS response omitted a requested object"));
    }
    Ok(actions)
}

/// Stream an object to `destination` and verify its length and SHA-256 digest.
/// The destination may contain partial or unverified data until this succeeds. Discard it on error.
pub fn copy_verified(pointer: Pointer<'_>, mut source: impl Read, mut destination: impl Write) -> Result<()> {
    let mut digest = Sha256::new();
    let mut remaining = pointer.size;
    let mut buf = [0; 16 * 1024];
    loop {
        let count = match source.read(&mut buf) {
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result.or_raise(|| message("Could not read Git LFS object"))?,
        };
        if count == 0 {
            break;
        }
        if count as u64 > remaining {
            bail!(
                "Git LFS object {} exceeds its declared size {}",
                pointer.oid,
                pointer.size
            );
        }
        remaining -= count as u64;
        digest.update(&buf[..count]);
        destination
            .write_all(&buf[..count])
            .or_raise(|| message("Could not write Git LFS object"))?;
    }
    if remaining != 0 {
        bail!(
            "Git LFS object {} is shorter than its declared size {}",
            pointer.oid,
            pointer.size
        );
    }
    if faster_hex::hex_string(&digest.finalize()) != pointer.oid {
        bail!("Git LFS object {} failed SHA-256 verification", pointer.oid);
    }
    Ok(())
}
