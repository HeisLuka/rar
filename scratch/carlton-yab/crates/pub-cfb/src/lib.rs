use anyhow::{Context, Result};
use serde::Serialize;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;
use uuid::Uuid;

pub const CFB_INVENTORY_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CfbEntry {
    pub path: String,
    pub name: String,
    pub kind: EntryKind,
    pub len: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Root,
    Storage,
    Stream,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CfbInventory {
    pub schema_version: u32,
    pub entries: Vec<CfbEntry>,
}

pub fn inspect_path(path: impl AsRef<Path>) -> Result<CfbInventory> {
    let path = path.as_ref();
    let file =
        File::open(path).with_context(|| format!("не удалось открыть файл {}", path.display()))?;

    inspect_reader(file)
        .with_context(|| format!("не удалось разобрать CFB-файл {}", path.display()))
}

pub fn inspect_reader<R: Read + Seek>(reader: R) -> Result<CfbInventory> {
    let compound = cfb::CompoundFile::open(reader).context("не удалось разобрать CFB-контейнер")?;

    let mut entries: Vec<_> = compound
        .walk()
        .map(|entry| CfbEntry {
            path: canonical_cfb_path(entry.path()),
            name: entry.name().to_owned(),
            kind: if entry.is_root() {
                EntryKind::Root
            } else if entry.is_stream() {
                EntryKind::Stream
            } else {
                EntryKind::Storage
            },
            len: entry.len(),
        })
        .collect();

    entries.sort_by(|left, right| left.path.cmp(&right.path));

    Ok(CfbInventory {
        schema_version: CFB_INVENTORY_SCHEMA_VERSION,
        entries,
    })
}

/// Читает один точный stream из CFB по логическому пути.
///
/// Функция ничего не знает о семантике Publisher и не нормализует имя потока.
/// Она нужна projection-парсерам, которым требуются исходные байты вместе с
/// отдельно отслеживаемым `StreamPath`.
pub fn read_stream_path(path: impl AsRef<Path>, stream_path: &str) -> Result<Vec<u8>> {
    let path = path.as_ref();
    let file =
        File::open(path).with_context(|| format!("не удалось открыть файл {}", path.display()))?;

    read_stream_reader(file, stream_path).with_context(|| {
        format!(
            "не удалось прочитать поток {stream_path} из CFB-файла {}",
            path.display()
        )
    })
}

pub fn read_stream_reader<R: Read + Seek>(reader: R, stream_path: &str) -> Result<Vec<u8>> {
    let mut compound =
        cfb::CompoundFile::open(reader).context("не удалось разобрать CFB-контейнер")?;
    let mut stream = compound
        .open_stream(stream_path)
        .with_context(|| format!("в CFB отсутствует поток {stream_path}"))?;

    let mut bytes = Vec::new();
    stream
        .read_to_end(&mut bytes)
        .with_context(|| format!("не удалось прочитать поток {stream_path}"))?;
    Ok(bytes)
}

#[derive(Debug, Clone)]
struct CfbRewriteEntry {
    path: PathBuf,
    kind: EntryKind,
    clsid: Uuid,
    state_bits: u32,
    created: SystemTime,
    modified: SystemTime,
    bytes: Option<Vec<u8>>,
}

/// Создаёт копию исходного CFB и заменяет в ней ровно один существующий stream.
///
/// В отличие от fresh repack, функция открывает byte-for-byte копию исходного
/// контейнера в read-write режиме. Поэтому source directory topology и прочее
/// неэкспонированное CFB-состояние не пересоздаётся без необходимости.
///
/// Все untouched logical stream payloads должны остаться byte-for-byte равны
/// source. Exposed entry metadata также проверяется после mutation.
///
/// Функция не утверждает, что конкретное приложение примет изменённый CFB.
/// Это preservation-first container primitive; native application validation
/// остаётся обязанностью вызывающего writer.
pub fn replace_stream_reader<R: Read + Seek>(
    mut reader: R,
    stream_path: &str,
    replacement: &[u8],
) -> Result<Vec<u8>> {
    reader
        .seek(SeekFrom::Start(0))
        .context("не удалось перейти к началу исходного CFB")?;
    let mut source_bytes = Vec::new();
    reader
        .read_to_end(&mut source_bytes)
        .context("не удалось прочитать исходные CFB-байты")?;

    let mut source = cfb::CompoundFile::open(std::io::Cursor::new(source_bytes.as_slice()))
        .context("не удалось разобрать исходный CFB-контейнер")?;
    let source_version = source.version();

    let mut entries = source
        .walk()
        .map(|entry| CfbRewriteEntry {
            path: entry.path().to_path_buf(),
            kind: if entry.is_root() {
                EntryKind::Root
            } else if entry.is_stream() {
                EntryKind::Stream
            } else {
                EntryKind::Storage
            },
            clsid: entry.clsid().to_owned(),
            state_bits: entry.state_bits(),
            created: entry.created(),
            modified: entry.modified(),
            bytes: None,
        })
        .collect::<Vec<_>>();

    let target_path = Path::new(stream_path);
    let target = entries
        .iter()
        .find(|entry| entry.path == target_path)
        .with_context(|| format!("в CFB отсутствует target stream {stream_path}"))?;
    if target.kind != EntryKind::Stream {
        anyhow::bail!("target path {stream_path} не является CFB stream");
    }

    for entry in &mut entries {
        if entry.kind != EntryKind::Stream {
            continue;
        }
        let mut stream = source.open_stream(&entry.path).with_context(|| {
            format!("не удалось открыть source stream {}", entry.path.display())
        })?;
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).with_context(|| {
            format!(
                "не удалось прочитать source stream {}",
                entry.path.display()
            )
        })?;
        entry.bytes = Some(bytes);
    }
    drop(source);

    let mut output = cfb::CompoundFile::open(std::io::Cursor::new(source_bytes))
        .context("не удалось открыть копию CFB в read-write режиме")?;
    {
        let mut stream = output.open_stream(target_path).with_context(|| {
            format!("не удалось открыть target stream {stream_path} для записи")
        })?;
        let replacement_len = u64::try_from(replacement.len())
            .context("размер replacement stream не помещается в u64")?;
        stream
            .set_len(replacement_len)
            .with_context(|| format!("не удалось изменить размер target stream {stream_path}"))?;
        stream
            .seek(SeekFrom::Start(0))
            .with_context(|| format!("не удалось перейти к началу target stream {stream_path}"))?;
        stream
            .write_all(replacement)
            .with_context(|| format!("не удалось записать target stream {stream_path}"))?;
        stream
            .flush()
            .with_context(|| format!("не удалось flush target stream {stream_path}"))?;
    }
    output.flush().context("не удалось flush output CFB")?;

    let bytes = output.into_inner().into_inner();
    validate_rewritten_cfb(&entries, target_path, replacement, source_version, &bytes)?;
    Ok(bytes)
}

