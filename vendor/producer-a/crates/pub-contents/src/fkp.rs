use pub_core::{RawSpan, StreamPath};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const FKP_PAGE_SIZE: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FkpPropertyKind {
    Character,
    Paragraph,
    Cell,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FkpProperty {
    /// Offset from the beginning of the 512-byte FKP page.
    pub page_offset: u16,
    /// Raw length marker stored at the property block.
    ///
    /// Character and cell properties use a byte count. Paragraph properties
    /// in the verified Win16-era layout use a word count.
    pub length_marker: u8,
    pub payload: Vec<u8>,
    pub source: RawSpan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FkpRun {
    pub fc_first: u32,
    pub fc_lim: u32,
    /// Exact source bytes for the inclusive run start FC.
    pub fc_first_source: RawSpan,
    /// Exact source bytes for the exclusive run limit FC.
    pub fc_lim_source: RawSpan,
    /// Raw one-byte FKP property pointer before multiplication by two.
    pub property_pointer: u8,
    /// Exact source byte for the raw property pointer, including zero/default runs.
    pub property_pointer_source: RawSpan,
    pub property: Option<FkpProperty>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FkpPage {
    pub kind: FkpPropertyKind,
    pub run_count: u8,
    pub runs: Vec<FkpRun>,
    pub source: RawSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FkpReadError {
    WrongPageSize {
        found: usize,
    },
    RunDirectoryOutOfBounds {
        run_count: u8,
    },
    FcNotMonotonic {
        run_index: usize,
        fc_first: u32,
        fc_lim: u32,
    },
    PropertyOffsetOutOfBounds {
        run_index: usize,
        pointer: u8,
        byte_offset: usize,
    },
    PropertyLengthOutOfBounds {
        run_index: usize,
        byte_offset: usize,
        payload_len: usize,
    },
    PropertyOverlapsRunDirectory {
        run_index: usize,
        byte_offset: usize,
        directory_end: usize,
    },
}

impl fmt::Display for FkpReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongPageSize { found } => {
                write!(f, "FKP page must be exactly 512 bytes, found {found}")
            }
            Self::RunDirectoryOutOfBounds { run_count } => write!(
                f,
                "FKP run directory does not fit in one page: crun={run_count}"
            ),
            Self::FcNotMonotonic {
                run_index,
                fc_first,
                fc_lim,
            } => write!(
                f,
                "FKP run {run_index} has non-increasing FC range {fc_first:#x}..{fc_lim:#x}"
            ),
            Self::PropertyOffsetOutOfBounds {
                run_index,
                pointer,
                byte_offset,
            } => write!(
                f,
                "FKP run {run_index} property pointer {pointer:#04x} resolves outside page at {byte_offset}"
            ),
            Self::PropertyLengthOutOfBounds {
                run_index,
                byte_offset,
                payload_len,
            } => write!(
                f,
                "FKP run {run_index} property at {byte_offset} with payload {payload_len} bytes exceeds page"
            ),
            Self::PropertyOverlapsRunDirectory {
                run_index,
                byte_offset,
                directory_end,
            } => write!(
                f,
                "FKP run {run_index} property at {byte_offset} overlaps run directory ending at {directory_end}"
            ),
        }
    }
}

impl std::error::Error for FkpReadError {}

/// Parse one verified 512-byte Win16-era formatting page.
///
/// The physical layout is the Microsoft Word FKP layout also observed in
/// Publisher 2 output produced by the byte-pinned MSWPUB2.CNV oracle:
///
/// - last byte: crun;
/// - front: rgfc[crun + 1] as little-endian u32 file-character offsets;
/// - next crun bytes: property pointers;
/// - each nonzero pointer resolves as page_base + 2 * pointer;
/// - property blocks are packed from the end of the page backwards.
///
/// Character and cell-style property blocks store a byte-count length marker.
/// Paragraph property blocks store a word-count length marker in the verified
/// layout.
pub fn parse_fkp_page(
    stream: StreamPath,
    stream_offset: u64,
    page: &[u8],
    kind: FkpPropertyKind,
) -> Result<FkpPage, FkpReadError> {
    if page.len() != FKP_PAGE_SIZE {
        return Err(FkpReadError::WrongPageSize { found: page.len() });
    }

    let run_count = page[FKP_PAGE_SIZE - 1];
    let run_count_usize = usize::from(run_count);
    let fc_count = run_count_usize + 1;
    let fc_bytes = fc_count
        .checked_mul(4)
        .ok_or(FkpReadError::RunDirectoryOutOfBounds { run_count })?;
    let directory_end = fc_bytes
        .checked_add(run_count_usize)
        .ok_or(FkpReadError::RunDirectoryOutOfBounds { run_count })?;

    if directory_end > FKP_PAGE_SIZE - 1 {
        return Err(FkpReadError::RunDirectoryOutOfBounds { run_count });
    }

    let mut fcs = Vec::with_capacity(fc_count);
    for index in 0..fc_count {
        let offset = index * 4;
        fcs.push(u32::from_le_bytes([
            page[offset],
            page[offset + 1],
            page[offset + 2],
            page[offset + 3],
        ]));
    }

    let mut runs = Vec::with_capacity(run_count_usize);
    for run_index in 0..run_count_usize {
        let fc_first = fcs[run_index];
        let fc_lim = fcs[run_index + 1];
        if fc_first >= fc_lim {
            return Err(FkpReadError::FcNotMonotonic {
                run_index,
                fc_first,
                fc_lim,
            });
        }

        let fc_first_offset = run_index * 4;
        let fc_lim_offset = (run_index + 1) * 4;
        let property_pointer_offset = fc_bytes + run_index;
        let property_pointer = page[property_pointer_offset];
        let property = if property_pointer == 0 {
            None
        } else {
            let byte_offset = usize::from(property_pointer) * 2;
            if byte_offset >= FKP_PAGE_SIZE - 1 {
                return Err(FkpReadError::PropertyOffsetOutOfBounds {
                    run_index,
                    pointer: property_pointer,
                    byte_offset,
                });
            }
            if byte_offset < directory_end {
                return Err(FkpReadError::PropertyOverlapsRunDirectory {
                    run_index,
                    byte_offset,
                    directory_end,
                });
            }

            let length_marker = page[byte_offset];
            let payload_len = match kind {
                FkpPropertyKind::Character | FkpPropertyKind::Cell => usize::from(length_marker),
                FkpPropertyKind::Paragraph => usize::from(length_marker) * 2,
            };
            let payload_start = byte_offset + 1;
            let payload_end = payload_start.checked_add(payload_len).ok_or(
                FkpReadError::PropertyLengthOutOfBounds {
                    run_index,
                    byte_offset,
                    payload_len,
                },
            )?;
            if payload_end > FKP_PAGE_SIZE - 1 {
                return Err(FkpReadError::PropertyLengthOutOfBounds {
                    run_index,
                    byte_offset,
                    payload_len,
                });
            }

            Some(FkpProperty {
                page_offset: u16::try_from(byte_offset)
                    .expect("512-byte FKP offsets always fit in u16"),
                length_marker,
                payload: page[payload_start..payload_end].to_vec(),
                source: RawSpan {
                    stream: stream.clone(),
                    offset: stream_offset + byte_offset as u64,
                    len: (payload_len + 1) as u64,
                },
            })
        };

        runs.push(FkpRun {
            fc_first,
            fc_lim,
            fc_first_source: RawSpan {
                stream: stream.clone(),
                offset: stream_offset + fc_first_offset as u64,
                len: 4,
            },
            fc_lim_source: RawSpan {
                stream: stream.clone(),
                offset: stream_offset + fc_lim_offset as u64,
                len: 4,
            },
            property_pointer,
            property_pointer_source: RawSpan {
                stream: stream.clone(),
                offset: stream_offset + property_pointer_offset as u64,
                len: 1,
            },
            property,
        });
    }

    Ok(FkpPage {
        kind,
        run_count,
        runs,
        source: RawSpan {
            stream,
            offset: stream_offset,
            len: FKP_PAGE_SIZE as u64,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page_with_runs(
        fcs: &[u32],
        pointers: &[u8],
        records: &[(usize, &[u8])],
    ) -> [u8; FKP_PAGE_SIZE] {
        assert_eq!(fcs.len(), pointers.len() + 1);
        let mut page = [0u8; FKP_PAGE_SIZE];
        for (index, fc) in fcs.iter().enumerate() {
            page[index * 4..index * 4 + 4].copy_from_slice(&fc.to_le_bytes());
        }
        let fc_bytes = fcs.len() * 4;
        page[fc_bytes..fc_bytes + pointers.len()].copy_from_slice(pointers);
        for (offset, bytes) in records {
            page[*offset..*offset + bytes.len()].copy_from_slice(bytes);
        }
        page[FKP_PAGE_SIZE - 1] = pointers.len() as u8;
        page
    }

    #[test]
    fn parses_verified_character_fkp_shape() {
        let page = page_with_runs(
            &[0x50, 0x51, 0x52, 0x57],
            &[0xFC, 0xF9, 0xF6],
            &[
                (0x1F8, &[0x05, 0, 0, 0, 0, 0x04]),
                (0x1F2, &[0x05, 1, 0, 0, 0, 0x04]),
                (0x1EC, &[0x05, 0, 0, 0, 0, 0x04]),
            ],
        );

        let parsed = parse_fkp_page(
            StreamPath("/Contents".into()),
            512,
            &page,
            FkpPropertyKind::Character,
        )
        .unwrap();

        assert_eq!(parsed.run_count, 3);
        assert_eq!(
            parsed
                .runs
                .iter()
                .map(|run| (run.fc_first, run.fc_lim))
                .collect::<Vec<_>>(),
            vec![(0x50, 0x51), (0x51, 0x52), (0x52, 0x57)]
        );
        assert_eq!(
            parsed
                .runs
                .iter()
                .map(|run| run.property.as_ref().unwrap().payload[0])
                .collect::<Vec<_>>(),
            vec![0, 1, 0]
        );
        assert_eq!(
            parsed.runs[1].property.as_ref().unwrap().source.offset,
            1010
        );
        assert_eq!(
            parsed.runs[0].fc_first_source,
            RawSpan {
                stream: StreamPath("/Contents".into()),
                offset: 512,
                len: 4,
            }
        );
        assert_eq!(
            parsed.runs[2].fc_lim_source,
            RawSpan {
                stream: StreamPath("/Contents".into()),
                offset: 524,
                len: 4,
            }
        );
        assert_eq!(
            parsed.runs[2].property_pointer_source,
            RawSpan {
                stream: StreamPath("/Contents".into()),
                offset: 530,
                len: 1,
            }
        );
    }

    #[test]
    fn parses_verified_paragraph_fkp_shape() {
        let page = page_with_runs(
            &[0x50, 0x53, 0x56, 0x58],
            &[0x00, 0xFD, 0xFD],
            &[(0x1FA, &[0x02, 0x04, 0x00, 0x01, 0x00])],
        );

        let parsed = parse_fkp_page(
            StreamPath("/Contents".into()),
            1024,
            &page,
            FkpPropertyKind::Paragraph,
        )
        .unwrap();

        assert_eq!(parsed.run_count, 3);
        assert!(parsed.runs[0].property.is_none());
        assert_eq!(
            parsed.runs[0].property_pointer_source,
            RawSpan {
                stream: StreamPath("/Contents".into()),
                offset: 1040,
                len: 1,
            }
        );
        let centered = parsed.runs[1].property.as_ref().unwrap();
        assert_eq!(centered.length_marker, 2);
        assert_eq!(centered.payload, vec![0x04, 0x00, 0x01, 0x00]);
        assert_eq!(centered.source.offset, 1530);
        assert_eq!(
            parsed.runs[2].property.as_ref().unwrap().source,
            centered.source
        );
    }

    #[test]
    fn rejects_property_pointer_into_run_directory() {
        let page = page_with_runs(&[0x50, 0x57], &[0x04], &[]);

        let err = parse_fkp_page(
            StreamPath("/Contents".into()),
            512,
            &page,
            FkpPropertyKind::Character,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            FkpReadError::PropertyOverlapsRunDirectory { .. }
        ));
    }
}
