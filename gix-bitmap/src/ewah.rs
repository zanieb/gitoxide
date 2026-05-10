///
pub mod decode {
    /// The error returned by [`decode()`](super::decode()).
    pub type Error = gix_error::Exn<gix_error::ValidationError>;
}

/// Decode `data` as EWAH bitmap.
pub fn decode(data: &[u8]) -> Result<(Vec, &[u8]), decode::Error> {
    use crate::decode;
    use gix_error::{OptionExt, message};

    let (num_bits, data) = decode::u32(data).ok_or_raise(|| message("eof reading amount of bits").into())?;
    let (len, data) = decode::u32(data).ok_or_raise(|| message("eof reading chunk length").into())?;
    let len = len as usize;

    // NOTE: git does this by copying all bytes first, and then it will change the endianness in a separate loop.
    //       Maybe it's faster, but we can't do it without unsafe. Let's leave it to the optimizer and maybe
    //       one day somebody will find out that it's worth it to use unsafe here.
    let byte_len = len
        .checked_mul(std::mem::size_of::<u64>())
        .ok_or_raise(|| message("EWAH bitmap word length overflows usize").into())?;
    let (mut bits, data) = data
        .split_at_checked(byte_len)
        .ok_or_raise(|| message("eof while reading bit data").into())?;
    let mut buf = std::vec::Vec::<u64>::with_capacity(len);
    for _ in 0..len {
        let (bit_num, rest) = bits.split_at(std::mem::size_of::<u64>());
        bits = rest;
        buf.push(u64::from_be_bytes(bit_num.try_into().unwrap()));
    }

    let (rlw, data) = decode::u32(data).ok_or_raise(|| message("eof while reading run length width").into())?;
    validate_words(&buf, rlw)?;

    Ok((
        Vec {
            num_bits,
            bits: buf,
            rlw: rlw.into(),
        },
        data,
    ))
}

fn validate_words(words: &[u64], rlw: u32) -> Result<(), decode::Error> {
    if words.is_empty() {
        return if rlw == 0 {
            Ok(())
        } else {
            Err(validation_error(
                "EWAH bitmap running length word offset outside word buffer",
            ))
        };
    }
    let rlw = usize::try_from(rlw).expect("u32 fits usize");
    if rlw >= words.len() {
        return Err(validation_error(
            "EWAH bitmap running length word offset outside word buffer",
        ));
    }

    let mut pos = 0usize;
    while pos < words.len() {
        let literal_words = usize::try_from(rlw_literal_words(&words[pos]))
            .map_err(|_| validation_error("EWAH bitmap literal word count does not fit usize"))?;
        pos = pos
            .checked_add(1)
            .and_then(|pos| pos.checked_add(literal_words))
            .ok_or_else(|| validation_error("EWAH bitmap literal word count overflows word buffer"))?;
        if pos > words.len() {
            return Err(validation_error("EWAH bitmap literal word count exceeds word buffer"));
        }
    }

    Ok(())
}

fn validation_error(message: &'static str) -> decode::Error {
    use gix_error::ErrorExt;

    gix_error::ValidationError::from(message).raise()
}

mod write {
    use super::Vec;

    impl Vec {
        /// Write this EWAH bitmap in its on-disk representation.
        pub fn write_to(&self, mut out: impl std::io::Write) -> std::io::Result<()> {
            out.write_all(&self.num_bits.to_be_bytes())?;
            out.write_all(
                &u32::try_from(self.bits.len())
                    .map_err(|_| {
                        std::io::Error::new(std::io::ErrorKind::InvalidInput, "EWAH bitmap has more than 2^32 words")
                    })?
                    .to_be_bytes(),
            )?;
            for word in &self.bits {
                out.write_all(&word.to_be_bytes())?;
            }
            out.write_all(
                &u32::try_from(self.rlw)
                    .map_err(|_| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "EWAH bitmap running length word offset exceeds 2^32",
                        )
                    })?
                    .to_be_bytes(),
            )
        }
    }
}

