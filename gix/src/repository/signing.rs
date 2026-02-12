use crate::bstr::BString;
use gix_hash::ObjectId;
use gix_object::WriteTo;

/// The format used for cryptographic signing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SigningFormat {
    /// OpenPGP (default), using `gpg`.
    #[default]
    Gpg,
    /// X.509, using `gpgsm`.
    X509,
    /// SSH, using `ssh-keygen`.
    Ssh,
}

/// Configuration for signing operations, as read from the git configuration.
#[derive(Debug, Clone, Default)]
pub struct SigningConfig {
    /// The signing key, from `user.signingKey`.
    pub key: Option<BString>,
    /// The signing format, from `gpg.format`.
    pub format: SigningFormat,
    /// The program to use for GPG signing, from `gpg.program` or `gpg.openpgp.program`.
    pub gpg_program: Option<BString>,
    /// The program to use for SSH signing, from `gpg.ssh.program`.
    pub ssh_program: Option<BString>,
    /// The program to use for X.509 signing, from `gpg.x509.program`.
    pub x509_program: Option<BString>,
}

/// A trait for signing data.
///
/// Implementations are expected to produce a signature for the given data.
/// The signature should be in the format appropriate for embedding in git objects
/// (e.g., ASCII-armored PGP or SSH signature).
///
/// # Example
///
/// Implementors can use external programs (gpg, ssh-keygen) or pure-Rust libraries
/// (sequoia-pgp) to produce signatures.
pub trait Sign {
    /// Sign the given `data` and return the signature as bytes.
    ///
    /// The returned signature will be embedded into the git object as-is.
    /// For commits, it becomes the `gpgsig` extra header.
    /// For tags, it becomes the trailing PGP signature.
    fn sign(&self, data: &[u8]) -> Result<BString, Box<dyn std::error::Error + Send + Sync>>;
}

/// A signing backend that shells out to `gpg` (or a configured alternative) to sign data.
#[cfg(feature = "command")]
pub struct GpgSign {
    /// The program to invoke (default: `gpg`).
    pub program: BString,
    /// The signing key identifier.
    pub signing_key: BString,
}

#[cfg(feature = "command")]
impl GpgSign {
    /// Create a new GPG signing backend.
    pub fn new(program: impl Into<BString>, signing_key: impl Into<BString>) -> Self {
        GpgSign {
            program: program.into(),
            signing_key: signing_key.into(),
        }
    }
}

#[cfg(feature = "command")]
impl Sign for GpgSign {
    fn sign(&self, data: &[u8]) -> Result<BString, Box<dyn std::error::Error + Send + Sync>> {
        use std::io::Write;
        use std::process::{Command, Stdio};

        let program = gix_path::from_bstring(self.program.clone());
        let signing_key = gix_path::from_bstring(self.signing_key.clone());

        let mut child = Command::new(program.as_os_str())
            .args(["--status-fd=2", "-bsau"])
            .arg(signing_key.as_os_str())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        // Write data and drop stdin so the child process sees EOF.
        {
            let mut stdin = child.stdin.take().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::Other, "failed to open stdin for gpg process")
            })?;
            stdin.write_all(data)?;
        }
        let output = child.wait_with_output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("gpg failed to sign the data: {stderr}").into());
        }

        // Verify we got a SIG_CREATED status
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.contains("[GNUPG:] SIG_CREATED ") {
            return Err(format!("gpg did not produce a signature: {stderr}").into());
        }

        Ok(BString::from(output.stdout))
    }
}

/// A signing backend that shells out to `ssh-keygen` to sign data using SSH keys.
#[cfg(feature = "command")]
pub struct SshSign {
    /// The program to invoke (default: `ssh-keygen`).
    pub program: BString,
    /// The signing key path or literal key prefixed with `key::`.
    pub signing_key: BString,
}

#[cfg(feature = "command")]
impl SshSign {
    /// Create a new SSH signing backend.
    pub fn new(program: impl Into<BString>, signing_key: impl Into<BString>) -> Self {
        SshSign {
            program: program.into(),
            signing_key: signing_key.into(),
        }
    }
}

