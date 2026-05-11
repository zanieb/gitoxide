use std::{borrow::Cow, sync::atomic::AtomicBool};

use gix_protocol::{
    Handshake,
    push::{Command, Options, response},
};
use gix_transport::{Protocol, client::Capabilities};

#[cfg(feature = "blocking-client")]
type Cursor = std::io::Cursor<Vec<u8>>;
#[cfg(feature = "async-client")]
type Cursor = futures_lite::io::Cursor<Vec<u8>>;

#[cfg(feature = "async-client")]
fn transport<W: futures_io::AsyncWrite + Unpin>(
    response: Vec<u8>,
    out: W,
) -> gix_transport::client::git::async_io::Connection<Cursor, W> {
    gix_transport::client::git::async_io::Connection::new(
        Cursor::new(response),
        out,
        Protocol::V1,
        "does/not/matter",
        None::<(&str, _)>,
        gix_transport::client::git::ConnectMode::Process,
        false,
    )
}

#[cfg(feature = "blocking-client")]
fn transport<W: std::io::Write>(
    response: Vec<u8>,
    out: W,
) -> gix_transport::client::git::blocking_io::Connection<Cursor, W> {
    gix_transport::client::git::blocking_io::Connection::new(
        Cursor::new(response),
        out,
        Protocol::V1,
        "does/not/matter",
        None::<(&str, _)>,
        gix_transport::client::git::ConnectMode::Process,
        false,
    )
}

fn oid(hex_sha: &str) -> gix_hash::ObjectId {
    gix_hash::ObjectId::from_hex(hex_sha.as_bytes()).expect("valid oid")
}

fn handshake(capabilities: &[u8]) -> Handshake {
    let (capabilities, _) = Capabilities::from_bytes(capabilities).expect("valid capabilities");
    Handshake {
        server_protocol_version: Protocol::V1,
        refs: None,
        v1_shallow_updates: None,
        capabilities,
    }
}

#[maybe_async::test(feature = "blocking-client", async(feature = "async-client", async_std::test))]
async fn command_list_uses_binary_packetlines_and_valid_agent_capability() -> crate::Result {
    let mut transport = transport(b"000eunpack ok\n0017ok refs/heads/main\n0000".to_vec(), Vec::new());
    let old_id = oid("1111111111111111111111111111111111111111");
    let new_id = gix_hash::ObjectId::null(gix_hash::Kind::Sha1);
    let command = Command::new("refs/heads/main", old_id, new_id);

    let outcome = gix_protocol::push(
        std::slice::from_ref(&command),
        |_, _, _| -> Result<bool, std::io::Error> { panic!("delete-only push must not write a pack") },
        gix_features::progress::Discard,
        &AtomicBool::default(),
        &handshake(b"\0report-status delete-refs atomic"),
        &mut transport,
        ("agent", Some(Cow::Borrowed("git/test"))),
        false,
        Options {
            dry_run: false,
            atomic: true,
        },
    )
    .await?;

    assert!(matches!(
        outcome.ref_updates.as_slice(),
        [response::StatusV1::Ok { ref_name }] if ref_name.as_ref() as &[u8] == b"refs/heads/main"
    ));

    let written = transport.into_inner().1;
    let payload_len = usize::from_str_radix(std::str::from_utf8(&written[..4])?, 16)?;
    let payload = &written[4..payload_len];
    assert_eq!(
        payload,
        format!("{old_id} {new_id} refs/heads/main\0agent=git/test report-status atomic").as_bytes()
    );
    assert_eq!(
        &written[payload_len..],
        b"0000",
        "command list must end with a traced flush packet before any pack data"
    );
    Ok(())
}

#[maybe_async::test(feature = "blocking-client", async(feature = "async-client", async_std::test))]
async fn dry_run_does_not_touch_transport_or_write_pack() -> crate::Result {
    let mut transport = transport(Vec::new(), Vec::new());
    let command = Command::new(
        "refs/heads/main",
        gix_hash::ObjectId::null(gix_hash::Kind::Sha1),
        oid("2222222222222222222222222222222222222222"),
    );

    let outcome = gix_protocol::push(
        std::slice::from_ref(&command),
        |_, _, _| -> Result<bool, std::io::Error> { panic!("dry-run must not write a pack") },
        gix_features::progress::Discard,
        &AtomicBool::default(),
        &handshake(b"\0report-status"),
        &mut transport,
        ("agent", Some(Cow::Borrowed("git/test"))),
        false,
        Options {
            dry_run: true,
            atomic: false,
        },
    )
    .await?;

    assert!(matches!(
        outcome.ref_updates.as_slice(),
        [response::StatusV1::Ok { ref_name }] if ref_name.as_ref() as &[u8] == b"refs/heads/main"
    ));
    assert!(
        transport.into_inner().1.is_empty(),
        "dry-run must not write to the remote"
    );
    Ok(())
}
