use crate::DocumentPageList;
use pub_core::{RawSpan, StreamPath};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Один фиксированный u32-overlay внутри уже разобранного DOCUMENT PageList.
///
/// Патч хранит ожидаемое исходное значение, поэтому применение не должно
/// молча работать поверх другого поколения байтов.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentSequenceHandlePatch {
    pub source: RawSpan,
    pub expected_handle: u32,
    pub replacement_handle: u32,
}

/// План перестановки только существующих physical handles.
///
/// Это низкоуровневый primitive для DOCUMENT.field0x02, а не модель
/// видимого порядка страниц. План не классифицирует PAGE/raw0x59/служебные
/// объекты и не создаёт новые handles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentSequencePermutationPlan {
    pub stream: StreamPath,
    pub original_handles: Vec<u32>,
    pub desired_handles: Vec<u32>,
    pub patches: Vec<DocumentSequenceHandlePatch>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentSequencePermutationError {
    LengthMismatch {
        current: usize,
        desired: usize,
    },
    NotPermutation {
        current: Vec<u32>,
        desired: Vec<u32>,
    },
    UnexpectedHandleSpan {
        source: RawSpan,
    },
    MixedStreams {
        expected: StreamPath,
        found: StreamPath,
        offset: u64,
    },
    UnexpectedStream {
        expected: StreamPath,
        found: StreamPath,
    },
    SpanTooLarge {
        source: RawSpan,
    },
    SpanOutOfBounds {
        source: RawSpan,
        stream_len: usize,
    },
    StaleHandle {
        source: RawSpan,
        expected: u32,
        actual: u32,
    },
}

impl fmt::Display for DocumentSequencePermutationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LengthMismatch { current, desired } => write!(
                f,
                "перестановка DOCUMENT PageList должна сохранять число handles: сейчас {current}, запрошено {desired}"
            ),
            Self::NotPermutation { .. } => write!(
                f,
                "новая последовательность DOCUMENT PageList должна быть перестановкой исходного multiset handles"
            ),
            Self::UnexpectedHandleSpan { source } => write!(
                f,
                "handle DOCUMENT PageList должен занимать ровно 4 байта: offset={}, len={}",
                source.offset, source.len
            ),
            Self::MixedStreams {
                expected,
                found,
                offset,
            } => write!(
                f,
                "элемент DOCUMENT PageList с offset={offset} относится к другому потоку: ожидался {}, найден {}",
                expected.0, found.0
            ),
            Self::UnexpectedStream { expected, found } => write!(
                f,
                "план DOCUMENT PageList нельзя применить к другому потоку: ожидался {}, найден {}",
                expected.0, found.0
            ),
            Self::SpanTooLarge { source } => write!(
                f,
                "диапазон handle DOCUMENT PageList не помещается в адресное пространство: offset={}, len={}",
                source.offset, source.len
            ),
            Self::SpanOutOfBounds { source, stream_len } => write!(
                f,
                "диапазон handle DOCUMENT PageList выходит за поток: offset={}, len={}, stream_len={stream_len}",
                source.offset, source.len
            ),
            Self::StaleHandle {
                source,
                expected,
                actual,
            } => write!(
                f,
                "исходный handle DOCUMENT PageList изменился по offset={}: ожидался {expected}, найден {actual}",
                source.offset
            ),
        }
    }
}

impl std::error::Error for DocumentSequencePermutationError {}

/// Строит same-length план перестановки уже существующих handles.
///
/// Функция проверяет не только длину, но и multiset значений. Поэтому через
/// этот API нельзя добавить/удалить handle или заменить его новым ID.
pub fn plan_confirmed_document_sequence_permutation(
    page_list: &DocumentPageList,
    desired_handles: &[u32],
) -> Result<DocumentSequencePermutationPlan, DocumentSequencePermutationError> {
    let original_handles = page_list.handles().collect::<Vec<_>>();

    if original_handles.len() != desired_handles.len() {
        return Err(DocumentSequencePermutationError::LengthMismatch {
            current: original_handles.len(),
            desired: desired_handles.len(),
        });
    }

    if handle_multiset(&original_handles) != handle_multiset(desired_handles) {
        return Err(DocumentSequencePermutationError::NotPermutation {
            current: original_handles,
            desired: desired_handles.to_vec(),
        });
    }

    let stream = page_list.block.source.stream.clone();
    let mut patches = Vec::new();

    for (entry, replacement_handle) in page_list
        .entries
        .iter()
        .zip(desired_handles.iter().copied())
    {
        if entry.handle_source.len != 4 {
            return Err(DocumentSequencePermutationError::UnexpectedHandleSpan {
                source: entry.handle_source.clone(),
            });
        }
        if entry.handle_source.stream != stream {
            return Err(DocumentSequencePermutationError::MixedStreams {
                expected: stream.clone(),
                found: entry.handle_source.stream.clone(),
                offset: entry.handle_source.offset,
            });
        }
        if entry.handle != replacement_handle {
            patches.push(DocumentSequenceHandlePatch {
                source: entry.handle_source.clone(),
                expected_handle: entry.handle,
                replacement_handle,
            });
        }
    }

    Ok(DocumentSequencePermutationPlan {
        stream,
        original_handles,
        desired_handles: desired_handles.to_vec(),
        patches,
    })
}

