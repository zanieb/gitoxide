use crate::pack::fixture_path;

#[test]
fn encode_decode_roundtrip() -> crate::Result {
    let buf = std::fs::read(fixture_path(
        "objects/pack/pack-11fdfa9e156ab73caae3b6da867192221f2089c2.pack",
    ))?;
    let expected_encoded_header = &buf[..12];
    let (version, num_objects) = gix_pack::data::header::decode(expected_encoded_header.try_into()?)?;
    let actual_encoded_header = gix_pack::data::header::encode(version, num_objects);
    assert_eq!(actual_encoded_header, expected_encoded_header);
    Ok(())
}

mod proptests {
    use gix_pack::data::{header, Version};
    use proptest::prelude::*;

    fn arb_version() -> impl Strategy<Value = Version> {
        prop_oneof![Just(Version::V2), Just(Version::V3),]
    }

    proptest! {
        #[test]
        fn header_encode_decode_roundtrip(version in arb_version(), num_objects: u32) {
            let encoded = header::encode(version, num_objects);
            let (decoded_version, decoded_num) = header::decode(&encoded).unwrap();
            prop_assert_eq!(decoded_version, version);
            prop_assert_eq!(decoded_num, num_objects);
        }

        #[test]
        fn header_decode_rejects_invalid_magic(data: [u8; 12]) {
            // If the first 4 bytes aren't "PACK", decode must fail
            if &data[..4] != b"PACK" {
                prop_assert!(header::decode(&data).is_err());
            }
        }

        #[test]
        fn header_decode_rejects_invalid_version(num_objects: u32) {
            // Versions other than 2 and 3 must be rejected
            for bad_version in [0u32, 1, 4, 5, 100, u32::MAX] {
                let mut buf = [0u8; 12];
                buf[..4].copy_from_slice(b"PACK");
                buf[4..8].copy_from_slice(&bad_version.to_be_bytes());
                buf[8..].copy_from_slice(&num_objects.to_be_bytes());
                prop_assert!(header::decode(&buf).is_err());
            }
        }
    }
}