mod access {
    use super::{rlw_literal_words, Vec, RLW_RUNNING_BITS};

    impl Vec {
        /// Create a bitmap from a sequence of bit values.
        ///
        /// The resulting bitmap uses a literal-only EWAH representation.
        ///
        /// Returns `None` if `bits.len()` exceeds `u32::MAX`.
        pub fn from_bits(bits: &[bool]) -> Option<Self> {
            let literal_words: std::vec::Vec<u64> = bits
                .chunks(64)
                .map(|chunk| {
                    chunk.iter().enumerate().fold(
                        0u64,
                        |word, (idx, bit)| {
                            if *bit { word | (1u64 << idx) } else { word }
                        },
                    )
                })
                .collect();
            let num_bits = bits.len().try_into().ok()?;

            Some(Vec {
                num_bits,
                bits: std::iter::once((literal_words.len() as u64) << (1 + RLW_RUNNING_BITS))
                    .chain(literal_words)
                    .collect(),
                rlw: 0,
            })
        }

        /// Write the bitmap as EWAH bytes to `out`.
        ///
        /// These bytes can be parsed again with [`decode()`](super::decode()).
        pub fn write_to(&self, out: &mut impl std::io::Write) -> std::io::Result<()> {
            let len: u32 = self.bits.len().try_into().map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "bit word count exceeds u32::MAX")
            })?;
            let rlw: u32 = self.rlw.try_into().map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "run length word offset exceeds u32::MAX",
                )
            })?;

            out.write_all(&self.num_bits.to_be_bytes())?;
            out.write_all(&len.to_be_bytes())?;
            for word in &self.bits {
                out.write_all(&word.to_be_bytes())?;
            }
            out.write_all(&rlw.to_be_bytes())
        }

        /// Call `f(index)` for each bit that is true, given the index of the bit that identifies it uniquely within the bit array.
        /// If `f` returns `None` the iteration will be stopped and `None` is returned.
        ///
        /// The index is sequential like in any other vector.
        pub fn for_each_set_bit(&self, mut f: impl FnMut(usize) -> Option<()>) -> Option<()> {
            let num_bits = self.num_bits();
            let mut index = 0usize;
            let mut iter = self.bits.iter();
            while let Some(word) = iter.next() {
                if index >= num_bits {
                    return Some(());
                }

                let running_len = usize::try_from(rlw_running_len_bits(word)).ok()?;
                let run_end = index.checked_add(running_len)?;
                if rlw_runbit_is_set(word) {
                    while index < run_end && index < num_bits {
                        f(index)?;
                        index += 1;
                    }
                } else {
                    index = run_end;
                }

                for _ in 0..rlw_literal_words(word) {
                    if index >= num_bits {
                        return Some(());
                    }
                    let word = iter.next()?;
                    let bits_in_word = num_bits.checked_sub(index)?.min(64);
                    for bit_index in 0..bits_in_word {
                        if word & (1 << bit_index) != 0 {
                            f(index)?;
                        }
                        index += 1;
                    }
                }
            }
            Some(())
        }

        /// The amount of bits we are currently holding.
        pub fn num_bits(&self) -> usize {
            self.num_bits.try_into().expect("we are not on 16 bit systems")
        }
    }

    #[inline]
    fn rlw_running_len_bits(w: &u64) -> u64 {
        rlw_running_len(w) * 64
    }

    #[inline]
    fn rlw_running_len(w: &u64) -> u64 {
        (w >> 1) & RLW_LARGEST_RUNNING_COUNT
    }

    #[inline]
    fn rlw_runbit_is_set(w: &u64) -> bool {
        w & 1 == 1
    }

    const RLW_LARGEST_RUNNING_COUNT: u64 = (1 << RLW_RUNNING_BITS) - 1;
}

const RLW_RUNNING_BITS: u64 = 4 * 8;

#[inline]
fn rlw_literal_words(w: &u64) -> u64 {
    w >> (1 + RLW_RUNNING_BITS)
}

