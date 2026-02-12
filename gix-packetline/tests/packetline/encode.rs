mod data_to_write {
    #[cfg(feature = "blocking-io")]
    use std::io;

    use bstr::ByteSlice;
    #[cfg(all(feature = "async-io", not(feature = "blocking-io")))]
    use futures_lite::io;

    #[cfg(all(feature = "async-io", not(feature = "blocking-io")))]
    use gix_packetline::async_io::encode::data_to_write;
    #[cfg(feature = "blocking-io")]
    use gix_packetline::blocking_io::encode::data_to_write;

    #[crate::bisync::bisync]
    #[cfg_attr(feature = "blocking-io", test)]
    #[cfg_attr(all(feature = "async-io", not(feature = "blocking-io")), async_std::test)]
    async fn binary_and_non_binary() -> gix_testtools::TestResult {
        let mut out = Vec::new();
        let res = data_to_write(b"\0", &mut out).await?;
        assert_eq!(res, 5);
        assert_eq!(out.as_bstr(), b"0005\0".as_bstr());

        out.clear();
        let res = data_to_write("hello world, it works\n".as_bytes(), &mut out).await?;
        assert_eq!(res, 26);
        assert_eq!(out.as_bstr(), b"001ahello world, it works\n".as_bstr());

        Ok(())
    }

    #[crate::bisync::bisync]
    #[cfg_attr(feature = "blocking-io", test)]
    #[cfg_attr(all(feature = "async-io", not(feature = "blocking-io")), async_std::test)]
    async fn error_if_data_exceeds_limit() {
        fn vec_sized(size: usize) -> Vec<u8> {
            vec![0; size]
        }

        let res = data_to_write(&vec_sized(65516 + 1), io::sink()).await;
        let err = (res).expect_err("the packet line is invalid");
        insta::assert_debug_snapshot!(err, "error if data exceeds limit", @r#"
        Custom {
            kind: Other,
            error: Message {
                message: "Cannot encode more than 65516 bytes, got 65517",
                class: Validation,
            },
        }
        "#);
    }

    #[crate::bisync::bisync]
    #[cfg_attr(feature = "blocking-io", test)]
    #[cfg_attr(all(feature = "async-io", not(feature = "blocking-io")), async_std::test)]
    async fn error_if_data_is_empty() {
        let err = (data_to_write(&[], io::sink()).await).expect_err("the packet line is invalid");
        insta::assert_debug_snapshot!(err, "error if data is empty", @r#"
        Custom {
            kind: Other,
            error: Message {
                message: "Empty lines are invalid",
                class: Validation,
            },
        }
        "#);
    }
}

mod text_to_write {
    use bstr::ByteSlice;
    #[cfg(all(feature = "async-io", not(feature = "blocking-io")))]
    use gix_packetline::async_io::encode::text_to_write;
    #[cfg(feature = "blocking-io")]
    use gix_packetline::blocking_io::encode::text_to_write;

    #[crate::bisync::bisync]
    #[cfg_attr(feature = "blocking-io", test)]
    #[cfg_attr(all(feature = "async-io", not(feature = "blocking-io")), async_std::test)]
    async fn always_appends_a_newline() -> gix_testtools::TestResult {
        let mut out = Vec::new();
        let res = text_to_write(b"a", &mut out).await?;
        assert_eq!(res, 6);
        assert_eq!(out.as_bstr(), b"0006a\n".as_bstr());

        out.clear();
        let res = text_to_write(b"a\n", &mut out).await?;
        assert_eq!(res, 7);
        assert_eq!(
            out.as_bstr(),
            b"0007a\n\n".as_bstr(),
            "newline must be appended, as the receiving end is likely to remove it"
        );
        Ok(())
    }
}

mod error {
    use bstr::ByteSlice;
    #[cfg(all(feature = "async-io", not(feature = "blocking-io")))]
    use gix_packetline::async_io::encode::error_to_write;
    #[cfg(feature = "blocking-io")]
    use gix_packetline::blocking_io::encode::error_to_write;

    #[crate::bisync::bisync]
    #[cfg_attr(feature = "blocking-io", test)]
    #[cfg_attr(all(feature = "async-io", not(feature = "blocking-io")), async_std::test)]
    async fn write_line() -> gix_testtools::TestResult {
        let mut out = Vec::new();
        let res = error_to_write(b"hello error", &mut out).await?;
        assert_eq!(res, 19);
        assert_eq!(out.as_bstr(), b"0013ERR hello error".as_bstr());
        Ok(())
    }
}

mod proptests {
    #[cfg(all(feature = "async-io", not(feature = "blocking-io")))]
    use gix_packetline::async_io::encode::data_to_write;
    #[cfg(all(feature = "blocking-io", not(feature = "async-io")))]
    use gix_packetline::blocking_io::encode::data_to_write;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn data_encode_decode_roundtrip(data in proptest::collection::vec(any::<u8>(), 1..=65516)) {
            let mut buf = Vec::new();
            #[cfg(feature = "blocking-io")]
            {
                data_to_write(&data, &mut buf).expect("encoding valid-size data must succeed");
            }
            #[cfg(all(feature = "async-io", not(feature = "blocking-io")))]
            {
                async_std::task::block_on(async {
                    data_to_write(&data, &mut buf).await.expect("encoding valid-size data must succeed");
                });
            }
            let line = gix_packetline::decode::all_at_once(&buf).expect("decoding encoded data must succeed");
            prop_assert_eq!(line.as_slice().expect("data line"), data.as_slice());
        }

        #[test]
        fn streaming_multi_packet_parse(
            items in proptest::collection::vec(
                proptest::collection::vec(any::<u8>(), 1..=100),
                1..=10
            )
        ) {
            // Encode multiple packet lines, concatenate, then parse them all with streaming decoder
            let mut wire = Vec::new();
            for item in &items {
                #[cfg(feature = "blocking-io")]
                {
                    data_to_write(item, &mut wire).expect("encode");
                }
                #[cfg(all(feature = "async-io", not(feature = "blocking-io")))]
                {
                    async_std::task::block_on(async {
                        data_to_write(item, &mut wire).await.expect("encode");
                    });
                }
            }

            let mut remaining = wire.as_slice();
            let mut parsed = Vec::new();
            while !remaining.is_empty() {
                match gix_packetline::decode::streaming(remaining).expect("no error") {
                    gix_packetline::decode::Stream::Complete { line, bytes_consumed } => {
                        parsed.push(line.as_slice().expect("data").to_vec());
                        remaining = &remaining[bytes_consumed..];
                    }
                    gix_packetline::decode::Stream::Incomplete { .. } => break,
                }
            }
            prop_assert_eq!(parsed.len(), items.len(), "must parse all {} items", items.len());
            for (parsed_item, original) in parsed.iter().zip(items.iter()) {
                prop_assert_eq!(parsed_item, original);
            }
        }
    }
}

mod flush_delim_response_end {
    use bstr::ByteSlice;
    #[cfg(all(feature = "async-io", not(feature = "blocking-io")))]
    use gix_packetline::async_io::encode::{delim_to_write, flush_to_write, response_end_to_write};
    #[cfg(feature = "blocking-io")]
    use gix_packetline::blocking_io::encode::{delim_to_write, flush_to_write, response_end_to_write};

    #[crate::bisync::bisync]
    #[cfg_attr(feature = "blocking-io", test)]
    #[cfg_attr(all(feature = "async-io", not(feature = "blocking-io")), async_std::test)]
    async fn success_flush_delim_response_end() -> gix_testtools::TestResult {
        let mut out = Vec::new();
        let res = flush_to_write(&mut out).await?;
        assert_eq!(res, 4);
        assert_eq!(out.as_bstr(), b"0000".as_bstr());

        out.clear();
        let res = delim_to_write(&mut out).await?;
        assert_eq!(res, 4);
        assert_eq!(out.as_bstr(), b"0001".as_bstr());

        out.clear();
        let res = response_end_to_write(&mut out).await?;
        assert_eq!(res, 4);
        assert_eq!(out.as_bstr(), b"0002".as_bstr());
        Ok(())
    }
}