/// Применяет заранее проверенный план к байтам конкретного Contents-потока.
///
/// Сначала валидируются все ожидаемые исходные u32. Запись начинается только
/// после успешной проверки каждого патча, поэтому stale input не приводит к
/// частично применённой перестановке.
pub fn apply_confirmed_document_sequence_permutation(
    stream: &StreamPath,
    bytes: &mut [u8],
    plan: &DocumentSequencePermutationPlan,
) -> Result<(), DocumentSequencePermutationError> {
    if stream != &plan.stream {
        return Err(DocumentSequencePermutationError::UnexpectedStream {
            expected: plan.stream.clone(),
            found: stream.clone(),
        });
    }

    let mut prepared = Vec::with_capacity(plan.patches.len());

    for patch in &plan.patches {
        if patch.source.stream != *stream {
            return Err(DocumentSequencePermutationError::MixedStreams {
                expected: stream.clone(),
                found: patch.source.stream.clone(),
                offset: patch.source.offset,
            });
        }
        if patch.source.len != 4 {
            return Err(DocumentSequencePermutationError::UnexpectedHandleSpan {
                source: patch.source.clone(),
            });
        }

        let start = usize::try_from(patch.source.offset).map_err(|_| {
            DocumentSequencePermutationError::SpanTooLarge {
                source: patch.source.clone(),
            }
        })?;
        let end =
            start
                .checked_add(4)
                .ok_or_else(|| DocumentSequencePermutationError::SpanTooLarge {
                    source: patch.source.clone(),
                })?;
        let current = bytes.get(start..end).ok_or_else(|| {
            DocumentSequencePermutationError::SpanOutOfBounds {
                source: patch.source.clone(),
                stream_len: bytes.len(),
            }
        })?;
        let actual = u32::from_le_bytes([current[0], current[1], current[2], current[3]]);

        if actual != patch.expected_handle {
            return Err(DocumentSequencePermutationError::StaleHandle {
                source: patch.source.clone(),
                expected: patch.expected_handle,
                actual,
            });
        }

        prepared.push((start, patch.replacement_handle.to_le_bytes()));
    }

    for (start, replacement) in prepared {
        bytes[start..start + 4].copy_from_slice(&replacement);
    }

    Ok(())
}

