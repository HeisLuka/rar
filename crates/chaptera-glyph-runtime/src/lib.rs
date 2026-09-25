use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const GLYPH_RUNTIME_SCHEMA_V1: &str = "chaptera.glyph-runtime.v1";
pub const GLYPH_RUNTIME_BENCHMARK_SCHEMA_V1: &str = "chaptera.glyph-runtime-benchmark.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderGlyphV1 {
    pub glyph_id: u32,
    pub x_emu: i64,
    pub y_emu: i64,
    pub advance_emu: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderGlyphRunV1 {
    pub page_id: String,
    pub story_id: String,
    pub frame_node_id: String,
    pub scalar_start: u32,
    pub scalar_end: u32,
    pub font_resource_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paint_id: Option<String>,
    pub glyphs: Vec<RenderGlyphV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderFontResourceV1 {
    pub resource_id: String,
    pub kind: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GlyphRenderMode {
    Raster,
    Vector,
    ColorComplex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GlyphQuality {
    Normal,
    High,
    Fidelity,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GlyphCacheKey {
    pub font_fingerprint: String,
    pub face_index: u32,
    pub glyph_id: u32,
    pub render_mode: GlyphRenderMode,
    pub scale_bucket: u16,
    pub quality: GlyphQuality,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FontRuntimeState {
    Pending,
    Ready,
    Blocked,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlyphBinding {
    pub key: GlyphCacheKey,
    pub residency_generation: u64,
    pub font_generation: u64,
    pub device_generation: u64,
    pub atlas_generation: u64,
    pub atlas_page: u32,
    pub atlas_slot: u32,
    pub rasterizer_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GlyphRequestResult {
    Ready { binding: GlyphBinding },
    Pending { key: GlyphCacheKey },
    Blocked { key: GlyphCacheKey },
    Failed { key: GlyphCacheKey },
    Unsupported { key: GlyphCacheKey, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedGlyphRun {
    pub geometry: Vec<RenderGlyphV1>,
    pub materials: Vec<GlyphRequestResult>,
    pub render_mode: GlyphRenderMode,
    pub scale_bucket: u16,
    pub quality: GlyphQuality,
    pub canonical_geometry_mutated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialPolicy {
    pub render_mode: GlyphRenderMode,
    pub scale_bucket: u16,
    pub quality: GlyphQuality,
}

pub fn select_material_policy(zoom: f64, dpr: f64) -> Result<MaterialPolicy, GlyphRuntimeError> {
    let scale = zoom * dpr;
    if !scale.is_finite() || scale <= 0.0 {
        return Err(GlyphRuntimeError::InvalidScale { zoom, dpr });
    }
    let policy = if scale <= 1.0 {
        MaterialPolicy { render_mode: GlyphRenderMode::Raster, scale_bucket: 1, quality: GlyphQuality::Normal }
    } else if scale <= 2.0 {
        MaterialPolicy { render_mode: GlyphRenderMode::Raster, scale_bucket: 2, quality: GlyphQuality::Normal }
    } else if scale <= 4.0 {
        MaterialPolicy { render_mode: GlyphRenderMode::Raster, scale_bucket: 4, quality: GlyphQuality::High }
    } else if scale <= 8.0 {
        MaterialPolicy { render_mode: GlyphRenderMode::Raster, scale_bucket: 8, quality: GlyphQuality::High }
    } else {
        MaterialPolicy { render_mode: GlyphRenderMode::Vector, scale_bucket: 0, quality: GlyphQuality::Fidelity }
    };
    Ok(policy)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaterializeError {
    Unsupported(String),
    Failed(String),
}

pub trait GlyphMaterializer {
    fn materialize(&mut self, key: &GlyphCacheKey) -> Result<Vec<u8>, MaterializeError>;
}

#[derive(Debug, Clone)]
struct GlyphEntry {
    material_bytes: Vec<u8>,
    residency_generation: u64,
    font_generation: u64,
    last_touch: u64,
    atlas_page: Option<u32>,
    atlas_slot: Option<u32>,
}

#[derive(Debug, Clone)]
struct FontState {
    state: FontRuntimeState,
    generation: u64,
}

#[derive(Debug, Clone)]
struct Atlas {
    page_capacity: u32,
    max_pages: u32,
    generation: u64,
    placements: BTreeMap<GlyphCacheKey, (u32, u32)>,
}

impl Atlas {
    fn new(page_capacity: u32, max_pages: u32) -> Result<Self, GlyphRuntimeError> {
        if page_capacity == 0 || max_pages == 0 {
            return Err(GlyphRuntimeError::InvalidAtlasBounds { page_capacity, max_pages });
        }
        Ok(Self {
            page_capacity,
            max_pages,
            generation: 1,
            placements: BTreeMap::new(),
        })
    }

    fn capacity(&self) -> usize {
        self.page_capacity as usize * self.max_pages as usize
    }

    fn allocate(&mut self, key: &GlyphCacheKey) -> Option<(u32, u32)> {
        if let Some(place) = self.placements.get(key) {
            return Some(*place);
        }
        if self.placements.len() >= self.capacity() {
            return None;
        }
        let used: BTreeSet<_> = self.placements.values().copied().collect();
        for ordinal in 0..self.capacity() {
            let place = (
                u32::try_from(ordinal / self.page_capacity as usize).ok()?,
                u32::try_from(ordinal % self.page_capacity as usize).ok()?,
            );
            if !used.contains(&place) {
                self.placements.insert(key.clone(), place);
                return Some(place);
            }
        }
        None
    }

    fn release(&mut self, key: &GlyphCacheKey) {
        self.placements.remove(key);
    }

    fn repack(&mut self) -> usize {
        let old = self.placements.clone();
        let keys = old.keys().cloned().collect::<Vec<_>>();
        self.placements.clear();
        for (index, key) in keys.into_iter().enumerate() {
            let page = (index / self.page_capacity as usize) as u32;
            let slot = (index % self.page_capacity as usize) as u32;
            self.placements.insert(key, (page, slot));
        }
        self.generation += 1;
        old.iter()
            .filter(|(key, place)| self.placements.get(*key) != Some(*place))
            .count()
    }

    fn occupancy(&self) -> AtlasOccupancy {
        if self.placements.is_empty() {
            return AtlasOccupancy {
                pages: 0,
                live_slots: 0,
                capacity_slots: 0,
                free_slots: 0,
                fragmentation_ratio_ppm: 0,
            };
        }
        let pages = self.placements.values().map(|(page, _)| *page).max().unwrap_or(0) + 1;
        let capacity_slots = pages * self.page_capacity;
        let live_slots = self.placements.len() as u32;
        let free_slots = capacity_slots.saturating_sub(live_slots);
        let fragmentation_ratio_ppm = if capacity_slots == 0 {
            0
        } else {
            u32::try_from((u64::from(free_slots) * 1_000_000) / u64::from(capacity_slots))
                .unwrap_or(u32::MAX)
        };
        AtlasOccupancy {
            pages,
            live_slots,
            capacity_slots,
            free_slots,
            fragmentation_ratio_ppm,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AtlasOccupancy {
    pub pages: u32,
    pub live_slots: u32,
    pub capacity_slots: u32,
    pub free_slots: u32,
    pub fragmentation_ratio_ppm: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlyphRuntimeMetrics {
    pub requests: u64,
    pub hits: u64,
    pub misses: u64,
    pub pending: u64,
    pub blocked: u64,
    pub failed: u64,
    pub unsupported: u64,
    pub evictions: u64,
    pub automatic_evictions: u64,
    pub repacks: u64,
    pub device_resets: u64,
    pub font_generation_invalidations: u64,
    pub stale_binding_rejections: u64,
    pub upload_bytes: u64,
    pub materialize_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlyphRuntimeReceipt {
    pub schema: String,
    pub device_generation: u64,
    pub atlas_generation: u64,
    pub unique_logical_entries: usize,
    pub resident_entries: usize,
    pub resident_material_bytes: u64,
    pub atlas: AtlasOccupancy,
    pub metrics: GlyphRuntimeMetrics,
    pub hit_ratio_ppm: u32,
    pub authority: GlyphRuntimeAuthority,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlyphRuntimeAuthority {
    pub reshapes_text: bool,
    pub discovers_host_fonts: bool,
    pub mutates_canonical_glyph_positions: bool,
    pub atlas_slot_is_semantic_identity: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GlyphRuntimeError {
    InvalidAtlasBounds { page_capacity: u32, max_pages: u32 },
    InvalidScale { zoom: f64, dpr: f64 },
    InvalidFontResourceKind { found: String },
    MissingFontFingerprint,
    FontResourceMismatch { expected: String, found: String },
    InvalidFingerprint,
    AtlasCapacityUnavailable,
}

impl fmt::Display for GlyphRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for GlyphRuntimeError {}

pub struct GlyphRuntime<M: GlyphMaterializer> {
    materializer: M,
    atlas: Atlas,
    entries: BTreeMap<GlyphCacheKey, GlyphEntry>,
    fonts: BTreeMap<String, FontState>,
    device_generation: u64,
    touch_seq: u64,
    rasterizer_version: u32,
    metrics: GlyphRuntimeMetrics,
}

impl<M: GlyphMaterializer> GlyphRuntime<M> {
    pub fn new(
        materializer: M,
        page_capacity: u32,
        max_pages: u32,
        rasterizer_version: u32,
    ) -> Result<Self, GlyphRuntimeError> {
        Ok(Self {
            materializer,
            atlas: Atlas::new(page_capacity, max_pages)?,
            entries: BTreeMap::new(),
            fonts: BTreeMap::new(),
            device_generation: 1,
            touch_seq: 0,
            rasterizer_version,
            metrics: GlyphRuntimeMetrics::default(),
        })
    }

    pub fn materializer(&self) -> &M {
        &self.materializer
    }

    pub fn materializer_mut(&mut self) -> &mut M {
        &mut self.materializer
    }

    pub fn set_font_state(
        &mut self,
        fingerprint: impl Into<String>,
        state: FontRuntimeState,
        generation: u64,
    ) -> Result<(), GlyphRuntimeError> {
        let fingerprint = fingerprint.into();
        validate_fingerprint(&fingerprint)?;
        let previous = self.fonts.insert(
            fingerprint.clone(),
            FontState { state, generation },
        );
        if previous.as_ref().is_some_and(|old| old.generation != generation) {
            let keys = self.entries
                .keys()
                .filter(|key| key.font_fingerprint == fingerprint)
                .cloned()
                .collect::<Vec<_>>();
            for key in keys {
                self.atlas.release(&key);
                if let Some(entry) = self.entries.get_mut(&key) {
                    entry.atlas_page = None;
                    entry.atlas_slot = None;
                    entry.residency_generation += 1;
                    entry.font_generation = generation;
                }
            }
            self.metrics.font_generation_invalidations += 1;
        }
        Ok(())
    }

    pub fn request(&mut self, key: GlyphCacheKey) -> Result<GlyphRequestResult, GlyphRuntimeError> {
        validate_fingerprint(&key.font_fingerprint)?;
        self.metrics.requests += 1;
        self.touch_seq += 1;

        let font = self.fonts.get(&key.font_fingerprint).cloned().unwrap_or(FontState {
            state: FontRuntimeState::Pending,
            generation: 0,
        });
        match font.state {
            FontRuntimeState::Pending => {
                self.metrics.pending += 1;
                return Ok(GlyphRequestResult::Pending { key });
            }
            FontRuntimeState::Blocked => {
                self.metrics.blocked += 1;
                return Ok(GlyphRequestResult::Blocked { key });
            }
            FontRuntimeState::Failed => {
                self.metrics.failed += 1;
                return Ok(GlyphRequestResult::Failed { key });
            }
            FontRuntimeState::Ready => {}
        }

        if key.render_mode == GlyphRenderMode::ColorComplex {
            self.metrics.unsupported += 1;
            return Ok(GlyphRequestResult::Unsupported {
                key,
                reason: "color_complex_requires_explicit_materializer_path".into(),
            });
        }

        if self.entries.get(&key).is_some_and(|entry| {
            entry.atlas_page.is_some()
                && entry.atlas_slot.is_some()
                && entry.font_generation == font.generation
        }) {
            self.metrics.hits += 1;
            let device_generation = self.device_generation;
            let atlas_generation = self.atlas.generation;
            let rasterizer_version = self.rasterizer_version;
            let entry = self.entries.get_mut(&key).expect("checked above");
            entry.last_touch = self.touch_seq;
            let binding = GlyphBinding {
                key: key.clone(),
                residency_generation: entry.residency_generation,
                font_generation: entry.font_generation,
                device_generation,
                atlas_generation,
                atlas_page: entry.atlas_page.expect("resident entry page"),
                atlas_slot: entry.atlas_slot.expect("resident entry slot"),
                rasterizer_version,
            };
            return Ok(GlyphRequestResult::Ready { binding });
        }

        self.metrics.misses += 1;
        self.ensure_atlas_capacity(&key)?;
        let material = match self.materializer.materialize(&key) {
            Ok(bytes) => bytes,
            Err(MaterializeError::Unsupported(reason)) => {
                self.metrics.unsupported += 1;
                return Ok(GlyphRequestResult::Unsupported { key, reason });
            }
            Err(MaterializeError::Failed(_reason)) => {
                self.metrics.failed += 1;
                return Ok(GlyphRequestResult::Failed { key });
            }
        };
        let (page, slot) = self.atlas.allocate(&key).ok_or(GlyphRuntimeError::AtlasCapacityUnavailable)?;
        let residency_generation = self.entries.get(&key)
            .map(|entry| entry.residency_generation + 1)
            .unwrap_or(1);
        let len = material.len() as u64;
        self.entries.insert(
            key.clone(),
            GlyphEntry {
                material_bytes: material,
                residency_generation,
                font_generation: font.generation,
                last_touch: self.touch_seq,
                atlas_page: Some(page),
                atlas_slot: Some(slot),
            },
        );
        self.metrics.materialize_bytes += len;
        self.metrics.upload_bytes += len;
        let entry = self.entries.get(&key).expect("just inserted");
        Ok(GlyphRequestResult::Ready {
            binding: self.binding(&key, entry),
        })
    }

    fn ensure_atlas_capacity(&mut self, requested: &GlyphCacheKey) -> Result<(), GlyphRuntimeError> {
        if self.atlas.placements.contains_key(requested) || self.atlas.placements.len() < self.atlas.capacity() {
            return Ok(());
        }
        let victim = self.entries.iter()
            .filter(|(_, entry)| entry.atlas_page.is_some())
            .min_by_key(|(key, entry)| (entry.last_touch, (*key).clone()))
            .map(|(key, _)| key.clone())
            .ok_or(GlyphRuntimeError::AtlasCapacityUnavailable)?;
        self.evict_internal(&victim, true);
        Ok(())
    }

    fn binding(&self, key: &GlyphCacheKey, entry: &GlyphEntry) -> GlyphBinding {
        GlyphBinding {
            key: key.clone(),
            residency_generation: entry.residency_generation,
            font_generation: entry.font_generation,
            device_generation: self.device_generation,
            atlas_generation: self.atlas.generation,
            atlas_page: entry.atlas_page.expect("resident entry page"),
            atlas_slot: entry.atlas_slot.expect("resident entry slot"),
            rasterizer_version: self.rasterizer_version,
        }
    }

    pub fn validate_binding(&mut self, binding: &GlyphBinding) -> bool {
        let ok = self.entries.get(&binding.key).is_some_and(|entry| {
            entry.atlas_page == Some(binding.atlas_page)
                && entry.atlas_slot == Some(binding.atlas_slot)
                && entry.residency_generation == binding.residency_generation
                && entry.font_generation == binding.font_generation
                && binding.device_generation == self.device_generation
                && binding.atlas_generation == self.atlas.generation
                && binding.rasterizer_version == self.rasterizer_version
        });
        if !ok {
            self.metrics.stale_binding_rejections += 1;
        }
        ok
    }

    pub fn evict(&mut self, key: &GlyphCacheKey) {
        self.evict_internal(key, false);
    }

    fn evict_internal(&mut self, key: &GlyphCacheKey, automatic: bool) {
        if self.atlas.placements.contains_key(key) {
            self.atlas.release(key);
            if let Some(entry) = self.entries.get_mut(key) {
                entry.atlas_page = None;
                entry.atlas_slot = None;
                entry.residency_generation += 1;
            }
            self.metrics.evictions += 1;
            if automatic {
                self.metrics.automatic_evictions += 1;
            }
        }
    }

    pub fn repack(&mut self) -> usize {
        let moved = self.atlas.repack();
        for (key, entry) in &mut self.entries {
            if let Some((page, slot)) = self.atlas.placements.get(key).copied() {
                entry.atlas_page = Some(page);
                entry.atlas_slot = Some(slot);
                entry.residency_generation += 1;
            }
        }
        self.metrics.repacks += 1;
        moved
    }

    pub fn reset_device(&mut self) {
        self.device_generation += 1;
        self.atlas = Atlas::new(self.atlas.page_capacity, self.atlas.max_pages)
            .expect("existing atlas bounds are valid");
        for entry in self.entries.values_mut() {
            if entry.atlas_page.is_some() {
                entry.atlas_page = None;
                entry.atlas_slot = None;
                entry.residency_generation += 1;
            }
        }
        self.metrics.device_resets += 1;
    }

    pub fn prepare_run(
        &mut self,
        run: &RenderGlyphRunV1,
        font: &RenderFontResourceV1,
        face_index: u32,
        zoom: f64,
        dpr: f64,
    ) -> Result<PreparedGlyphRun, GlyphRuntimeError> {
        if font.kind != "font" {
            return Err(GlyphRuntimeError::InvalidFontResourceKind { found: font.kind.clone() });
        }
        if font.resource_id != run.font_resource_id {
            return Err(GlyphRuntimeError::FontResourceMismatch {
                expected: run.font_resource_id.clone(),
                found: font.resource_id.clone(),
            });
        }
        if font.content_hash.is_empty() {
            return Err(GlyphRuntimeError::MissingFontFingerprint);
        }
        validate_fingerprint(&font.content_hash)?;
        let policy = select_material_policy(zoom, dpr)?;
        let geometry = run.glyphs.clone();
        let materials = run.glyphs.iter()
            .map(|glyph| {
                self.request(GlyphCacheKey {
                    font_fingerprint: font.content_hash.clone(),
                    face_index,
                    glyph_id: glyph.glyph_id,
                    render_mode: policy.render_mode,
                    scale_bucket: policy.scale_bucket,
                    quality: policy.quality,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(PreparedGlyphRun {
            canonical_geometry_mutated: geometry != run.glyphs,
            geometry,
            materials,
            render_mode: policy.render_mode,
            scale_bucket: policy.scale_bucket,
            quality: policy.quality,
        })
    }

    pub fn receipt(&self) -> GlyphRuntimeReceipt {
        let requests = self.metrics.requests;
        let hit_ratio_ppm = if requests == 0 {
            0
        } else {
            u32::try_from((self.metrics.hits * 1_000_000) / requests).unwrap_or(u32::MAX)
        };
        GlyphRuntimeReceipt {
            schema: GLYPH_RUNTIME_SCHEMA_V1.into(),
            device_generation: self.device_generation,
            atlas_generation: self.atlas.generation,
            unique_logical_entries: self.entries.len(),
            resident_entries: self.atlas.placements.len(),
            resident_material_bytes: self.entries.values()
                .filter(|entry| entry.atlas_page.is_some())
                .map(|entry| entry.material_bytes.len() as u64)
                .sum(),
            atlas: self.atlas.occupancy(),
            metrics: self.metrics.clone(),
            hit_ratio_ppm,
            authority: GlyphRuntimeAuthority {
                reshapes_text: false,
                discovers_host_fonts: false,
                mutates_canonical_glyph_positions: false,
                atlas_slot_is_semantic_identity: false,
            },
        }
    }
}

fn validate_fingerprint(value: &str) -> Result<(), GlyphRuntimeError> {
    let stripped = value.strip_prefix("sha256:").unwrap_or(value);
    if stripped.len() != 64 || !stripped.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(GlyphRuntimeError::InvalidFingerprint);
    }
    Ok(())
}

#[derive(Debug, Default)]
pub struct DeterministicTestMaterializer {
    pub calls: u64,
}

impl GlyphMaterializer for DeterministicTestMaterializer {
    fn materialize(&mut self, key: &GlyphCacheKey) -> Result<Vec<u8>, MaterializeError> {
        self.calls += 1;
        let mut hasher = Sha256::new();
        hasher.update(key.font_fingerprint.as_bytes());
        hasher.update(key.face_index.to_le_bytes());
        hasher.update(key.glyph_id.to_le_bytes());
        hasher.update([key.render_mode as u8]);
        hasher.update(key.scale_bucket.to_le_bytes());
        hasher.update([key.quality as u8]);
        let digest = hasher.finalize();
        let size = match key.render_mode {
            GlyphRenderMode::Raster => usize::from(key.scale_bucket.max(1)) * 32,
            GlyphRenderMode::Vector => 128,
            GlyphRenderMode::ColorComplex => return Err(MaterializeError::Unsupported("color_complex".into())),
        };
        let mut out = Vec::with_capacity(size);
        while out.len() < size {
            out.extend_from_slice(&digest);
        }
        out.truncate(size);
        Ok(out)
    }
}