#[cfg(feature = "command")]
impl Sign for SshSign {
    fn sign(&self, data: &[u8]) -> Result<BString, Box<dyn std::error::Error + Send + Sync>> {
        use std::io::Write;
        use std::process::{Command, Stdio};

        let is_literal = self.signing_key.starts_with(b"key::") || self.signing_key.starts_with(b"ssh-");

        // Write data to a temp file (ssh-keygen requires a file).
        // Use gix_tempfile for signal-safe cleanup and unique naming.
        let tmp_dir = std::env::temp_dir();
        let mut buffer_file = gix_tempfile::new(
            &tmp_dir,
            gix_tempfile::ContainingDirectory::Exists,
            gix_tempfile::AutoRemove::Tempfile,
        )?;
        buffer_file.write_all(data)?;
        let buffer_path = buffer_file.with_mut(|f| f.path().to_owned())?;

        let key_file_path;
        let _key_file_guard: Option<gix_tempfile::Handle<gix_tempfile::handle::Writable>>;

        if is_literal {
            let literal_key = if let Some(stripped) = self.signing_key.strip_prefix(b"key::") {
                stripped
            } else {
                &self.signing_key
            };
            let mut key_file = gix_tempfile::new(
                &tmp_dir,
                gix_tempfile::ContainingDirectory::Exists,
                gix_tempfile::AutoRemove::Tempfile,
            )?;
            key_file.write_all(literal_key)?;
            key_file_path = key_file.with_mut(|f| f.path().to_owned())?;
            _key_file_guard = Some(key_file);
        } else {
            key_file_path = gix_path::from_bstring(self.signing_key.clone());
            _key_file_guard = None;
        };

        let program = gix_path::from_bstring(self.program.clone());
        let mut cmd = Command::new(program.as_os_str());
        cmd.args(["-Y", "sign", "-n", "git", "-f"]);
        cmd.arg(&key_file_path);
        if is_literal {
            cmd.arg("-U");
        }
        cmd.arg(&buffer_path);
        cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());

        let output = cmd.output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("ssh-keygen failed to sign: {stderr}").into());
        }

        // Read the signature file (ssh-keygen writes <file>.sig)
        let sig_path = buffer_path.with_extension("sig");
        let signature = std::fs::read(&sig_path)?;
        let _ = std::fs::remove_file(&sig_path);

        // Temp files are cleaned up when guards are dropped
        Ok(BString::from(signature))
    }
}

