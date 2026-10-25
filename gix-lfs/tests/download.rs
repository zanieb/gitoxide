#![cfg(feature = "download")]

use gix_lfs::{Pointer, download};

const DATA: &[u8] = b"uv LFS fixture\n";
const OID: &str = "b8d86944388b573611669f708489a4a940b4cf1257f87237cad657fd79447aab";
const POINTER: Pointer<'_> = Pointer { oid: OID, size: 15 };

#[test]
fn verifies_streamed_contents_before_accepting_an_object() -> gix_error::TestResult {
    let mut copied = Vec::new();
    download::copy_verified(POINTER, DATA, &mut copied)?;
    assert_eq!(copied, DATA, "the verified contents are passed to the writer");
    for invalid in [b"short".as_slice(), b"uv LFS fixture!\n", b"uv LFS fixturE\n"] {
        assert!(
            download::copy_verified(POINTER, invalid, std::io::sink()).is_err(),
            "both the size and digest must match"
        );
    }
    Ok(())
}

#[test]
fn verifies_batch_objects_and_keeps_action_headers() -> gix_error::TestResult {
    let request = download::request(&[POINTER])?;
    let request: serde_json::Value = serde_json::from_slice(&request)?;
    assert_eq!(
        request["objects"][0]["oid"], OID,
        "the batch identifies the requested object"
    );
    assert_eq!(request["transfers"][0], "basic", "only the basic transfer is requested");
    let object = serde_json::json!({"oid":OID,"size":15,"actions":{"download":{"href":"https://example.invalid/object","header":{"Private-Token":"action"}}}});
    let response = |objects| serde_json::to_vec(&serde_json::json!({"objects":objects}));
    let actions = download::actions(&[POINTER], &response(vec![object.clone()])?)?;
    assert_eq!(
        actions[OID].header["Private-Token"], "action",
        "action headers belong to the download request"
    );
    for objects in [
        vec![],
        vec![object.clone(), object.clone()],
        vec![serde_json::json!({"oid":OID,"size":16,"actions":object["actions"]})],
        vec![serde_json::json!({"oid":OID,"size":15,"error":{"code":404,"message":"not found"}})],
        vec![serde_json::json!({"oid":"unrequested","size":15,"actions":object["actions"]})],
    ] {
        assert!(
            download::actions(&[POINTER], &response(objects)?).is_err(),
            "a batch must cover each requested object exactly once"
        );
    }
    Ok(())
}
