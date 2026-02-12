use std::{
    io::Write,
    sync::atomic::{AtomicBool, Ordering},
};

use gix_features::progress::DynNestedProgress;

use crate::push::{self, Command, Error, Options, Outcome, ProgressId};
#[cfg(feature = "async-client")]
use crate::transport::client::async_io::{ExtendedBufRead, HandleProgress, Transport};
#[cfg(feature = "blocking-client")]
use crate::transport::client::blocking_io::{ExtendedBufRead, HandleProgress, Transport};

/// Perform one push operation using the given `transport`.
///
/// `commands` are the reference updates to send to the server.
///
/// `write_pack` is called with a writer to which the pack data should be written.
/// It receives:
/// * A writer to send pack data to the server
/// * Progress reporting
/// * An interrupt flag
///
/// The function returns `Ok(true)` if it wrote pack data, or `Ok(false)` if there
/// was no data to send (e.g. all commands are deletes).
///
/// `Context` provides the transport and handshake info, similar to [`fetch::Context`](crate::fetch::Context).
///
/// **Note that the interaction will never be ended**, even on error or failure, leaving it up to the caller to do that.
#[maybe_async::maybe_async]
pub async fn push<P, T, E>(
    commands: &[Command],
    write_pack: impl FnOnce(&mut dyn Write, &mut dyn DynNestedProgress, &AtomicBool) -> Result<bool, E>,
    mut progress: P,
    should_interrupt: &AtomicBool,
    handshake: &crate::Handshake,
    transport: &mut T,
    user_agent: (&'static str, Option<std::borrow::Cow<'static, str>>),
    trace_packetlines: bool,
    options: Options,
) -> Result<Outcome, Error>
where
    P: gix_features::progress::NestedProgress,
    P::SubProgress: 'static,
    T: Transport,
    E: Into<Box<dyn std::error::Error + Send + Sync + 'static>>,
{
    let _span = gix_trace::coarse!("gix_protocol::push()");

    if commands.is_empty() {
        return Err(Error::NoCommands);
    }

    if should_interrupt.load(Ordering::Relaxed) {
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "push interrupted before starting",
        )));
    }

    let capabilities = &handshake.capabilities;
    let protocol_version = handshake.server_protocol_version;

    // Check for report-status capability (required to know if push succeeded).
    let has_report_status = capabilities.contains("report-status");
    let has_atomic = capabilities.contains("atomic");
    let has_side_band_64k = capabilities.contains("side-band-64k");
    let has_delete_refs = capabilities.contains("delete-refs");
    let has_ofs_delta = capabilities.contains("ofs-delta");
    let _has_push_options = capabilities.contains("push-options");

    if options.atomic && !has_atomic {
        return Err(Error::AtomicNotSupported);
    }

    // Check if any command is a delete and requires delete-refs capability.
    let has_deletes = commands.iter().any(|c| c.is_delete());
    if has_deletes && !has_delete_refs {
        return Err(Error::MissingServerCapability {
            feature: "delete-refs",
            description: "The server does not support deleting references",
        });
    }

    // Determine if we need to send a pack (any non-delete command).
    let needs_pack = commands.iter().any(|c| !c.is_delete());

    progress.set_name("pushing".into());

    // Build the capability string for the first command line.
    let mut cap_list = Vec::new();
    if has_report_status {
        cap_list.push("report-status");
    }
    if has_ofs_delta {
        cap_list.push("ofs-delta");
    }
    if has_side_band_64k {
        cap_list.push("side-band-64k");
    }
    if options.atomic && has_atomic {
        cap_list.push("atomic");
    }
    // Add agent
    let agent_str = match &user_agent.1 {
        Some(v) => format!("agent={}={}", user_agent.0, v),
        None => format!("agent={}", user_agent.0),
    };

    // For V1/V0 protocol, we write commands as packet lines directly using the transport's
    // request writer. We use Binary write mode since we're sending raw data (ref updates + pack).
    let use_sideband = has_side_band_64k
        && matches!(
            protocol_version,
            gix_transport::Protocol::V0 | gix_transport::Protocol::V1
        );

    let mut writer = transport.request(
        gix_transport::client::WriteMode::OneLfTerminatedLinePerWriteCall,
        gix_transport::client::MessageKind::Flush,
        trace_packetlines,
    )?;

    // Write the commands. The first command line includes capabilities.
    // We build command lines as raw bytes to avoid lossy UTF-8 conversion of ref names,
    // which are arbitrary byte strings in Git.
    for (idx, cmd) in commands.iter().enumerate() {
        let line = if idx == 0 {
            let mut caps = cap_list.join(" ");
            if !caps.is_empty() {
                caps.insert(0, ' ');
            }
            let mut buf = format!("{} {} ", cmd.old_id, cmd.new_id).into_bytes();
            buf.extend_from_slice(&cmd.ref_name);
            buf.push(0); // NUL separator before capabilities
            buf.extend_from_slice(agent_str.as_bytes());
            buf.extend_from_slice(caps.as_bytes());
            buf
        } else {
            let mut buf = format!("{} {} ", cmd.old_id, cmd.new_id).into_bytes();
            buf.extend_from_slice(&cmd.ref_name);
            buf
        };
        #[cfg(feature = "blocking-client")]
        {
            writer.write_all(&line)?;
        }
        #[cfg(feature = "async-client")]
        {
            use futures_lite::AsyncWriteExt;
            writer.write_all(&line).await?;
        }
    }

    // Send the pack if needed.
    if needs_pack {
        // Transition to binary mode for pack data.
        // We need to send a flush first to end the command list, then the pack data.
        let (mut raw_writer, mut reader) = writer.into_parts();

        // Write flush packet to end the command list.
        // We write the raw "0000" bytes directly because `into_parts()` gave us the raw transport
        // writer (not a packetline writer). A flush packet is defined as exactly these 4 ASCII bytes
        // in the Git protocol spec, regardless of protocol version. This matches C Git's
        // `packet_flush()` behavior at the wire level.
        //
        // NOTE: This assumes `into_parts()` returns a writer that does NOT add packetline framing.
        // If the transport wraps output (e.g., HTTP smart protocol chunked encoding), this must be
        // handled at a lower layer. The `gix-transport` crate guarantees this for all built-in
        // transports: `into_parts()` yields the raw byte stream.
        #[cfg(feature = "blocking-client")]
        {
            raw_writer.write_all(b"0000")?;
            raw_writer.flush()?;
        }
        #[cfg(feature = "async-client")]
        {
            use futures_lite::AsyncWriteExt;
            raw_writer.write_all(b"0000").await?;
            raw_writer.flush().await?;
        }

        // Write pack data.
        // In async mode, wrap the AsyncWrite in BlockOn to provide a sync Write interface
        // for the write_pack callback, following the same pattern fetch uses for consume_pack.
        progress.set_name("sending pack".into());
        #[cfg(feature = "blocking-client")]
        let wrote_pack = {
            write_pack(&mut *raw_writer, &mut progress, should_interrupt)
                .map_err(|e| Error::PackGeneration(e.into()))?
        };
        #[cfg(feature = "async-client")]
        let wrote_pack = {
            let mut blocking_writer = crate::futures_lite::io::BlockOn::new(raw_writer);
            let result = write_pack(&mut blocking_writer, &mut progress, should_interrupt)
                .map_err(|e| Error::PackGeneration(e.into()))?;
            raw_writer = blocking_writer.into_inner();
            result
        };

        if !wrote_pack {
            // Write an empty pack (header + trailer) if write_pack says nothing to send.
            // Git expects a pack even if there are only deletes... but we checked needs_pack above,
            // so this shouldn't normally happen.
        }
        #[cfg(feature = "blocking-client")]
        {
            raw_writer.flush()?;
        }
        #[cfg(feature = "async-client")]
        {
            use futures_lite::AsyncWriteExt;
            raw_writer.flush().await?;
        }

        // Drop the writer to signal we're done sending, then read the response.
        drop(raw_writer);

        // Reset the reader so it can read past any previous flush packet (from the handshake)
        // and parse the server's push response.
        reader.reset(protocol_version);
        let reader = Some(reader);
        parse_push_response(
            reader,
            commands,
            has_report_status,
            &mut progress,
            should_interrupt,
            use_sideband,
        )
        .await
    } else {
        // All deletes, no pack needed.
        let reader = writer.into_read().await?;
        parse_push_response(
            Some(reader),
            commands,
            has_report_status,
            &mut progress,
            should_interrupt,
            use_sideband,
        )
        .await
    }
}

