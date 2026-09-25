use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Логический путь потока внутри контейнера публикации.
///
/// Тип намеренно не называется `StreamId`: в MS-CFB термин «stream ID»
/// обозначает числовой идентификатор записи каталога. Путь — это адрес
/// на уровне адаптера контейнера, а не собственная идентичность объекта Publisher.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct StreamPath(pub String);

/// Явный objectId из старого семейства Contents 0x22.
///
/// Для проверенных Publisher 98/2000 каталог хранит этот идентификатор как
/// отдельное 16-битное значение. Этот тип нельзя использовать как seqNum
/// семейства 0x2C.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Contents0x22ObjectId(pub u16);

/// Один идентификатор из массива Quill SYID.
///
/// В проверенной грамматике SYID элементы массива хранятся как 32-битные
/// значения. Семантические связи с Contents должны добавляться отдельно и
/// только для подтверждённой области действия.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct QuillSyid(pub u32);

/// Поздний Publisher Oid в подтверждённом 8-байтном представлении.
///
/// Oid является отдельным пространством идентичности. Его нельзя приравнивать
/// к физическому object handle / seqNum, и одинаковый Oid может встречаться у
/// разных физических объектов сценария.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PublisherOid(pub [u8; 8]);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawSpan {
    pub stream: StreamPath,
    pub offset: u64,
    pub len: u64,
}

impl RawSpan {
    pub fn end(&self) -> Option<u64> {
        self.offset.checked_add(self.len)
    }
}

/// Неизменяемые исходные байты потоков для происхождения данных и
/// восстановления без потерь.
///
/// Парсеры могут декодировать из этих байтов структуры более высокого уровня,
/// но `RawSpan` имеет смысл только пока сохранён соответствующий
/// `RawPublication`.
#[derive(Debug, Clone, Default)]
pub struct RawPublication {
    streams: BTreeMap<StreamPath, Arc<[u8]>>,
}

impl RawPublication {
    pub fn from_streams(streams: impl IntoIterator<Item = (StreamPath, Vec<u8>)>) -> Self {
        let streams = streams
            .into_iter()
            .map(|(path, bytes)| (path, Arc::<[u8]>::from(bytes)))
            .collect();

        Self { streams }
    }

    pub fn stream(&self, path: &StreamPath) -> Option<&[u8]> {
        self.streams.get(path).map(AsRef::as_ref)
    }

    pub fn bytes(&self, span: &RawSpan) -> Option<&[u8]> {
        let end = span.end()?;
        let start = usize::try_from(span.offset).ok()?;
        let end = usize::try_from(end).ok()?;
        self.stream(&span.stream)?.get(start..end)
    }
}

/// Значение, декодированное из одного непрерывного участка исходных байтов.
///
/// Производные семантические значения, которые зависят от нескольких записей
/// или проекций, должны хранить происхождение явно, а не через этот вспомогательный тип.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decoded<T> {
    pub value: T,
    pub source: RawSpan,
    pub raw: Vec<u8>,
}

/// Состояние декодирования одного сырого элемента.
///
/// Тип неизвестного признака сделан обобщённым намеренно: разные части PUB
/// используют разные пространства ключей и разную ширину идентификаторов.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParseState<T, I> {
    Known(Decoded<T>),
    Unknown { tag: I, source: RawSpan },
    Malformed { source: RawSpan, message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_span_end_is_checked() {
        let span = RawSpan {
            stream: StreamPath("/Contents".into()),
            offset: 10,
            len: 5,
        };
        assert_eq!(span.end(), Some(15));
    }

    #[test]
    fn raw_publication_resolves_only_in_bounds() {
        let path = StreamPath("/Contents".into());
        let raw = RawPublication::from_streams([(path.clone(), vec![0x10, 0x20, 0x30, 0x40])]);

        let valid = RawSpan {
            stream: path.clone(),
            offset: 1,
            len: 2,
        };
        assert_eq!(raw.bytes(&valid), Some(&[0x20, 0x30][..]));

        let out_of_bounds = RawSpan {
            stream: path,
            offset: 3,
            len: 2,
        };
        assert_eq!(raw.bytes(&out_of_bounds), None);
    }

    #[test]
    fn identity_namespaces_remain_distinct_types() {
        let old_id = Contents0x22ObjectId(7);
        let syid = QuillSyid(7);
        let oid = PublisherOid([0; 8]);

        assert_eq!(old_id.0, 7);
        assert_eq!(syid.0, 7);
        assert_eq!(oid.0, [0; 8]);
    }
}