/// The error returned by signing operations.
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum Error {
    #[error("no signing key configured (set user.signingKey in git config)")]
    NoSigningKey,
    #[error("unknown signing format in gpg.format: {value:?}")]
    UnknownSigningFormat { value: BString },
    #[error("signing failed: {0}")]
    Sign(Box<dyn std::error::Error + Send + Sync>),
    #[error(transparent)]
    WriteObject(#[from] crate::object::write::Error),
    #[error(transparent)]
    ReferenceEdit(#[from] crate::reference::edit::Error),
    #[error(transparent)]
    ReferenceNameValidation(#[from] gix_ref::name::Error),
    #[error(transparent)]
    ParseTime(#[from] crate::config::time::Error),
    #[error("committer identity is not configured")]
    CommitterMissing,
    #[error("author identity is not configured")]
    AuthorMissing,
}

impl crate::Repository {
    /// Read signing configuration from the repository's git config.
    ///
    /// This reads `user.signingKey`, `gpg.format`, `gpg.program`,
    /// `gpg.ssh.program`, and `gpg.x509.program`.
    ///
    /// Returns an error if `gpg.format` is set to an unrecognized value.
    pub fn signing_config(&self) -> Result<SigningConfig, Error> {
        let config = &self.config.resolved;

        let key = config.string("user.signingKey").map(|v| v.into_owned());

        let format = match config.string("gpg.format") {
            Some(v) => match &**v {
                b"openpgp" => SigningFormat::Gpg,
                b"x509" => SigningFormat::X509,
                b"ssh" => SigningFormat::Ssh,
                _ => return Err(Error::UnknownSigningFormat { value: v.into_owned() }),
            },
            None => SigningFormat::default(),
        };

        let gpg_program = config
            .string("gpg.program")
            .or_else(|| config.string("gpg.openpgp.program"))
            .map(|v| v.into_owned());

        let ssh_program = config.string("gpg.ssh.program").map(|v| v.into_owned());

        let x509_program = config.string("gpg.x509.program").map(|v| v.into_owned());

        Ok(SigningConfig {
            key,
            format,
            gpg_program,
            ssh_program,
            x509_program,
        })
    }

    /// Create a signed commit with the given `signer`.
    ///
    /// This is similar to [`commit()`](Self::commit()), but the commit object is first
    /// serialized, then signed by the `signer`, and finally stored with the signature
    /// embedded as a `gpgsig` extra header.
    ///
    /// The commit is written to the object database and `reference` is updated to point to it.
    pub fn commit_signed<Name, E>(
        &self,
        reference: Name,
        message: impl AsRef<str>,
        tree: impl Into<ObjectId>,
        parents: impl IntoIterator<Item = impl Into<ObjectId>>,
        signer: &dyn Sign,
    ) -> Result<crate::Id<'_>, Error>
    where
        Name: TryInto<gix_ref::FullName, Error = E>,
        Error: From<E>,
    {
        let author = self.author().ok_or(Error::AuthorMissing)??;
        let committer = self.committer().ok_or(Error::CommitterMissing)??;
        self.commit_signed_as(committer, author, reference, message, tree, parents, signer)
    }

    /// Create a signed commit with explicitly specified `committer` and `author`.
    ///
    /// See [`commit_signed()`](Self::commit_signed()) for details.
    pub fn commit_signed_as<'a, 'c, Name, E>(
        &self,
        committer: impl Into<gix_actor::SignatureRef<'c>>,
        author: impl Into<gix_actor::SignatureRef<'a>>,
        reference: Name,
        message: impl AsRef<str>,
        tree: impl Into<ObjectId>,
        parents: impl IntoIterator<Item = impl Into<ObjectId>>,
        signer: &dyn Sign,
    ) -> Result<crate::Id<'_>, Error>
    where
        Name: TryInto<gix_ref::FullName, Error = E>,
        Error: From<E>,
    {
        use gix_ref::transaction::{Change, LogChange, PreviousValue, RefEdit, RefLog};
        use gix_ref::Target;
        use smallvec::SmallVec;

        let committer_ref = committer.into();
        let author_ref = author.into();
        let reference = reference.try_into()?;
        let tree = tree.into();
        let parents: SmallVec<[ObjectId; 1]> = parents.into_iter().map(Into::into).collect();

        // Build the unsigned commit object
        let unsigned_commit = gix_object::Commit {
            message: message.as_ref().into(),
            tree,
            author: author_ref.into(),
            committer: committer_ref.into(),
            encoding: None,
            parents: parents.clone(),
            extra_headers: Default::default(),
        };

        // Serialize the unsigned commit to bytes
        let mut commit_data = Vec::new();
        unsigned_commit
            .write_to(&mut commit_data)
            .map_err(|err| Error::Sign(Box::new(err)))?;

        // Sign the serialized commit data
        let signature = signer.sign(&commit_data).map_err(Error::Sign)?;

        // Now create the commit with the gpgsig extra header
        let signed_commit = gix_object::Commit {
            extra_headers: vec![("gpgsig".into(), signature)],
            ..unsigned_commit
        };

        let commit_id = self.write_object(&signed_commit)?;

        // Update the reference
        self.edit_references_as(
            Some(RefEdit {
                change: Change::Update {
                    log: LogChange {
                        mode: RefLog::AndReference,
                        force_create_reflog: false,
                        message: crate::reference::log::message(
                            "commit",
                            signed_commit.message.as_ref(),
                            signed_commit.parents.len(),
                        ),
                    },
                    expected: match signed_commit.parents.first().map(|p| Target::Object(*p)) {
                        Some(previous) => {
                            if reference.as_bstr() == "HEAD" {
                                PreviousValue::MustExistAndMatch(previous)
                            } else {
                                PreviousValue::ExistingMustMatch(previous)
                            }
                        }
                        None => PreviousValue::MustNotExist,
                    },
                    new: Target::Object(commit_id.inner),
                },
                name: reference,
                deref: true,
            }),
            Some(committer_ref),
        )?;

        Ok(commit_id)
    }

    /// Create a signed tag reference.
    ///
    /// This is similar to [`tag()`](Self::tag()), but the tag object is first serialized,
    /// then signed by the `signer`, and stored with the signature appended.
    pub fn tag_signed(
        &self,
        name: impl AsRef<str>,
        target: impl AsRef<gix_hash::oid>,
        target_kind: gix_object::Kind,
        tagger: Option<gix_actor::SignatureRef<'_>>,
        message: impl AsRef<str>,
        constraint: gix_ref::transaction::PreviousValue,
        signer: &dyn Sign,
    ) -> Result<crate::Reference<'_>, Error> {
        // Build the unsigned tag
        let unsigned_tag = gix_object::Tag {
            target: target.as_ref().into(),
            target_kind,
            name: name.as_ref().into(),
            tagger: tagger
                .map(|t| t.to_owned())
                .transpose()
                .map_err(|err| Error::Sign(Box::new(err)))?,
            message: message.as_ref().into(),
            pgp_signature: None,
        };

        // Serialize the unsigned tag
        let mut tag_data = Vec::new();
        unsigned_tag
            .write_to(&mut tag_data)
            .map_err(|err| Error::Sign(Box::new(err)))?;

        // Sign the serialized tag data
        let signature = signer.sign(&tag_data).map_err(Error::Sign)?;

        // Create the tag with the signature
        let signed_tag = gix_object::Tag {
            pgp_signature: Some(signature),
            ..unsigned_tag
        };

        let tag_id = self.write_object(&signed_tag)?;
        Ok(self
            .tag_reference(name, tag_id, constraint)
            .map_err(Error::ReferenceEdit)?)
    }
}