fn validate_rewritten_cfb(
    source_entries: &[CfbRewriteEntry],
    target_path: &Path,
    replacement: &[u8],
    source_version: cfb::Version,
    output_bytes: &[u8],
) -> Result<()> {
    let mut output = cfb::CompoundFile::open(std::io::Cursor::new(output_bytes))
        .context("изменённый CFB не открывается")?;
    if output.version() != source_version {
        anyhow::bail!(
            "CFB version changed during stream mutation: source={source_version:?}, output={:?}",
            output.version()
        );
    }
    let output_entries = output
        .walk()
        .map(|entry| {
            let kind = if entry.is_root() {
                EntryKind::Root
            } else if entry.is_stream() {
                EntryKind::Stream
            } else {
                EntryKind::Storage
            };
            (
                entry.path().to_path_buf(),
                (
                    kind,
                    entry.clsid().to_owned(),
                    entry.state_bits(),
                    entry.created(),
                    entry.modified(),
                ),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();

    if output_entries.len() != source_entries.len() {
        anyhow::bail!(
            "CFB entry count changed during rewrite: source={}, output={}",
            source_entries.len(),
            output_entries.len()
        );
    }

    for source in source_entries {
        let (kind, clsid, state_bits, created, modified) = output_entries
            .get(&source.path)
            .with_context(|| format!("output CFB потерял entry {}", source.path.display()))?;
        if *kind != source.kind {
            anyhow::bail!("CFB entry kind changed for {}", source.path.display());
        }
        if *state_bits != source.state_bits {
            anyhow::bail!("CFB state bits changed for {}", source.path.display());
        }
        if matches!(source.kind, EntryKind::Root | EntryKind::Storage) {
            if clsid != &source.clsid {
                anyhow::bail!("CFB CLSID changed for {}", source.path.display());
            }
            if created != &source.created || modified != &source.modified {
                anyhow::bail!(
                    "CFB storage timestamps changed for {}",
                    source.path.display()
                );
            }
        }

        if source.kind == EntryKind::Stream {
            let mut stream = output.open_stream(&source.path).with_context(|| {
                format!(
                    "не удалось повторно открыть stream {}",
                    source.path.display()
                )
            })?;
            let mut actual = Vec::new();
            stream.read_to_end(&mut actual).with_context(|| {
                format!(
                    "не удалось повторно прочитать stream {}",
                    source.path.display()
                )
            })?;
            let expected = if source.path == target_path {
                replacement
            } else {
                source
                    .bytes
                    .as_deref()
                    .expect("source stream bytes were collected")
            };
            if actual != expected {
                anyhow::bail!(
                    "logical stream payload changed unexpectedly for {}",
                    source.path.display()
                );
            }
        }
    }

    Ok(())
}

fn canonical_cfb_path(path: &Path) -> String {
    let names: Vec<_> = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_string_lossy()),
            _ => None,
        })
        .collect();

    if names.is_empty() {
        "/".to_owned()
    } else {
        format!("/{}", names.join("/"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use std::time::{Duration, UNIX_EPOCH};

    fn synthetic_cfb() -> Cursor<Vec<u8>> {
        let mut compound = cfb::CompoundFile::create(Cursor::new(Vec::new()))
            .expect("синтетический CFB должен создаваться");

        compound
            .create_storage("/Zoo")
            .expect("хранилище Zoo должно создаваться");
        compound
            .create_storage("/Alpha")
            .expect("хранилище Alpha должно создаваться");

        compound
            .create_stream("/Zoo/last")
            .expect("поток Zoo/last должен создаваться")
            .write_all(b"1234")
            .expect("данные Zoo/last должны записываться");

        compound
            .create_stream("/Alpha/first")
            .expect("поток Alpha/first должен создаваться")
            .write_all(b"12")
            .expect("данные Alpha/first должны записываться");

        compound
            .set_storage_clsid(
                "/Alpha",
                Uuid::parse_str("12345678-1234-5678-9abc-def012345678").unwrap(),
            )
            .expect("CLSID Alpha должен устанавливаться");
        compound
            .set_state_bits("/Alpha", 0x1234_5678)
            .expect("state bits Alpha должны устанавливаться");
        compound
            .set_state_bits("/Zoo/last", 0x55aa_00ff)
            .expect("state bits stream должны устанавливаться");
        compound
            .set_created_time("/Alpha", UNIX_EPOCH + Duration::from_secs(1_000))
            .expect("created time Alpha должен устанавливаться");
        compound
            .set_modified_time("/Alpha", UNIX_EPOCH + Duration::from_secs(2_000))
            .expect("modified time Alpha должен устанавливаться");

        compound
            .flush()
            .expect("синтетический CFB должен сбрасываться в память");

        let mut cursor = compound.into_inner();
        cursor.set_position(0);
        cursor
    }

    #[test]
    fn inventory_is_sorted_by_canonical_path() {
        let inventory = inspect_reader(synthetic_cfb()).expect("CFB должен разбираться");

        let paths: Vec<_> = inventory
            .entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect();

        assert_eq!(
            paths,
            vec!["/", "/Alpha", "/Alpha/first", "/Zoo", "/Zoo/last"]
        );
        assert_eq!(inventory.schema_version, CFB_INVENTORY_SCHEMA_VERSION);
    }

    #[test]
    fn inventory_preserves_kind_and_stream_length() {
        let inventory = inspect_reader(synthetic_cfb()).expect("CFB должен разбираться");

        let first = inventory
            .entries
            .iter()
            .find(|entry| entry.path == "/Alpha/first")
            .expect("поток Alpha/first должен присутствовать");

        assert_eq!(first.kind, EntryKind::Stream);
        assert_eq!(first.len, 2);

        let alpha = inventory
            .entries
            .iter()
            .find(|entry| entry.path == "/Alpha")
            .expect("хранилище Alpha должно присутствовать");

        assert_eq!(alpha.kind, EntryKind::Storage);
        assert_eq!(alpha.len, 0);
    }

    #[test]
    fn reads_exact_stream_bytes() {
        let bytes =
            read_stream_reader(synthetic_cfb(), "/Alpha/first").expect("поток должен читаться");
        assert_eq!(bytes, b"12");
    }

    #[test]
    fn missing_stream_is_explicit_error() {
        let error = read_stream_reader(synthetic_cfb(), "/Missing")
            .expect_err("несуществующий stream должен быть ошибкой");
        assert!(error.to_string().contains("/Missing"));
    }

    #[test]
    fn in_copy_mutation_replaces_one_stream_and_preserves_logical_state() {
        let source = synthetic_cfb().into_inner();
        let rewritten =
            replace_stream_reader(Cursor::new(source.clone()), "/Alpha/first", b"replacement")
                .expect("bounded CFB in-copy mutation должна пройти");

        assert_eq!(
            read_stream_reader(Cursor::new(rewritten.clone()), "/Alpha/first")
                .expect("target stream должен читаться"),
            b"replacement"
        );
        assert_eq!(
            read_stream_reader(Cursor::new(rewritten.clone()), "/Zoo/last")
                .expect("untouched stream должен читаться"),
            b"1234"
        );

        let original =
            cfb::CompoundFile::open(Cursor::new(source)).expect("source CFB должен открываться");
        let output =
            cfb::CompoundFile::open(Cursor::new(rewritten)).expect("output CFB должен открываться");
        assert_eq!(original.version(), output.version());

        for path in ["/Alpha", "/Zoo/last"] {
            let before = original.entry(path).expect("source entry");
            let after = output.entry(path).expect("output entry");
            assert_eq!(before.clsid(), after.clsid());
            assert_eq!(before.state_bits(), after.state_bits());
            if before.is_storage() {
                assert_eq!(before.created(), after.created());
                assert_eq!(before.modified(), after.modified());
            }
        }
    }

    #[test]
    fn in_copy_mutation_rejects_missing_or_non_stream_target() {
        let source = synthetic_cfb().into_inner();

        let missing = replace_stream_reader(Cursor::new(source.clone()), "/Missing", b"x")
            .expect_err("missing stream должен блокировать rewrite");
        assert!(missing.to_string().contains("/Missing"));

        let storage = replace_stream_reader(Cursor::new(source), "/Alpha", b"x")
            .expect_err("storage path не должен приниматься как stream");
        assert!(storage.to_string().contains("не является CFB stream"));
    }

    #[test]
    fn invalid_input_returns_cfb_error() {
        let error = inspect_reader(Cursor::new(b"not a cfb".to_vec()))
            .expect_err("произвольные байты не должны считаться CFB");

        assert!(
            error
                .to_string()
                .contains("не удалось разобрать CFB-контейнер"),
            "неожиданная ошибка: {error:#}"
        );
    }
}
