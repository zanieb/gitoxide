use gix_lfs::Pointer;

const OID: &str = "4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393";

#[test]
fn reads_standard_and_legacy_pointers() {
    for version in [
        "https://git-lfs.github.com/spec/v1",
        "https://hawser.github.com/spec/v1",
        "http://git-media.io/v/2",
    ] {
        let data = format!("version {version}\noid sha256:{OID}\nsize 12345\n");
        let pointer = Pointer::from_bytes(data.as_bytes()).expect("a standard pointer");
        assert_eq!(
            (pointer.oid, pointer.size),
            (OID, 12345),
            "the pointer identifies the original contents"
        );
    }
    let empty = Pointer::from_bytes(b"").expect("empty files pass through Git LFS unchanged");
    assert_eq!(empty.size, 0, "empty files have zero size");
    assert_eq!(
        empty.oid, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "empty files use the SHA-256 of empty input"
    );
}

#[test]
fn rejects_invalid_or_extended_pointers() {
    let data = format!("version https://git-lfs.github.com/spec/v1\noid sha256:{OID}\nsize 12345\n");
    for invalid in [
        data.replace("spec/v1", "spec/v2"),
        data.replace(OID, &OID.to_uppercase()),
        data.replace("sha256:", "sha1:"),
        data.replace("12345", "-1"),
        data.replace("12345", "18446744073709551616"),
        data.replace("oid ", &format!("ext-0-encrypted sha256:{OID}\noid ")),
        data.replace("size ", "size  "),
        data.replace("12345", "012345"),
        data.replace("oid ", "size 0\noid "),
        data.trim_end().to_owned(),
        format!("{data}{}", "x".repeat(1024)),
    ] {
        assert!(
            Pointer::from_bytes(invalid.as_bytes()).is_none(),
            "input must not be accepted as an extension-free pointer: {invalid:?}"
        );
    }
}