/// A growable collection of u64 that are seen as stream of individual bits.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vec {
    num_bits: u32,
    bits: std::vec::Vec<u64>,
    /// RLW is an offset into the `bits` buffer, so `1` translates into &bits\[1] essentially.
    rlw: u64,
}

#[cfg(test)]
mod tests {
    use super::{decode, Vec};
    use std::vec::Vec as StdVec;

    #[test]
    fn write_to_produces_the_on_disk_representation() {
        let bitmap = Vec {
            num_bits: 128,
            bits: vec![0x0000_0004_0000_0000, 0x8000_0000_0000_0001, 0x0000_0000_0000_0002],
            rlw: 0,
        };
        let mut out = StdVec::new();
        bitmap.write_to(&mut out).unwrap();

        assert_eq!(
            out,
            [
                0x00, 0x00, 0x00, 0x80, // bit count
                0x00, 0x00, 0x00, 0x03, // word count
                0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, // RLW
                0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, // literal word 1
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, // literal word 2
                0x00, 0x00, 0x00, 0x00, // RLW offset
            ]
        );
    }

    #[test]
    fn decoded_bitmaps_roundtrip_through_write_to() {
        let input = [
            0x00, 0x00, 0x00, 0x80, // bit count
            0x00, 0x00, 0x00, 0x03, // word count
            0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, // RLW
            0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, // literal word 1
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, // literal word 2
            0x00, 0x00, 0x00, 0x00, // RLW offset
            0xaa, 0xbb,
        ];
        let (bitmap, rest) = decode(&input).unwrap();
        assert_eq!(rest, [0xaa, 0xbb]);
        assert_eq!(bitmap.num_bits(), 128);

        let mut set_bits = StdVec::new();
        bitmap
            .for_each_set_bit(|index| {
                set_bits.push(index);
                Some(())
            })
            .unwrap();
        assert_eq!(set_bits, [0, 63, 65]);

        let mut out = StdVec::new();
        bitmap.write_to(&mut out).unwrap();
        assert_eq!(out, input[..input.len() - rest.len()]);
    }

    #[test]
    fn decode_rejects_literal_count_past_word_buffer() {
        let input = [
            0x00, 0x00, 0x00, 0x80, // bit count
            0x00, 0x00, 0x00, 0x01, // word count
            0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, // RLW declares one literal word
            0x00, 0x00, 0x00, 0x00, // RLW offset
        ];

        let err = decode(&input).unwrap_err();
        assert!(
            err.to_string().contains("literal word count exceeds word buffer"),
            "{err}"
        );
    }

    #[test]
    fn decode_rejects_rlw_offset_outside_word_buffer() {
        let input = [
            0x00, 0x00, 0x00, 0x00, // bit count
            0x00, 0x00, 0x00, 0x01, // word count
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // RLW
            0x00, 0x00, 0x00, 0x01, // invalid RLW offset
        ];

        let err = decode(&input).unwrap_err();
        assert!(
            err.to_string()
                .contains("running length word offset outside word buffer"),
            "{err}"
        );
    }

    #[test]
    fn set_bit_iteration_ignores_literal_padding_beyond_declared_bits() {
        let bitmap = Vec {
            num_bits: 2,
            bits: vec![1 << 33, u64::MAX],
            rlw: 0,
        };

        let mut set_bits = StdVec::new();
        bitmap
            .for_each_set_bit(|index| {
                set_bits.push(index);
                Some(())
            })
            .unwrap();

        assert_eq!(set_bits, [0, 1]);
    }

    #[test]
    fn set_bit_iteration_ignores_run_padding_beyond_declared_bits() {
        let bitmap = Vec {
            num_bits: 3,
            bits: vec![0b11],
            rlw: 0,
        };

        let mut set_bits = StdVec::new();
        bitmap
            .for_each_set_bit(|index| {
                set_bits.push(index);
                Some(())
            })
            .unwrap();

        assert_eq!(set_bits, [0, 1, 2]);
    }
}