fn handle_multiset(handles: &[u32]) -> BTreeMap<u32, usize> {
    let mut counts = BTreeMap::new();
    for handle in handles {
        *counts.entry(*handle).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ContentsCursor, parse_confirmed_block, parse_confirmed_document_page_list};

    fn parse_outer(bytes: &[u8]) -> crate::RawContentsBlock {
        let mut cursor = ContentsCursor::new(StreamPath("/Contents".into()), bytes);
        parse_confirmed_block(&mut cursor).expect("outer PageList должен читаться")
    }

    fn encode_page_list(handles: &[u32]) -> Vec<u8> {
        let declared_length = 4 + handles.len() * 6;
        let mut bytes = Vec::with_capacity(2 + declared_length);
        bytes.extend_from_slice(&[0x02, 0xA0]);
        bytes.extend_from_slice(&(declared_length as u32).to_le_bytes());
        for handle in handles {
            bytes.extend_from_slice(&[0x00, 0x70]);
            bytes.extend_from_slice(&handle.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn plans_and_applies_exact_observed_two_handle_swap() {
        let original = [263, 266, 295, 269, 272, 275, 279];
        let desired = [263, 295, 266, 269, 272, 275, 279];
        let mut bytes = encode_page_list(&original);
        let before = bytes.clone();
        let page_list = parse_confirmed_document_page_list(&bytes, parse_outer(&bytes))
            .expect("PageList должен читаться");

        let plan = plan_confirmed_document_sequence_permutation(&page_list, &desired)
            .expect("существующие handles должны переставляться");

        assert_eq!(plan.original_handles, original);
        assert_eq!(plan.desired_handles, desired);
        assert_eq!(plan.patches.len(), 2);
        assert_eq!(plan.patches[0].source.offset, 14);
        assert_eq!(plan.patches[0].expected_handle, 266);
        assert_eq!(plan.patches[0].replacement_handle, 295);
        assert_eq!(plan.patches[1].source.offset, 20);
        assert_eq!(plan.patches[1].expected_handle, 295);
        assert_eq!(plan.patches[1].replacement_handle, 266);

        apply_confirmed_document_sequence_permutation(
            &StreamPath("/Contents".into()),
            &mut bytes,
            &plan,
        )
        .expect("план должен применяться к исходным байтам");

        let reparsed = parse_confirmed_document_page_list(&bytes, parse_outer(&bytes))
            .expect("переставленный PageList должен оставаться валидным");
        assert_eq!(reparsed.handles().collect::<Vec<_>>(), desired);

        let changed_bytes = before
            .iter()
            .zip(&bytes)
            .filter(|(left, right)| left != right)
            .count();
        assert_eq!(
            changed_bytes, 2,
            "контрольный 266<->295 overlay должен менять ровно два байта"
        );
    }

    #[test]
    fn planner_keeps_non_page_handles_in_same_physical_namespace() {
        let bytes = encode_page_list(&[263, 272, 266]);
        let page_list = parse_confirmed_document_page_list(&bytes, parse_outer(&bytes))
            .expect("PageList должен читаться");

        let plan = plan_confirmed_document_sequence_permutation(&page_list, &[263, 266, 272])
            .expect("низкоуровневый primitive не должен фильтровать raw0x59 handle");

        assert_eq!(plan.patches.len(), 2);
    }

    #[test]
    fn planner_rejects_non_permutation() {
        let bytes = encode_page_list(&[263, 266, 295]);
        let page_list = parse_confirmed_document_page_list(&bytes, parse_outer(&bytes))
            .expect("PageList должен читаться");

        let error = plan_confirmed_document_sequence_permutation(&page_list, &[263, 266, 999])
            .expect_err("новый handle нельзя синтезировать через permutation API");

        assert_eq!(
            error,
            DocumentSequencePermutationError::NotPermutation {
                current: vec![263, 266, 295],
                desired: vec![263, 266, 999],
            }
        );
    }

    #[test]
    fn planner_rejects_length_change() {
        let bytes = encode_page_list(&[263, 266, 295]);
        let page_list = parse_confirmed_document_page_list(&bytes, parse_outer(&bytes))
            .expect("PageList должен читаться");

        let error = plan_confirmed_document_sequence_permutation(&page_list, &[263, 266])
            .expect_err("перестановка не должна менять число handles");

        assert_eq!(
            error,
            DocumentSequencePermutationError::LengthMismatch {
                current: 3,
                desired: 2,
            }
        );
    }

    #[test]
    fn duplicate_handles_are_compared_as_multiset() {
        let bytes = encode_page_list(&[263, 266, 266]);
        let page_list = parse_confirmed_document_page_list(&bytes, parse_outer(&bytes))
            .expect("PageList должен читаться");

        let plan = plan_confirmed_document_sequence_permutation(&page_list, &[266, 263, 266])
            .expect("повторяющиеся handles должны поддерживаться как multiset");

        assert_eq!(plan.patches.len(), 2);
    }

    #[test]
    fn apply_is_transactional_on_stale_input() {
        let mut bytes = encode_page_list(&[263, 266, 295]);
        let page_list = parse_confirmed_document_page_list(&bytes, parse_outer(&bytes))
            .expect("PageList должен читаться");
        let plan = plan_confirmed_document_sequence_permutation(&page_list, &[295, 266, 263])
            .expect("план должен строиться");

        let stale_offset = usize::try_from(plan.patches[1].source.offset)
            .expect("тестовый offset должен помещаться");
        bytes[stale_offset..stale_offset + 4].copy_from_slice(&999u32.to_le_bytes());
        let stale_snapshot = bytes.clone();

        let error = apply_confirmed_document_sequence_permutation(
            &StreamPath("/Contents".into()),
            &mut bytes,
            &plan,
        )
        .expect_err("stale source должен отклоняться до первой записи");

        assert!(matches!(
            error,
            DocumentSequencePermutationError::StaleHandle {
                expected: 295,
                actual: 999,
                ..
            }
        ));
        assert_eq!(bytes, stale_snapshot);
    }

    #[test]
    fn apply_rejects_wrong_stream_without_mutation() {
        let mut bytes = encode_page_list(&[263, 266]);
        let page_list = parse_confirmed_document_page_list(&bytes, parse_outer(&bytes))
            .expect("PageList должен читаться");
        let plan = plan_confirmed_document_sequence_permutation(&page_list, &[266, 263])
            .expect("план должен строиться");
        let before = bytes.clone();

        let error = apply_confirmed_document_sequence_permutation(
            &StreamPath("/Other".into()),
            &mut bytes,
            &plan,
        )
        .expect_err("план нельзя применять к другому потоку");

        assert_eq!(
            error,
            DocumentSequencePermutationError::UnexpectedStream {
                expected: StreamPath("/Contents".into()),
                found: StreamPath("/Other".into()),
            }
        );
        assert_eq!(bytes, before);
    }
}
