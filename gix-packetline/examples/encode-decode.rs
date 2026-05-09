use gix_packetline::{decode::Stream, PacketLineRef};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut wire = Vec::new();
    gix_packetline::blocking_io::encode::data_to_write(b"hello", &mut wire)?;
    gix_packetline::blocking_io::encode::flush_to_write(&mut wire)?;

    println!("wire: {}", String::from_utf8_lossy(&wire));

    let Stream::Complete {
        line: first,
        bytes_consumed,
    } = gix_packetline::decode::streaming(&wire)?
    else {
        unreachable!("example writes a complete packet line")
    };
    if let PacketLineRef::Data(data) = first {
        println!("data: {}", String::from_utf8_lossy(data));
    }

    let Stream::Complete { line: second, .. } = gix_packetline::decode::streaming(&wire[bytes_consumed..])? else {
        unreachable!("flush packet is complete")
    };
    println!("terminator: {second:?}");

    Ok(())
}