#[maybe_async::maybe_async]
async fn read_response<'a>(
    mut reader: Box<dyn ExtendedBufRead<'a> + Unpin + 'a>,
    progress: &mut dyn DynNestedProgress,
    should_interrupt: &'a AtomicBool,
    _use_sideband: bool,
) -> Result<Vec<u8>, Error> {
    // Set up sideband progress handling.
    reader.set_progress_handler(Some(Box::new({
        let mut remote_progress = progress.add_child_with_id("remote".to_string(), ProgressId::RemoteProgress.into());
        move |is_err: bool, data: &[u8]| {
            crate::RemoteProgress::translate_to_progress(is_err, data, &mut remote_progress);
            if should_interrupt.load(Ordering::Relaxed) {
                std::ops::ControlFlow::Break(())
            } else {
                std::ops::ControlFlow::Continue(())
            }
        }
    }) as HandleProgress<'a>));

    // Read pktline-framed data from the server, extracting the content of each data packet.
    // The server sends the report-status response inside sideband channel 1 (when side-band-64k
    // is active). Each sideband data packet may contain inner pktline-framed data. We need to
    // extract the content from both layers of framing.
    let mut raw_sideband_data = Vec::new();
    {
        #[cfg(feature = "async-client")]
        use crate::transport::client::async_io::ReadlineBufRead;
        #[cfg(feature = "blocking-client")]
        use crate::transport::client::blocking_io::ReadlineBufRead;
        while let Some(result) = reader.readline().await {
            match result? {
                Ok(line) => {
                    if let Some(data) = line.as_slice() {
                        raw_sideband_data.extend_from_slice(data);
                    }
                }
                Err(_) => break,
            }
        }
    }

    // The sideband data may contain inner pktlines. Parse them to extract the text content.
    let response_data = extract_inner_pktline_content(&raw_sideband_data);
    Ok(response_data)
}

#[maybe_async::maybe_async]
async fn parse_push_response<'a>(
    reader: Option<Box<dyn ExtendedBufRead<'a> + Unpin + 'a>>,
    commands: &[Command],
    has_report_status: bool,
    progress: &mut dyn DynNestedProgress,
    should_interrupt: &'a AtomicBool,
    use_sideband: bool,
) -> Result<Outcome, Error> {
    if has_report_status {
        let reader = reader.expect("reader is provided when has_report_status is true");
        let response_data = read_response(reader, progress, should_interrupt, use_sideband).await?;
        let (unpack_status, ref_updates) = push::response::parse_v1(&response_data)?;

        if let push::response::UnpackStatus::Failed { ref reason } = unpack_status {
            return Err(Error::UnpackFailed {
                reason: String::from_utf8_lossy(reason).into_owned(),
            });
        }

        Ok(Outcome {
            ref_updates,
            unpack_status,
        })
    } else {
        Ok(Outcome {
            ref_updates: commands
                .iter()
                .map(|c| push::response::StatusV1::Ok {
                    ref_name: c.ref_name.clone(),
                })
                .collect(),
            unpack_status: push::response::UnpackStatus::Ok,
        })
    }
}

/// Extract text content from pktline-framed data.
///
/// When the server sends report-status over a sideband channel, the sideband data
/// itself contains pktline-framed status lines. This function parses those inner
/// pktlines and returns the concatenated text content.
///
/// If the data doesn't look like valid pktlines, it's returned as-is (the server
/// may not use inner pktline framing in some protocol configurations).
fn extract_inner_pktline_content(data: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    let mut pos = 0;

    // Skip sideband channel byte if present (0x01 = data, 0x02 = progress, 0x03 = error).
    // The sideband reader may pass through the channel marker.
    if !data.is_empty() && data[0] == 0x01 {
        pos = 1;
    }

    // Try to parse as pktlines: each starts with a 4-char hex length prefix
    while pos + 4 <= data.len() {
        let len_str = match std::str::from_utf8(&data[pos..pos + 4]) {
            Ok(s) => s,
            Err(_) => {
                // Not valid pktline framing; return raw data
                return data.to_vec();
            }
        };
        let pkt_len = match u16::from_str_radix(len_str, 16) {
            Ok(n) => n as usize,
            Err(_) => {
                // Not valid pktline framing; return raw data
                return data.to_vec();
            }
        };
        if pkt_len == 0 {
            // Flush packet -- end of pktlines
            break;
        }
        if pkt_len < 4 || pos + pkt_len > data.len() {
            // Invalid length or truncated; return raw data
            return data.to_vec();
        }
        // Content is after the 4-byte length prefix
        result.extend_from_slice(&data[pos + 4..pos + pkt_len]);
        pos += pkt_len;
    }

    if result.is_empty() && !data.is_empty() {
        // Couldn't parse anything as pktlines; return raw data
        data.to_vec()
    } else {
        result
    }
}
