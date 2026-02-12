mod signing {
    use gix::repository::signing::Sign;

    /// A mock signer that produces a deterministic fake signature for testing.
    struct MockSigner;

    impl Sign for MockSigner {
        fn sign(&self, _data: &[u8]) -> Result<gix::bstr::BString, Box<dyn std::error::Error + Send + Sync>> {
            Ok(gix::bstr::BString::from(
                "-----BEGIN PGP SIGNATURE-----\n\
                 mock-signature-for-testing\n\
                 -----END PGP SIGNATURE-----\n",
            ))
        }
    }

    /// A signer that captures the data it was asked to sign, for verification.
    struct CapturingSigner {
        captured: std::sync::Mutex<Vec<u8>>,
    }

    impl CapturingSigner {
        fn new() -> Self {
            CapturingSigner {
                captured: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn captured_data(&self) -> Vec<u8> {
            self.captured.lock().unwrap().clone()
        }
    }

    impl Sign for CapturingSigner {
        fn sign(&self, data: &[u8]) -> Result<gix::bstr::BString, Box<dyn std::error::Error + Send + Sync>> {
            *self.captured.lock().unwrap() = data.to_vec();
            Ok(gix::bstr::BString::from(
                "-----BEGIN PGP SIGNATURE-----\n\
                 captured-signer\n\
                 -----END PGP SIGNATURE-----\n",
            ))
        }
    }

    fn repo_rw() -> crate::Result<(gix::Repository, gix_testtools::tempfile::TempDir)> {
        crate::util::repo_rw("make_reset_repo.sh")
    }

    #[test]
    fn signed_commit_has_gpgsig_header() -> crate::Result {
        let (repo, _tmp) = repo_rw()?;

        let head_commit = repo.head_commit()?;
        let tree_id = head_commit.tree_id()?;

        let signer = MockSigner;
        let commit_id = repo.commit_signed("HEAD", "signed commit message", tree_id, [head_commit.id], &signer)?;

        let commit = repo.find_commit(commit_id)?;
        let decoded = commit.decode()?;

        let gpgsig = decoded.extra_headers().pgp_signature();
        assert!(gpgsig.is_some(), "signed commit should have a gpgsig extra header");

        let sig_value = gpgsig.unwrap();
        assert!(
            sig_value.starts_with(b"-----BEGIN PGP SIGNATURE-----"),
            "signature should start with PGP header, got: {:?}",
            sig_value
        );

        assert_eq!(decoded.message, "signed commit message", "commit message should match");

        Ok(())
    }

    #[test]
    fn signed_commit_updates_reference() -> crate::Result {
        let (repo, _tmp) = repo_rw()?;

        let head_commit = repo.head_commit()?;
        let tree_id = head_commit.tree_id()?;
        let old_head = head_commit.id;

        let signer = MockSigner;
        let commit_id = repo.commit_signed("HEAD", "another signed commit", tree_id, [old_head], &signer)?;

        let new_head = repo.head_id()?;
        assert_eq!(
            new_head.detach(),
            commit_id.detach(),
            "HEAD should point to the newly created signed commit"
        );

        Ok(())
    }

    #[test]
    fn signed_tag_has_signature() -> crate::Result {
        let (repo, _tmp) = repo_rw()?;

        let head_commit = repo.head_commit()?;
        let signer = MockSigner;

        let tag_ref = repo.tag_signed(
            "v1.0-signed",
            head_commit.id,
            gix::objs::Kind::Commit,
            None,
            "signed tag message",
            gix_ref::transaction::PreviousValue::MustNotExist,
            &signer,
        )?;

        let tag_obj = tag_ref.id().object()?;
        let tag = tag_obj.try_into_tag()?;
        let decoded = tag.decode()?;

        assert!(
            decoded.pgp_signature.is_some(),
            "signed tag should have a PGP signature"
        );

        let sig = decoded.pgp_signature.unwrap();
        assert!(
            sig.starts_with(b"-----BEGIN PGP SIGNATURE-----"),
            "tag signature should start with PGP header"
        );

        assert_eq!(decoded.message, "signed tag message");
        assert_eq!(decoded.name, "v1.0-signed");

        Ok(())
    }

    #[test]
    fn unsigned_commit_has_no_gpgsig() -> crate::Result {
        let (repo, _tmp) = repo_rw()?;

        let head_commit = repo.head_commit()?;
        let decoded = head_commit.decode()?;

        let gpgsig = decoded.extra_headers().pgp_signature();
        assert!(gpgsig.is_none(), "regular commit should not have a gpgsig header");

        Ok(())
    }

    #[test]
    fn signing_config_defaults() -> crate::Result {
        let (repo, _tmp) = repo_rw()?;

        let config = repo.signing_config()?;
        assert_eq!(config.format, gix::SigningFormat::Gpg, "default format should be Gpg");
        assert!(config.key.is_none(), "no signing key should be configured by default");

        Ok(())
    }

    #[test]
    fn signer_error_propagates() -> crate::Result {
        let (repo, _tmp) = repo_rw()?;

        struct FailingSigner;
        impl Sign for FailingSigner {
            fn sign(&self, _data: &[u8]) -> Result<gix::bstr::BString, Box<dyn std::error::Error + Send + Sync>> {
                Err("signing failed intentionally".into())
            }
        }

        let head_commit = repo.head_commit()?;
        let tree_id = head_commit.tree_id()?;

        let result = repo.commit_signed("HEAD", "should fail", tree_id, [head_commit.id], &FailingSigner);

        assert!(result.is_err(), "commit_signed should fail when signer fails");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("signing failed intentionally"),
            "error should contain signer's message: {}",
            err
        );

        Ok(())
    }

    /// Ported from t7510: verify the data passed to the signer is the correct unsigned
    /// commit serialization (the commit without the gpgsig header).
    #[test]
    fn signer_receives_correct_unsigned_commit_data() -> crate::Result {
        let (repo, _tmp) = repo_rw()?;

        let head_commit = repo.head_commit()?;
        let tree_id = head_commit.tree_id()?;

        let signer = CapturingSigner::new();
        let _commit_id = repo.commit_signed("HEAD", "verify signer data", tree_id, [head_commit.id], &signer)?;

        let captured = signer.captured_data();
        assert!(!captured.is_empty(), "signer should have been called with data");

        let captured_str = String::from_utf8_lossy(&captured);
        assert!(
            captured_str.contains("tree "),
            "signed data should contain 'tree' header"
        );
        assert!(
            captured_str.contains("parent "),
            "signed data should contain 'parent' header"
        );
        assert!(
            captured_str.contains("author "),
            "signed data should contain 'author' header"
        );
        assert!(
            captured_str.contains("committer "),
            "signed data should contain 'committer' header"
        );
        assert!(
            captured_str.contains("verify signer data"),
            "signed data should contain the commit message"
        );
        assert!(
            !captured_str.contains("gpgsig"),
            "signed data should NOT contain gpgsig header"
        );

        // Verify the captured data can be parsed as a commit
        let parsed = gix_object::CommitRef::from_bytes(&captured);
        assert!(
            parsed.is_ok(),
            "captured data should be a parseable commit: {:?}",
            parsed.err()
        );

        Ok(())
    }

    /// Ported from t7510: create multiple signed commits in sequence, verifying the
    /// parent chain is correct (like the 'create signed commits' test).
    #[test]
    fn multiple_signed_commits_in_sequence() -> crate::Result {
        let (repo, _tmp) = repo_rw()?;
        let signer = MockSigner;

        let head = repo.head_commit()?;
        let tree_id = head.tree_id()?;

        let first_signed = repo.commit_signed("HEAD", "first signed", tree_id, [head.id], &signer)?;
        let second_signed = repo.commit_signed("HEAD", "second signed", tree_id, [first_signed.detach()], &signer)?;
        let third_signed = repo.commit_signed("HEAD", "third signed", tree_id, [second_signed.detach()], &signer)?;

        // Verify the chain: third -> second -> first -> original head
        let third = repo.find_commit(third_signed)?;
        let third_decoded = third.decode()?;
        assert!(third_decoded.extra_headers().pgp_signature().is_some());
        let third_parents: Vec<_> = third_decoded.parents().collect();
        assert_eq!(third_parents, vec![second_signed.detach()]);

        let second = repo.find_commit(second_signed)?;
        let second_decoded = second.decode()?;
        assert!(second_decoded.extra_headers().pgp_signature().is_some());
        let second_parents: Vec<_> = second_decoded.parents().collect();
        assert_eq!(second_parents, vec![first_signed.detach()]);

        let first = repo.find_commit(first_signed)?;
        let first_decoded = first.decode()?;
        assert!(first_decoded.extra_headers().pgp_signature().is_some());
        let first_parents: Vec<_> = first_decoded.parents().collect();
        assert_eq!(first_parents, vec![head.id]);

        assert_eq!(repo.head_id()?.detach(), third_signed.detach());

        Ok(())
    }

    /// Ported from t7510: signed commit preserves correct tree, author, committer fields.
    #[test]
    fn signed_commit_preserves_metadata() -> crate::Result {
        let (repo, _tmp) = repo_rw()?;
        let signer = MockSigner;

        let head = repo.head_commit()?;
        let tree_id = head.tree_id()?;

        let commit_id = repo.commit_signed("HEAD", "metadata preservation test", tree_id, [head.id], &signer)?;

        let commit = repo.find_commit(commit_id)?;
        let decoded = commit.decode()?;

        assert_eq!(decoded.tree(), tree_id.detach());

        let parents: Vec<_> = decoded.parents().collect();
        assert_eq!(parents.len(), 1);
        assert_eq!(parents[0], head.id);

        let author = decoded.author()?;
        assert!(!author.name.is_empty(), "author name should be set");
        let committer_sig = decoded.committer()?;
        assert!(!committer_sig.name.is_empty(), "committer name should be set");

        assert_eq!(decoded.message, "metadata preservation test");
        assert!(decoded.extra_headers().pgp_signature().is_some());

        Ok(())
    }

    /// Ported from t7510: signed tag with a tagger identity should preserve it.
    #[test]
    fn signed_tag_with_tagger() -> crate::Result {
        let (repo, _tmp) = repo_rw()?;
        let signer = MockSigner;

        let head = repo.head_commit()?;

        let tagger = gix_actor::SignatureRef {
            name: "Test Tagger".into(),
            email: "tagger@example.com".into(),
            time: "1234567890 +0000",
        };

        let tag_ref = repo.tag_signed(
            "v2.0-signed",
            head.id,
            gix::objs::Kind::Commit,
            Some(tagger),
            "tag with tagger",
            gix_ref::transaction::PreviousValue::MustNotExist,
            &signer,
        )?;

        let tag_obj = tag_ref.id().object()?;
        let tag = tag_obj.try_into_tag()?;
        let decoded = tag.decode()?;

        assert!(decoded.pgp_signature.is_some(), "tag should be signed");
        assert_eq!(decoded.name, "v2.0-signed");
        assert_eq!(decoded.message, "tag with tagger");

        let tag_tagger = decoded.tagger()?.expect("tagger should be present");
        assert_eq!(tag_tagger.name, "Test Tagger");
        assert_eq!(tag_tagger.email, "tagger@example.com");

        Ok(())
    }

    /// Ported from t7510: signed commit with explicit author and committer
    /// using commit_signed_as.
    #[test]
    fn commit_signed_as_with_explicit_identities() -> crate::Result {
        let (repo, _tmp) = repo_rw()?;
        let signer = MockSigner;

        let head = repo.head_commit()?;
        let tree_id = head.tree_id()?;

        let author = gix_actor::SignatureRef {
            name: "Custom Author".into(),
            email: "author@test.com".into(),
            time: "1000000000 +0100",
        };
        let committer = gix_actor::SignatureRef {
            name: "Custom Committer".into(),
            email: "committer@test.com".into(),
            time: "1000000001 +0100",
        };

        let commit_id = repo.commit_signed_as(
            committer,
            author,
            "HEAD",
            "explicit identities",
            tree_id,
            [head.id],
            &signer,
        )?;

        let commit = repo.find_commit(commit_id)?;
        let decoded = commit.decode()?;

        assert!(decoded.extra_headers().pgp_signature().is_some());
        let parsed_author = decoded.author()?;
        assert_eq!(parsed_author.name, "Custom Author");
        assert_eq!(parsed_author.email, "author@test.com");
        let parsed_committer = decoded.committer()?;
        assert_eq!(parsed_committer.name, "Custom Committer");
        assert_eq!(parsed_committer.email, "committer@test.com");
        assert_eq!(decoded.message, "explicit identities");

        Ok(())
    }

    /// Ported from t7510: an unsigned tag has no pgp_signature field.
    #[test]
    fn unsigned_tag_has_no_signature() -> crate::Result {
        let (repo, _tmp) = repo_rw()?;

        let head = repo.head_commit()?;

        let tag_ref = repo.tag(
            "v-unsigned",
            head.id,
            gix::objs::Kind::Commit,
            None,
            "unsigned tag",
            gix_ref::transaction::PreviousValue::MustNotExist,
        )?;

        let tag_obj = tag_ref.id().object()?;
        let tag = tag_obj.try_into_tag()?;
        let decoded = tag.decode()?;

        assert!(
            decoded.pgp_signature.is_none(),
            "unsigned tag should have no PGP signature"
        );

        Ok(())
    }

    /// Ported from t7510: verify signer receives correct unsigned tag data.
    #[test]
    fn signer_receives_correct_unsigned_tag_data() -> crate::Result {
        let (repo, _tmp) = repo_rw()?;

        let head = repo.head_commit()?;
        let signer = CapturingSigner::new();

        let _tag_ref = repo.tag_signed(
            "v-capture",
            head.id,
            gix::objs::Kind::Commit,
            None,
            "capture tag data",
            gix_ref::transaction::PreviousValue::MustNotExist,
            &signer,
        )?;

        let captured = signer.captured_data();
        assert!(!captured.is_empty(), "signer should have been called with tag data");

        let captured_str = String::from_utf8_lossy(&captured);
        assert!(
            captured_str.contains("object "),
            "tag data should contain 'object' header"
        );
        assert!(
            captured_str.contains("type commit"),
            "tag data should contain 'type commit'"
        );
        assert!(
            captured_str.contains("tag v-capture"),
            "tag data should contain the tag name"
        );
        assert!(
            captured_str.contains("capture tag data"),
            "tag data should contain the tag message"
        );
        assert!(
            !captured_str.contains("-----BEGIN PGP"),
            "unsigned tag data should not contain a signature"
        );

        Ok(())
    }

    /// Ported from t7510: signing_config reads gpg.format = ssh correctly.
    #[test]
    fn signing_config_reads_ssh_format() -> crate::Result {
        let (mut repo, _tmp) = repo_rw()?;
        {
            let mut config = repo.config_snapshot_mut();
            config.set_raw_value(&"gpg.format", "ssh")?;
            config.set_raw_value(&"user.signingKey", "~/.ssh/id_ed25519")?;
            config.commit()?;
        }

        let config = repo.signing_config()?;
        assert_eq!(
            config.format,
            gix::SigningFormat::Ssh,
            "gpg.format=ssh should yield Ssh format"
        );
        assert!(
            config.key.as_ref().map_or(false, |k| k == "~/.ssh/id_ed25519"),
            "user.signingKey should be read: {:?}",
            config.key
        );
        Ok(())
    }

    /// Ported from t7510: signing_config reads gpg.format = x509 correctly.
    #[test]
    fn signing_config_reads_x509_format() -> crate::Result {
        let (mut repo, _tmp) = repo_rw()?;
        {
            let mut config = repo.config_snapshot_mut();
            config.set_raw_value(&"gpg.format", "x509")?;
            config.commit()?;
        }

        let config = repo.signing_config()?;
        assert_eq!(
            config.format,
            gix::SigningFormat::X509,
            "gpg.format=x509 should yield X509 format"
        );
        Ok(())
    }

    /// Ported from t7510: signing_config reads gpg.format = openpgp as Gpg.
    #[test]
    fn signing_config_reads_openpgp_format() -> crate::Result {
        let (mut repo, _tmp) = repo_rw()?;
        {
            let mut config = repo.config_snapshot_mut();
            config.set_raw_value(&"gpg.format", "openpgp")?;
            config.commit()?;
        }

        let config = repo.signing_config()?;
        assert_eq!(
            config.format,
            gix::SigningFormat::Gpg,
            "gpg.format=openpgp should yield Gpg format"
        );
        Ok(())
    }

    /// Ported from t7510: unknown gpg.format should produce an error.
    #[test]
    fn signing_config_rejects_unknown_format() -> crate::Result {
        let (mut repo, _tmp) = repo_rw()?;
        {
            let mut config = repo.config_snapshot_mut();
            config.set_raw_value(&"gpg.format", "unknown-format")?;
            config.commit()?;
        }

        let result = repo.signing_config();
        assert!(result.is_err(), "unknown gpg.format should produce an error");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("nknown signing format"),
            "error should mention unknown format: {err}"
        );
        Ok(())
    }

    /// Ported from t7510: signing_config reads custom program paths.
    #[test]
    fn signing_config_reads_program_paths() -> crate::Result {
        let (mut repo, _tmp) = repo_rw()?;
        {
            let mut config = repo.config_snapshot_mut();
            config.set_raw_value(&"gpg.program", "/usr/bin/gpg2")?;
            config.set_raw_value(&"gpg.ssh.program", "/usr/bin/ssh-keygen")?;
            config.set_raw_value(&"gpg.x509.program", "/usr/bin/gpgsm")?;
            config.commit()?;
        }

        let config = repo.signing_config()?;
        assert!(
            config.gpg_program.as_ref().map_or(false, |p| p == "/usr/bin/gpg2"),
            "gpg.program should be set: {:?}",
            config.gpg_program
        );
        assert!(
            config
                .ssh_program
                .as_ref()
                .map_or(false, |p| p == "/usr/bin/ssh-keygen"),
            "gpg.ssh.program should be set: {:?}",
            config.ssh_program
        );
        assert!(
            config.x509_program.as_ref().map_or(false, |p| p == "/usr/bin/gpgsm"),
            "gpg.x509.program should be set: {:?}",
            config.x509_program
        );
        Ok(())
    }

    /// Ported from t7510: SigningFormat default is Gpg.
    #[test]
    fn signing_format_default_is_gpg() {
        assert_eq!(
            gix::SigningFormat::default(),
            gix::SigningFormat::Gpg,
            "default signing format should be Gpg"
        );
    }

    /// Ported from t7510: signed tag targeting a tree (not commit).
    #[test]
    fn signed_tag_targeting_tree() -> crate::Result {
        let (repo, _tmp) = repo_rw()?;
        let signer = MockSigner;

        let head = repo.head_commit()?;
        let tree_id = head.tree_id()?;

        let tag_ref = repo.tag_signed(
            "v-tree-tag",
            tree_id,
            gix::objs::Kind::Tree,
            None,
            "tag on a tree",
            gix_ref::transaction::PreviousValue::MustNotExist,
            &signer,
        )?;

        let tag_obj = tag_ref.id().object()?;
        let tag = tag_obj.try_into_tag()?;
        let decoded = tag.decode()?;

        assert!(decoded.pgp_signature.is_some(), "tag should be signed");
        assert_eq!(decoded.target_kind, gix::objs::Kind::Tree);
        assert_eq!(decoded.name, "v-tree-tag");

        Ok(())
    }
}
