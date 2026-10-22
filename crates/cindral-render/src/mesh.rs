//! Cached model mesh generation, incremental GPU buffer updates, and offscreen compositing.
//!
//! [`MeshCache`] converts topological [`Model`] faces and edges with cached visibility flags into
//! contiguous triangle and line vertex arrays with power-of-two/fixed-capacity
//! slot reuse. Unchanged entities are never re-triangulated or re-uploaded.
//! [`GeometryBuffers`] maintains persistent `wgpu` vertex buffers with at least
//! 2× initial headroom and copy-preserving geometric growth. [`ModelRenderer`]
//! and [`ModelCallback`] render depth-tested faces and edges into a single-sample
//! transparent offscreen target and composite the result with premultiplied alpha
//! into `egui`'s multisampled viewport pass.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Weak};

use bytemuck::{Pod, Zeroable};
use cindral_core::{DVec3, EdgeId, Face, FaceId, KernelError, Model};
use egui_wgpu::{wgpu, CallbackResources, CallbackTrait, ScreenDescriptor};

/// Default flat neutral RGBA material color for model faces.
pub const FACE_COLOR: [f32; 4] = [0.58, 0.63, 0.69, 1.0];

/// Material style for a triangulated model face.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceStyle {
    /// Flat linear RGBA display color.
    pub color: [f32; 4],
    /// True when the face has an explicit material assignment overriding inherited container materials.
    pub explicit_material: bool,
    /// Texture array layer index (0 = default white fallback).
    pub texture_layer: u32,
}

impl FaceStyle {
    /// Creates a new face style with the default white texture layer (0).
    pub fn new(color: [f32; 4], explicit_material: bool) -> Self {
        Self {
            color,
            explicit_material,
            texture_layer: 0,
        }
    }

    /// Sets the texture array layer.
    pub fn with_texture(mut self, texture_layer: u32) -> Self {
        self.texture_layer = texture_layer;
        self
    }
}

impl Default for FaceStyle {
    fn default() -> Self {
        Self {
            color: FACE_COLOR,
            explicit_material: false,
            texture_layer: 0,
        }
    }
}

/// Default linear RGB color for model edges.
pub const EDGE_COLOR: [f32; 3] = [0.92, 0.94, 0.97];

/// Offscreen color target format (transparent RGBA).
const OFFSCREEN_COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Offscreen depth target format.
const OFFSCREEN_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Minimum byte capacity allocated for a GPU vertex buffer.
const MIN_BUFFER_BYTES: u64 = 256;

/// Interleaved GPU vertex for triangulated model faces.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct FaceVertex {
    /// World-space position in meters.
    pub position: [f32; 3],
    /// Unit supporting-plane normal.
    pub normal: [f32; 3],
    /// Flat linear RGBA material color.
    pub color: [f32; 4],
    /// Explicit material flag (1.0 = explicit face material, 0.0 = inherited container material).
    pub material: f32,
}

/// Interleaved GPU vertex for non-soft model edges.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct EdgeVertex {
    /// World-space endpoint position in meters.
    pub position: [f32; 3],
    /// RGB magnitudes; negative R/G encode soft/smooth, negative B a boundary profile.
    pub color: [f32; 3],
    /// First adjacent face normal for view-dependent silhouette profiles.
    pub normal_a: [f32; 3],
    /// Consistently oriented second adjacent face normal, zero at boundaries.
    pub normal_b: [f32; 3],
}

const FACE_VERTEX_BYTES: u64 = std::mem::size_of::<FaceVertex>() as u64;
const EDGE_VERTEX_BYTES: u64 = std::mem::size_of::<EdgeVertex>() as u64;

/// A contiguous byte range to upload into a GPU vertex buffer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BufferWrite {
    /// Byte offset from the start of the target GPU vertex buffer.
    pub offset: u64,
    /// Raw vertex bytes to write at `offset`.
    pub bytes: Vec<u8>,
}

/// CPU-prepared vertex buffer delta or full reset for [`GeometryBuffers`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MeshUpdate {
    /// Dirty byte ranges to write into the face triangle vertex buffer.
    pub face_writes: Vec<BufferWrite>,
    /// Dirty byte ranges to write into the edge line vertex buffer.
    pub edge_writes: Vec<BufferWrite>,
    /// Total number of vertices in the face buffer slot array (multiple of 3).
    pub face_vertices: u32,
    /// Total number of vertices in the edge buffer slot array (multiple of 2).
    pub edge_vertices: u32,
    /// True when the update represents a full cache rebuild.
    pub reset: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct FaceSignature {
    color: [f32; 4],
    explicit_material: bool,
    texture_layer: u32,
    normal: DVec3,
    distance: f64,
    outer: Vec<DVec3>,
    holes: Vec<Vec<DVec3>>,
}

#[derive(Clone, Debug)]
struct FaceSlot {
    /// Start index in triangles (each triangle is 3 [`FaceVertex`] elements).
    start_tri: usize,
    /// Allocated power-of-two capacity in triangles.
    capacity_tris: usize,
    /// Number of active triangles currently stored in this slot.
    active_tris: usize,
    /// Snapshot of the face geometry used to skip unchanged re-triangulation.
    signature: FaceSignature,
    boundary_edges: Vec<EdgeId>,
}

#[derive(Clone, Copy, Debug)]
struct EdgeSlot {
    /// Slot index in line segments (each segment is 2 [`EdgeVertex`] elements).
    slot: usize,
    /// Cached endpoint vertices.
    vertices: [EdgeVertex; 2],
}

/// CPU-side slot-allocated mesh cache for a [`Model`].
///
/// Faces are stored in power-of-two triangle slots and edges with cached visibility flags in
/// fixed-capacity 1-segment slots. Removed or inactive slots are zeroed out as
/// degenerate geometry and reused on subsequent allocations.
#[derive(Clone, Debug, Default)]
pub struct MeshCache {
    face_data: Vec<FaceVertex>,
    edge_data: Vec<EdgeVertex>,
    face_slots: BTreeMap<FaceId, FaceSlot>,
    edge_slots: BTreeMap<EdgeId, EdgeSlot>,
    free_face_slots: BTreeMap<usize, BTreeSet<usize>>,
    free_edge_slots: BTreeSet<usize>,
}

impl MeshCache {
    /// Triangulates all model faces and extracts all edges with cached visibility flags once using a custom face style closure,
    /// returning a populated cache and a full [`MeshUpdate`] (`reset = true`).
    pub fn rebuild_styled(
        model: &Model,
        style_fn: &impl Fn(FaceId, &Face) -> FaceStyle,
    ) -> Result<(Self, MeshUpdate), KernelError> {
        let mut cache = Self::default();

        for (&face_id, face) in model.faces() {
            let style = style_fn(face_id, face);
            let signature = build_face_signature(model, face, style)?;
            let triangles = build_face_triangles(model, face_id, face, style)?;
            if triangles.is_empty() {
                continue;
            }
            let active_tris = triangles.len() / 3;
            let capacity_tris = active_tris.next_power_of_two();
            let start_tri = cache.face_data.len() / 3;
            cache.face_data.extend_from_slice(&triangles);
            cache
                .face_data
                .resize((start_tri + capacity_tris) * 3, FaceVertex::zeroed());
            cache.face_slots.insert(
                face_id,
                FaceSlot {
                    start_tri,
                    capacity_tris,
                    active_tris,
                    signature,
                    boundary_edges: model.face_edges(face_id),
                },
            );
        }

        for (&edge_id, edge) in model.edges() {
            let vertices = styled_edge_vertices(model, edge_id, edge, DVec3::ZERO)?;
            let slot = cache.edge_data.len() / 2;
            cache.edge_data.extend_from_slice(&vertices);
            cache
                .edge_slots
                .insert(edge_id, EdgeSlot { slot, vertices });
        }

        let face_vertices =
            u32::try_from(cache.face_data.len()).map_err(|_| KernelError::Capacity)?;
        let edge_vertices =
            u32::try_from(cache.edge_data.len()).map_err(|_| KernelError::Capacity)?;

        let face_writes = if cache.face_data.is_empty() {
            Vec::new()
        } else {
            vec![BufferWrite {
                offset: 0,
                bytes: bytemuck::cast_slice(&cache.face_data).to_vec(),
            }]
        };
        let edge_writes = if cache.edge_data.is_empty() {
            Vec::new()
        } else {
            vec![BufferWrite {
                offset: 0,
                bytes: bytemuck::cast_slice(&cache.edge_data).to_vec(),
            }]
        };

        let update = MeshUpdate {
            face_writes,
            edge_writes,
            face_vertices,
            edge_vertices,
            reset: true,
        };
        Ok((cache, update))
    }

    /// Triangulates all model faces and extracts all edges with cached visibility flags once,
    /// returning a populated cache and a full [`MeshUpdate`] (`reset = true`).
    pub fn rebuild(model: &Model) -> Result<(Self, MeshUpdate), KernelError> {
        Self::rebuild_styled(model, &default_face_style)
    }

    /// Incrementally updates only the supplied `faces` and `edges` IDs using a custom face style closure.
    pub fn update_styled(
        &mut self,
        model: &Model,
        faces: &[FaceId],
        edges: &[EdgeId],
        style_fn: &impl Fn(FaceId, &Face) -> FaceStyle,
    ) -> Result<MeshUpdate, KernelError> {
        self.update_styled_in_frame(model, faces, edges, DVec3::ZERO, style_fn)
    }

    /// Update dirty geometry in a translated mesh frame without rebuilding untouched slots.
    #[tracing::instrument(level = "info", skip_all, name = "mesh.cpu_dirty")]
    pub fn update_styled_in_frame(
        &mut self,
        model: &Model,
        faces: &[FaceId],
        edges: &[EdgeId],
        offset: DVec3,
        style_fn: &impl Fn(FaceId, &Face) -> FaceStyle,
    ) -> Result<MeshUpdate, KernelError> {
        enum PreparedFace {
            Remove(FaceId),
            Unchanged,
            Upsert {
                id: FaceId,
                signature: FaceSignature,
                vertices: Vec<FaceVertex>,
            },
        }

        enum PreparedEdge {
            Remove(EdgeId),
            Unchanged,
            Upsert {
                id: EdgeId,
                vertices: [EdgeVertex; 2],
            },
        }

        let mut seen_faces = BTreeSet::new();
        let mut prepared_faces = Vec::with_capacity(faces.len());
        for &face_id in faces {
            if !seen_faces.insert(face_id) {
                continue;
            }
            match model.faces().get(&face_id) {
                None => prepared_faces.push(PreparedFace::Remove(face_id)),
                Some(face) => {
                    let style = style_fn(face_id, face);
                    let signature = build_face_signature_in_frame(model, face, style, offset)?;
                    if self
                        .face_slots
                        .get(&face_id)
                        .is_some_and(|slot| slot.signature == signature)
                    {
                        prepared_faces.push(PreparedFace::Unchanged);
                    } else {
                        let vertices =
                            build_face_triangles_in_frame(model, face_id, face, style, offset)?;
                        if vertices.is_empty() {
                            prepared_faces.push(PreparedFace::Remove(face_id));
                        } else {
                            prepared_faces.push(PreparedFace::Upsert {
                                id: face_id,
                                signature,
                                vertices,
                            });
                        }
                    }
                }
            }
        }

        // Face adjacency can change while an edge's endpoint IDs stay unchanged.
        // Include old and new boundaries so cached profile normals remain correct.
        let mut dirty_edges: BTreeSet<_> = edges.iter().copied().collect();
        for id in faces {
            if let Some(slot) = self.face_slots.get(id) {
                dirty_edges.extend(slot.boundary_edges.iter().copied());
            }
            dirty_edges.extend(model.face_edges(*id));
        }
        let edges: Vec<_> = dirty_edges.into_iter().collect();
        let mut seen_edges = BTreeSet::new();
        let mut prepared_edges = Vec::with_capacity(edges.len());
        for &edge_id in &edges {
            if !seen_edges.insert(edge_id) {
                continue;
            }
            match model.edges().get(&edge_id) {
                None => prepared_edges.push(PreparedEdge::Remove(edge_id)),
                Some(edge) => {
                    let vertices = styled_edge_vertices(model, edge_id, edge, offset)?;
                    if self
                        .edge_slots
                        .get(&edge_id)
                        .is_some_and(|slot| slot.vertices == vertices)
                    {
                        prepared_edges.push(PreparedEdge::Unchanged);
                    } else {
                        prepared_edges.push(PreparedEdge::Upsert {
                            id: edge_id,
                            vertices,
                        });
                    }
                }
            }
        }

        let mut dirty_face_ranges: Vec<(usize, usize)> = Vec::new();
        for op in prepared_faces {
            match op {
                PreparedFace::Unchanged => {}
                PreparedFace::Remove(face_id) => {
                    if let Some(slot) = self.face_slots.remove(&face_id) {
                        let start_v = slot.start_tri * 3;
                        let end_v = (slot.start_tri + slot.capacity_tris) * 3;
                        self.face_data[start_v..end_v].fill(FaceVertex::zeroed());
                        self.free_face_slot(slot.start_tri, slot.capacity_tris);
                        dirty_face_ranges.push((start_v, end_v));
                    }
                }
                PreparedFace::Upsert {
                    id,
                    signature,
                    vertices,
                } => {
                    let active_tris = vertices.len() / 3;
                    let needed_cap = active_tris.next_power_of_two();
                    if let Some(existing) = self.face_slots.remove(&id) {
                        if needed_cap <= existing.capacity_tris {
                            let start_v = existing.start_tri * 3;
                            let active_end_v = start_v + vertices.len();
                            let prev_end_v = start_v + existing.active_tris.max(active_tris) * 3;
                            self.face_data[start_v..active_end_v].copy_from_slice(&vertices);
                            if active_end_v < prev_end_v {
                                self.face_data[active_end_v..prev_end_v].fill(FaceVertex::zeroed());
                            }
                            self.face_slots.insert(
                                id,
                                FaceSlot {
                                    start_tri: existing.start_tri,
                                    capacity_tris: existing.capacity_tris,
                                    active_tris,
                                    signature,
                                    boundary_edges: model.face_edges(id),
                                },
                            );
                            dirty_face_ranges.push((start_v, prev_end_v));
                            continue;
                        }
                        let old_start_v = existing.start_tri * 3;
                        let old_end_v = (existing.start_tri + existing.capacity_tris) * 3;
                        self.face_data[old_start_v..old_end_v].fill(FaceVertex::zeroed());
                        self.free_face_slot(existing.start_tri, existing.capacity_tris);
                        dirty_face_ranges.push((old_start_v, old_end_v));
                    }

                    let start_tri = self.allocate_face_slot(needed_cap);
                    let start_v = start_tri * 3;
                    let active_end_v = start_v + vertices.len();
                    let cap_end_v = (start_tri + needed_cap) * 3;
                    self.face_data[start_v..active_end_v].copy_from_slice(&vertices);
                    if active_end_v < cap_end_v {
                        self.face_data[active_end_v..cap_end_v].fill(FaceVertex::zeroed());
                    }
                    self.face_slots.insert(
                        id,
                        FaceSlot {
                            start_tri,
                            capacity_tris: needed_cap,
                            active_tris,
                            signature,
                            boundary_edges: model.face_edges(id),
                        },
                    );
                    dirty_face_ranges.push((start_v, cap_end_v));
                }
            }
        }

        let mut dirty_edge_ranges: Vec<(usize, usize)> = Vec::new();
        for op in prepared_edges {
            match op {
                PreparedEdge::Unchanged => {}
                PreparedEdge::Remove(edge_id) => {
                    if let Some(existing) = self.edge_slots.remove(&edge_id) {
                        let start_v = existing.slot * 2;
                        let end_v = start_v + 2;
                        self.edge_data[start_v..end_v].fill(EdgeVertex::zeroed());
                        self.free_edge_slots.insert(existing.slot);
                        dirty_edge_ranges.push((start_v, end_v));
                    }
                }
                PreparedEdge::Upsert { id, vertices } => {
                    let slot = if let Some(existing) = self.edge_slots.get(&id) {
                        existing.slot
                    } else if let Some(free_slot) = self.free_edge_slots.pop_first() {
                        free_slot
                    } else {
                        let new_slot = self.edge_data.len() / 2;
                        self.edge_data.extend_from_slice(&[EdgeVertex::zeroed(); 2]);
                        new_slot
                    };
                    let start_v = slot * 2;
                    let end_v = start_v + 2;
                    self.edge_data[start_v..end_v].copy_from_slice(&vertices);
                    self.edge_slots.insert(id, EdgeSlot { slot, vertices });
                    dirty_edge_ranges.push((start_v, end_v));
                }
            }
        }

        let face_vertices =
            u32::try_from(self.face_data.len()).map_err(|_| KernelError::Capacity)?;
        let edge_vertices =
            u32::try_from(self.edge_data.len()).map_err(|_| KernelError::Capacity)?;

        let face_writes = merge_ranges(dirty_face_ranges)
            .into_iter()
            .map(|(start_v, end_v)| BufferWrite {
                offset: (start_v as u64) * FACE_VERTEX_BYTES,
                bytes: bytemuck::cast_slice(&self.face_data[start_v..end_v]).to_vec(),
            })
            .collect();

        let edge_writes = merge_ranges(dirty_edge_ranges)
            .into_iter()
            .map(|(start_v, end_v)| BufferWrite {
                offset: (start_v as u64) * EDGE_VERTEX_BYTES,
                bytes: bytemuck::cast_slice(&self.edge_data[start_v..end_v]).to_vec(),
            })
            .collect();

        for id in faces {
            if let Some(slot) = self.face_slots.get_mut(id) {
                slot.boundary_edges = model.face_edges(*id);
            }
        }
        Ok(MeshUpdate {
            face_writes,
            edge_writes,
            face_vertices,
            edge_vertices,
            reset: false,
        })
    }

    /// Incrementally updates only the supplied `faces` and `edges` IDs.
    ///
    /// Missing IDs release their slots and zero the
    /// underlying vertices as degenerate geometry. Unchanged slots are never
    /// re-triangulated or rewritten. Never traverses the full [`Model`].
    pub fn update(
        &mut self,
        model: &Model,
        faces: &[FaceId],
        edges: &[EdgeId],
    ) -> Result<MeshUpdate, KernelError> {
        self.update_styled(model, faces, edges, &default_face_style)
    }

    fn free_face_slot(&mut self, start_tri: usize, capacity_tris: usize) {
        self.free_face_slots
            .entry(capacity_tris)
            .or_default()
            .insert(start_tri);
    }

    fn allocate_face_slot(&mut self, needed_cap: usize) -> usize {
        if let Some(set) = self.free_face_slots.get_mut(&needed_cap) {
            if let Some(start_tri) = set.pop_first() {
                return start_tri;
            }
        }
        let larger_cap = self
            .free_face_slots
            .range((needed_cap + 1)..)
            .find_map(|(&cap, set)| (!set.is_empty()).then_some(cap));
        if let Some(mut block_cap) = larger_cap {
            if let Some(set) = self.free_face_slots.get_mut(&block_cap) {
                if let Some(start_tri) = set.pop_first() {
                    while block_cap > needed_cap {
                        block_cap /= 2;
                        self.free_face_slots
                            .entry(block_cap)
                            .or_default()
                            .insert(start_tri + block_cap);
                    }
                    return start_tri;
                }
            }
        }
        let start_tri = self.face_data.len() / 3;
        self.face_data
            .resize((start_tri + needed_cap) * 3, FaceVertex::zeroed());
        start_tri
    }
}

fn default_face_style(_id: FaceId, face: &Face) -> FaceStyle {
    FaceStyle {
        color: if face.color == [160, 177, 193, 255] {
            FACE_COLOR
        } else {
            [
                face.color[0] as f32 / 255.0,
                face.color[1] as f32 / 255.0,
                face.color[2] as f32 / 255.0,
                face.color[3] as f32 / 255.0,
            ]
        },
        explicit_material: face.material.is_some(),
        texture_layer: 0,
    }
}

fn build_face_signature(
    model: &Model,
    face: &Face,
    style: FaceStyle,
) -> Result<FaceSignature, KernelError> {
    build_face_signature_in_frame(model, face, style, DVec3::ZERO)
}
fn build_face_signature_in_frame(
    model: &Model,
    face: &Face,
    style: FaceStyle,
    offset: DVec3,
) -> Result<FaceSignature, KernelError> {
    if !face.plane.normal.is_finite() || !face.plane.distance.is_finite() {
        return Err(KernelError::InvalidPosition);
    }
    let outer = face
        .outer
        .iter()
        .map(|id| {
            let v = model.vertices().get(id).ok_or(KernelError::MissingEntity)?;
            if !v.position.is_finite() {
                return Err(KernelError::InvalidPosition);
            }
            Ok(v.position - offset)
        })
        .collect::<Result<Vec<_>, KernelError>>()?;
    let holes = face
        .holes
        .iter()
        .map(|ring| {
            ring.iter()
                .map(|id| {
                    let v = model.vertices().get(id).ok_or(KernelError::MissingEntity)?;
                    if !v.position.is_finite() {
                        return Err(KernelError::InvalidPosition);
                    }
                    Ok(v.position - offset)
                })
                .collect::<Result<Vec<_>, KernelError>>()
        })
        .collect::<Result<Vec<_>, KernelError>>()?;
    Ok(FaceSignature {
        color: style.color,
        explicit_material: style.explicit_material,
        texture_layer: style.texture_layer,
        normal: face.plane.normal,
        distance: face.plane.distance - face.plane.normal.dot(offset),
        outer,
        holes,
    })
}

fn build_face_triangles(
    model: &Model,
    face_id: FaceId,
    face: &Face,
    style: FaceStyle,
) -> Result<Vec<FaceVertex>, KernelError> {
    build_face_triangles_in_frame(model, face_id, face, style, DVec3::ZERO)
}
fn build_face_triangles_in_frame(
    model: &Model,
    face_id: FaceId,
    face: &Face,
    style: FaceStyle,
    offset: DVec3,
) -> Result<Vec<FaceVertex>, KernelError> {
    let tris = model.triangulate(face_id)?;
    let normal = face.plane.normal.as_vec3();
    let Some(normal) = normal.try_normalize() else {
        return Err(KernelError::InvalidPosition);
    };
    let normal_arr = normal.to_array();
    let packed_mat =
        crate::texture::pack_face_material(style.texture_layer, style.explicit_material);
    let mut vertices = Vec::with_capacity(tris.len() * 3);
    for tri in tris {
        for p in tri {
            let pos = (p - offset).as_vec3();
            if !pos.is_finite() {
                return Err(KernelError::InvalidPosition);
            }
            vertices.push(FaceVertex {
                position: pos.to_array(),
                normal: normal_arr,
                color: style.color,
                material: packed_mat,
            });
        }
    }
    Ok(vertices)
}

fn styled_edge_vertices(
    model: &Model,
    id: EdgeId,
    edge: &cindral_core::Edge,
    offset: DVec3,
) -> Result<[EdgeVertex; 2], KernelError> {
    if edge.hidden {
        return Ok([EdgeVertex::zeroed(); 2]);
    }
    let mut vertices = build_edge_vertices_in_frame(model, edge.vertices, offset)?;
    let faces: Vec<_> = model
        .edge_faces(id)
        .take(2)
        .map(|(_, face)| {
            let forward = std::iter::once(&face.outer)
                .chain(face.holes.iter())
                .any(|ring| {
                    ring.iter()
                        .zip(ring.iter().cycle().skip(1))
                        .take(ring.len())
                        .any(|(a, b)| *a == edge.vertices[0] && *b == edge.vertices[1])
                });
            (face.plane.normal, forward)
        })
        .collect();
    let normal_a = faces.first().map_or(DVec3::ZERO, |f| f.0);
    let normal_b = faces
        .get(1)
        .map_or(DVec3::ZERO, |f| if f.1 == faces[0].1 { -f.0 } else { f.0 });
    for vertex in &mut vertices {
        vertex.normal_a = normal_a.as_vec3().to_array();
        vertex.normal_b = normal_b.as_vec3().to_array();
        // Signs encode immutable edge attributes; zero remains the free-slot sentinel.
        if edge.soft {
            vertex.color[0] = -vertex.color[0];
        }
        if edge.smooth {
            vertex.color[1] = -vertex.color[1];
        }
        if faces.len() < 2 {
            vertex.color[2] = -vertex.color[2];
        }
    }
    Ok(vertices)
}
fn build_edge_vertices_in_frame(
    model: &Model,
    endpoints: [cindral_core::VertexId; 2],
    offset: DVec3,
) -> Result<[EdgeVertex; 2], KernelError> {
    let a = (model
        .vertices()
        .get(&endpoints[0])
        .ok_or(KernelError::MissingEntity)?
        .position
        - offset)
        .as_vec3();
    let b = (model
        .vertices()
        .get(&endpoints[1])
        .ok_or(KernelError::MissingEntity)?
        .position
        - offset)
        .as_vec3();
    if !a.is_finite() || !b.is_finite() {
        return Err(KernelError::InvalidPosition);
    }
    Ok([
        EdgeVertex {
            position: a.to_array(),
            color: EDGE_COLOR,
            normal_a: [0.; 3],
            normal_b: [0.; 3],
        },
        EdgeVertex {
            position: b.to_array(),
            color: EDGE_COLOR,
            normal_a: [0.; 3],
            normal_b: [0.; 3],
        },
    ])
}

fn merge_ranges(mut ranges: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    ranges.retain(|(start, end)| start < end);
    if ranges.is_empty() {
        return Vec::new();
    }
    ranges.sort_unstable_by_key(|&(start, _)| start);
    let mut merged = Vec::with_capacity(ranges.len());
    let mut current = ranges[0];
    for &(start, end) in &ranges[1..] {
        if start <= current.1 {
            current.1 = current.1.max(end);
        } else {
            merged.push(current);
            current = (start, end);
        }
    }
    merged.push(current);
    merged
}

/// Persistent GPU vertex buffers for cached model faces and edges.
///
/// Allocates with at least 2× initial headroom, writes only dirty byte ranges,
/// and preserves existing GPU buffer contents via buffer-to-buffer copy when
/// growing geometrically.
pub struct GeometryBuffers {
    face_buffer: wgpu::Buffer,
    edge_buffer: wgpu::Buffer,
    face_capacity: u64,
    edge_capacity: u64,
    face_vertices: u32,
    edge_vertices: u32,
}

impl GeometryBuffers {
    /// Creates initial persistent face and edge vertex buffers on `device`.
    pub fn new(device: &wgpu::Device) -> Self {
        let face_buffer = create_geometry_buffer(device, "Cindral model faces", MIN_BUFFER_BYTES);
        let edge_buffer = create_geometry_buffer(device, "Cindral model edges", MIN_BUFFER_BYTES);
        Self {
            face_buffer,
            edge_buffer,
            face_capacity: MIN_BUFFER_BYTES,
            edge_capacity: MIN_BUFFER_BYTES,
            face_vertices: 0,
            edge_vertices: 0,
        }
    }

    /// Applies a [`MeshUpdate`] to the GPU vertex buffers.
    ///
    /// On initial allocation (`reset = true`) or when an update exceeds current
    /// capacity, allocates at least 2× the required size. When growing on an
    /// incremental update (`reset = false`), existing data is copied into the
    /// resized buffer before dirty ranges are written with `queue.write_buffer`.
    #[tracing::instrument(level = "info", skip_all, name = "mesh.gpu_upload")]
    pub fn apply(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, update: &MeshUpdate) {
        let required_face = (u64::from(update.face_vertices) * FACE_VERTEX_BYTES)
            .max(max_write_end(&update.face_writes));
        let required_edge = (u64::from(update.edge_vertices) * EDGE_VERTEX_BYTES)
            .max(max_write_end(&update.edge_writes));

        let grow_face = if update.reset {
            required_face.saturating_mul(2) > self.face_capacity
        } else {
            required_face > self.face_capacity
        };
        let grow_edge = if update.reset {
            required_edge.saturating_mul(2) > self.edge_capacity
        } else {
            required_edge > self.edge_capacity
        };

        if grow_face || grow_edge {
            let mut copy_encoder = (!update.reset).then(|| {
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Cindral geometry buffer resize copy"),
                })
            });

            if grow_face {
                let new_capacity = align_capacity(
                    self.face_capacity
                        .saturating_mul(2)
                        .max(required_face.saturating_mul(2))
                        .max(MIN_BUFFER_BYTES),
                );
                let new_buffer =
                    create_geometry_buffer(device, "Cindral model faces", new_capacity);
                if let Some(encoder) = copy_encoder.as_mut() {
                    let copy_bytes =
                        (u64::from(self.face_vertices) * FACE_VERTEX_BYTES).min(self.face_capacity);
                    if copy_bytes > 0 {
                        encoder.copy_buffer_to_buffer(
                            &self.face_buffer,
                            0,
                            &new_buffer,
                            0,
                            align_capacity(copy_bytes).min(self.face_capacity),
                        );
                    }
                }
                self.face_buffer = new_buffer;
                self.face_capacity = new_capacity;
            }

            if grow_edge {
                let new_capacity = align_capacity(
                    self.edge_capacity
                        .saturating_mul(2)
                        .max(required_edge.saturating_mul(2))
                        .max(MIN_BUFFER_BYTES),
                );
                let new_buffer =
                    create_geometry_buffer(device, "Cindral model edges", new_capacity);
                if let Some(encoder) = copy_encoder.as_mut() {
                    let copy_bytes =
                        (u64::from(self.edge_vertices) * EDGE_VERTEX_BYTES).min(self.edge_capacity);
                    if copy_bytes > 0 {
                        encoder.copy_buffer_to_buffer(
                            &self.edge_buffer,
                            0,
                            &new_buffer,
                            0,
                            align_capacity(copy_bytes).min(self.edge_capacity),
                        );
                    }
                }
                self.edge_buffer = new_buffer;
                self.edge_capacity = new_capacity;
            }

            if let Some(encoder) = copy_encoder {
                queue.submit([encoder.finish()]);
            }
        }

        for write in &update.face_writes {
            if !write.bytes.is_empty() {
                queue.write_buffer(&self.face_buffer, write.offset, &write.bytes);
            }
        }
        for write in &update.edge_writes {
            if !write.bytes.is_empty() {
                queue.write_buffer(&self.edge_buffer, write.offset, &write.bytes);
            }
        }

        self.face_vertices = update.face_vertices;
        self.edge_vertices = update.edge_vertices;
    }

    /// Total capacity in bytes currently allocated across face and edge GPU buffers.
    pub fn allocated_bytes(&self) -> u64 {
        self.face_capacity + self.edge_capacity
    }

    /// Active face vertex count currently scheduled for drawing.
    pub fn face_vertices(&self) -> u32 {
        self.face_vertices
    }

    /// Active edge vertex count currently scheduled for drawing.
    pub fn edge_vertices(&self) -> u32 {
        self.edge_vertices
    }

    /// Read-only reference to the persistent face GPU buffer.
    pub fn face_buffer(&self) -> &wgpu::Buffer {
        &self.face_buffer
    }

    /// Read-only reference to the persistent edge GPU buffer.
    pub fn edge_buffer(&self) -> &wgpu::Buffer {
        &self.edge_buffer
    }
}

fn create_geometry_buffer(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: align_capacity(size.max(MIN_BUFFER_BYTES)),
        usage: wgpu::BufferUsages::VERTEX
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

fn align_capacity(size: u64) -> u64 {
    let align = wgpu::COPY_BUFFER_ALIGNMENT;
    size.div_ceil(align) * align
}

fn max_write_end(writes: &[BufferWrite]) -> u64 {
    writes
        .iter()
        .map(|w| w.offset.saturating_add(w.bytes.len() as u64))
        .max()
        .unwrap_or(0)
}

struct OffscreenTarget {
    size: [u32; 2],
    color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    composite_bind_group: wgpu::BindGroup,
}

/// Persistent GPU renderer for cached [`Model`] geometry.
///
/// Renders depth-tested triangles and edges to a transparent single-sample
/// offscreen RGBA target, then composites the result with premultiplied alpha
/// over the grid in `egui`'s multisampled render pass.
pub struct ModelRenderer {
    buffers: GeometryBuffers,
    uniform: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    face_pipeline: wgpu::RenderPipeline,
    edge_pipeline: wgpu::RenderPipeline,
    composite_pipeline: wgpu::RenderPipeline,
    composite_layout: wgpu::BindGroupLayout,
    composite_sampler: wgpu::Sampler,
    offscreen: OffscreenTarget,
    last_update: Option<Weak<MeshUpdate>>,
}

impl ModelRenderer {
    /// Creates GPU pipelines, buffers, and initial offscreen targets for the
    /// viewport's color `format` and MSAA `samples`.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, samples: u32) -> Self {
        let buffers = GeometryBuffers::new(device);

        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Cindral model camera"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Cindral model camera layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Cindral model camera binding"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });

        let mesh_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Cindral model mesh shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("mesh.wgsl").into()),
        });
        let mesh_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Cindral model mesh pipeline layout"),
            bind_group_layouts: &[&camera_layout],
            push_constant_ranges: &[],
        });

        let face_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Cindral model face pipeline"),
            layout: Some(&mesh_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &mesh_shader,
                entry_point: Some("vs_face"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: FACE_VERTEX_BYTES,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x3,
                        1 => Float32x3,
                        2 => Float32x4,
                        3 => Float32
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &mesh_shader,
                entry_point: Some("fs_face"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: OFFSCREEN_COLOR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: OFFSCREEN_DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: Default::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 1.0,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let edge_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Cindral model edge pipeline"),
            layout: Some(&mesh_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &mesh_shader,
                entry_point: Some("vs_edge"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: EDGE_VERTEX_BYTES,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &mesh_shader,
                entry_point: Some("fs_edge"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: OFFSCREEN_COLOR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: OFFSCREEN_DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let composite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Cindral model composite shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("composite.wgsl").into()),
        });
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Cindral model composite layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let composite_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Cindral model composite pipeline layout"),
                bind_group_layouts: &[&composite_layout],
                push_constant_ranges: &[],
            });
        let composite_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Cindral model composite pipeline"),
            layout: Some(&composite_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &composite_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &composite_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: samples.max(1),
                ..Default::default()
            },
            multiview: None,
            cache: None,
        });

        let composite_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Cindral model composite sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        let offscreen =
            create_offscreen_target(device, &composite_layout, &composite_sampler, [1, 1]);

        Self {
            buffers,
            uniform,
            camera_bind_group,
            face_pipeline,
            edge_pipeline,
            composite_pipeline,
            composite_layout,
            composite_sampler,
            offscreen,
            last_update: None,
        }
    }

    /// Borrows the underlying [`GeometryBuffers`].
    pub fn buffers(&self) -> &GeometryBuffers {
        &self.buffers
    }

    /// Mutably borrows the underlying [`GeometryBuffers`].
    pub fn buffers_mut(&mut self) -> &mut GeometryBuffers {
        &mut self.buffers
    }

    /// Current offscreen target dimensions in physical pixels.
    pub fn target_size(&self) -> [u32; 2] {
        self.offscreen.size
    }

    fn apply_update_once(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        update: &Arc<MeshUpdate>,
    ) {
        let already_applied = self
            .last_update
            .as_ref()
            .and_then(Weak::upgrade)
            .is_some_and(|prev| Arc::ptr_eq(&prev, update));
        if !already_applied {
            self.buffers.apply(device, queue, update);
            self.last_update = Some(Arc::downgrade(update));
        }
    }

    fn ensure_target_size(&mut self, device: &wgpu::Device, size: [u32; 2]) -> bool {
        if size[0] == 0 || size[1] == 0 {
            return false;
        }
        let max_dim = device.limits().max_texture_dimension_2d.max(1);
        let clamped = [size[0].min(max_dim), size[1].min(max_dim)];
        if self.offscreen.size != clamped {
            self.offscreen = create_offscreen_target(
                device,
                &self.composite_layout,
                &self.composite_sampler,
                clamped,
            );
        }
        true
    }

    fn render_offscreen(&self, encoder: &mut wgpu::CommandEncoder) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Cindral model offscreen pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.offscreen.color_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.offscreen.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        if self.buffers.face_vertices > 0 {
            pass.set_pipeline(&self.face_pipeline);
            pass.set_bind_group(0, &self.camera_bind_group, &[]);
            pass.set_vertex_buffer(0, self.buffers.face_buffer.slice(..));
            pass.draw(0..self.buffers.face_vertices, 0..1);
        }

        if self.buffers.edge_vertices > 0 {
            pass.set_pipeline(&self.edge_pipeline);
            pass.set_bind_group(0, &self.camera_bind_group, &[]);
            pass.set_vertex_buffer(0, self.buffers.edge_buffer.slice(..));
            pass.draw(0..self.buffers.edge_vertices, 0..1);
        }
    }
}

fn create_offscreen_target(
    device: &wgpu::Device,
    composite_layout: &wgpu::BindGroupLayout,
    composite_sampler: &wgpu::Sampler,
    size: [u32; 2],
) -> OffscreenTarget {
    let extent = wgpu::Extent3d {
        width: size[0],
        height: size[1],
        depth_or_array_layers: 1,
    };
    let color = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Cindral model offscreen color"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: OFFSCREEN_COLOR_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Cindral model offscreen depth"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: OFFSCREEN_DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
    let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
    let composite_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Cindral model composite bind group"),
        layout: composite_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&color_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(composite_sampler),
            },
        ],
    });
    OffscreenTarget {
        size,
        color_view,
        depth_view,
        composite_bind_group,
    }
}

/// Per-frame `egui-wgpu` callback payload for rendering cached model geometry.
#[derive(Clone, Debug)]
pub struct ModelCallback {
    /// Column-major view-projection matrix using wgpu's `0..1` depth range.
    pub view_projection: [[f32; 4]; 4],
    /// Viewport dimensions in physical pixels (`[width, height]`).
    pub size: [u32; 2],
    /// Optional pending mesh update to upload during `prepare`.
    pub update: Option<Arc<MeshUpdate>>,
}

impl CallbackTrait for ModelCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(renderer) = resources.get_mut::<ModelRenderer>() else {
            return Vec::new();
        };
        if let Some(update) = &self.update {
            renderer.apply_update_once(device, queue, update);
        }
        queue.write_buffer(
            &renderer.uniform,
            0,
            bytemuck::cast_slice(&self.view_projection),
        );
        if renderer.ensure_target_size(device, self.size) {
            renderer.render_offscreen(encoder);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        if self.size[0] == 0 || self.size[1] == 0 {
            return;
        }
        if let Some(renderer) = resources.get::<ModelRenderer>() {
            pass.set_pipeline(&renderer.composite_pipeline);
            pass.set_bind_group(0, &renderer.offscreen.composite_bind_group, &[]);
            pass.draw(0..6, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cindral_core::{Command, History};
    use glam::Vec3;

    fn decode_face_vertices(bytes: &[u8]) -> &[FaceVertex] {
        bytemuck::cast_slice(bytes)
    }

    fn decode_edge_vertices(bytes: &[u8]) -> &[EdgeVertex] {
        bytemuck::cast_slice(bytes)
    }

    fn triangle_area(tri: &[FaceVertex; 3]) -> f32 {
        let a = Vec3::from_array(tri[0].position);
        let b = Vec3::from_array(tri[1].position);
        let c = Vec3::from_array(tri[2].position);
        (b - a).cross(c - a).length() * 0.5
    }

    #[test]
    fn empty_model_rebuild_and_noop_update() {
        let model = Model::default();
        let (mut cache, init) = MeshCache::rebuild(&model).unwrap();
        assert!(init.reset);
        assert_eq!(init.face_vertices, 0);
        assert_eq!(init.edge_vertices, 0);
        assert!(init.face_writes.is_empty());
        assert!(init.edge_writes.is_empty());

        let noop = cache.update(&model, &[], &[]).unwrap();
        assert!(!noop.reset);
        assert_eq!(noop.face_vertices, 0);
        assert_eq!(noop.edge_vertices, 0);
        assert!(noop.face_writes.is_empty());
        assert!(noop.edge_writes.is_empty());

        let missing = cache
            .update(&model, &[FaceId::from_raw(42)], &[EdgeId::from_raw(99)])
            .unwrap();
        assert!(!missing.reset);
        assert!(missing.face_writes.is_empty());
        assert!(missing.edge_writes.is_empty());
    }

    #[test]
    fn hole_triangulation_and_finite_attributes() {
        let mut history = History::new();
        // Outer 10x10 square (area 100)
        history
            .execute(Command::AddPolyline {
                points: vec![
                    DVec3::new(0.0, 0.0, 0.0),
                    DVec3::new(10.0, 0.0, 0.0),
                    DVec3::new(10.0, 10.0, 0.0),
                    DVec3::new(0.0, 10.0, 0.0),
                ],
                closed: true,
            })
            .unwrap();
        // Inner 4x4 square hole (area 16); model has both the annulus (area 84) and inner face (area 16).
        history
            .execute(Command::AddPolyline {
                points: vec![
                    DVec3::new(3.0, 3.0, 0.0),
                    DVec3::new(7.0, 3.0, 0.0),
                    DVec3::new(7.0, 7.0, 0.0),
                    DVec3::new(3.0, 7.0, 0.0),
                ],
                closed: true,
            })
            .unwrap();

        let model = history.model();
        assert_eq!(model.faces().len(), 2);
        let hole_face_id = *model
            .faces()
            .iter()
            .find(|(_, f)| !f.holes.is_empty())
            .unwrap()
            .0;
        let inner_face_id = *model
            .faces()
            .iter()
            .find(|(_, f)| f.holes.is_empty())
            .unwrap()
            .0;

        let (mut cache, init) = MeshCache::rebuild(model).unwrap();
        assert!(init.reset);
        assert_eq!(init.edge_vertices, 16);
        // Annulus with 4-vertex outer + 4-vertex hole has 8 triangles (24 vertices);
        // inner quad has 2 triangles (6 vertices). Total = 30 vertices.
        assert_eq!(init.face_vertices, 30);
        assert_eq!(init.face_writes.len(), 1);
        assert_eq!(init.edge_writes.len(), 1);

        let face_verts = decode_face_vertices(&init.face_writes[0].bytes);
        assert_eq!(face_verts.len(), 30);
        for v in face_verts {
            assert!(v.position.iter().all(|c| c.is_finite()));
            assert!(v.normal.iter().all(|c| c.is_finite()));
            assert!(v.color.iter().all(|c| c.is_finite()));
            let n = Vec3::from_array(v.normal);
            assert!((n.length() - 1.0).abs() < 1e-5);
            assert_eq!(v.color, FACE_COLOR);
        }
        let total_area: f32 = face_verts
            .as_chunks::<3>()
            .0
            .iter()
            .map(triangle_area)
            .sum();
        assert!((total_area - 100.0).abs() < 1e-3);

        // Verify that updating only the annulus face (or removing the inner face slot via a synthetic model)
        // preserves exact hole subtraction (area 84.0).
        let empty_model = Model::default();
        let remove_inner = cache.update(&empty_model, &[inner_face_id], &[]).unwrap();
        assert!(!remove_inner.reset);
        assert_eq!(remove_inner.face_writes.len(), 1);
        // Remaining active triangles in cache belong solely to the holed annulus face (area 84).
        let remaining_area: f32 = cache
            .face_data
            .as_chunks::<3>()
            .0
            .iter()
            .map(triangle_area)
            .sum();
        assert!((remaining_area - 84.0).abs() < 1e-3);
        let noop_hole = cache.update(model, &[hole_face_id], &[]).unwrap();
        assert!(noop_hole.face_writes.is_empty());
    }

    #[test]
    fn soft_edges_retained_and_attributes_updated_incrementally() {
        let mut history = History::new();
        history
            .execute(Command::AddPolyline {
                points: vec![
                    DVec3::ZERO,
                    DVec3::X * 2.0,
                    DVec3::X * 2.0 + DVec3::Y * 2.0,
                    DVec3::Y * 2.0,
                ],
                closed: true,
            })
            .unwrap();
        history
            .execute(Command::AddEdge {
                start: DVec3::ZERO,
                end: DVec3::X * 2.0 + DVec3::Y * 2.0,
            })
            .unwrap();
        assert_eq!(history.model().edges().len(), 5);
        assert_eq!(history.model().faces().len(), 2);

        let diagonal_id = *history
            .model()
            .edges()
            .iter()
            .find(|(_, e)| {
                let p0 = history.model().vertices()[&e.vertices[0]].position;
                let p1 = history.model().vertices()[&e.vertices[1]].position;
                (p0.distance(p1) - 8.0_f64.sqrt()).abs() < 1e-5
            })
            .unwrap()
            .0;

        history
            .execute(Command::SetEdgeStyle {
                edges: vec![diagonal_id],
                soft: true,
                smooth: false,
            })
            .unwrap();

        let (mut cache, init) = MeshCache::rebuild(history.model()).unwrap();
        // Soft edges remain cached; the shader controls visibility without mesh changes.
        assert_eq!(init.edge_vertices, 10);
        assert_eq!(init.face_vertices, 6);

        // Mark one perimeter edge soft and update only its encoded attributes.
        let perimeter_id = *history
            .model()
            .edges()
            .keys()
            .find(|&&id| id != diagonal_id)
            .unwrap();
        history
            .execute(Command::SetEdgeStyle {
                edges: vec![perimeter_id],
                soft: true,
                smooth: false,
            })
            .unwrap();

        let soft_update = cache
            .update(history.model(), &[], &[perimeter_id, diagonal_id])
            .unwrap();
        assert!(!soft_update.reset);
        assert!(soft_update.face_writes.is_empty());
        assert_eq!(soft_update.edge_writes.len(), 1);
        let freed_offset = soft_update.edge_writes[0].offset;
        let zeroed = decode_edge_vertices(&soft_update.edge_writes[0].bytes);
        assert!(zeroed.iter().all(|v| v.color[0] < 0.));

        // Un-soften the perimeter edge; its existing slot must stay stable.
        history
            .execute(Command::SetEdgeStyle {
                edges: vec![perimeter_id],
                soft: false,
                smooth: false,
            })
            .unwrap();
        let unsoft_update = cache.update(history.model(), &[], &[perimeter_id]).unwrap();
        assert_eq!(unsoft_update.edge_writes.len(), 1);
        assert_eq!(unsoft_update.edge_writes[0].offset, freed_offset);
        assert_eq!(unsoft_update.edge_vertices, 10);
        let restored = decode_edge_vertices(&unsoft_update.edge_writes[0].bytes);
        assert!(restored.iter().all(|v| v.color[0] > 0. && v.color[1] > 0.));
    }

    #[test]
    fn incremental_update_writes_only_dirty_ranges_and_skips_unchanged() {
        let mut history = History::new();
        // Square A at x in 0..1
        history
            .execute(Command::AddPolyline {
                points: vec![
                    DVec3::new(0.0, 0.0, 0.0),
                    DVec3::new(1.0, 0.0, 0.0),
                    DVec3::new(1.0, 1.0, 0.0),
                    DVec3::new(0.0, 1.0, 0.0),
                ],
                closed: true,
            })
            .unwrap();
        let face_a = *history.model().faces().keys().next().unwrap();
        let edges_a: Vec<EdgeId> = history.model().edges().keys().copied().collect();

        // Square B at x in 5..6
        history
            .execute(Command::AddPolyline {
                points: vec![
                    DVec3::new(5.0, 0.0, 0.0),
                    DVec3::new(6.0, 0.0, 0.0),
                    DVec3::new(6.0, 1.0, 0.0),
                    DVec3::new(5.0, 1.0, 0.0),
                ],
                closed: true,
            })
            .unwrap();
        let face_b = *history
            .model()
            .faces()
            .keys()
            .find(|&&id| id != face_a)
            .unwrap();

        let (mut cache, init) = MeshCache::rebuild(history.model()).unwrap();
        assert_eq!(init.face_vertices, 12); // 2 quads * 2 tris * 3 verts
        assert_eq!(init.edge_vertices, 16); // 8 edges * 2 verts

        // Calling update with all existing IDs when nothing changed is a no-op.
        let all_faces: Vec<FaceId> = history.model().faces().keys().copied().collect();
        let all_edges: Vec<EdgeId> = history.model().edges().keys().copied().collect();
        let noop = cache
            .update(history.model(), &all_faces, &all_edges)
            .unwrap();
        assert!(!noop.reset);
        assert!(noop.face_writes.is_empty());
        assert!(noop.edge_writes.is_empty());

        // Split Square B with a diagonal. Square A is completely untouched.
        history
            .execute(Command::AddEdge {
                start: DVec3::new(5.0, 0.0, 0.0),
                end: DVec3::new(6.0, 1.0, 0.0),
            })
            .unwrap();

        let mut dirty_faces: Vec<FaceId> = history.model().faces().keys().copied().collect();
        dirty_faces.push(face_b);
        let dirty_edges: Vec<EdgeId> = history.model().edges().keys().copied().collect();

        let delta = cache
            .update(history.model(), &dirty_faces, &dirty_edges)
            .unwrap();
        assert!(!delta.reset);

        // Square A occupies face bytes 0..(6 * 36 = 216) and edge bytes 0..(8 * 24 = 192).
        // Neither may be rewritten!
        let face_a_bytes = 6 * FACE_VERTEX_BYTES;
        let edges_a_bytes = (edges_a.len() as u64) * 2 * EDGE_VERTEX_BYTES;
        for w in &delta.face_writes {
            assert!(
                w.offset >= face_a_bytes,
                "unchanged Square A face slot was rewritten at offset {}",
                w.offset
            );
        }
        for w in &delta.edge_writes {
            assert!(
                w.offset >= edges_a_bytes,
                "unchanged Square A edge slot was rewritten at offset {}",
                w.offset
            );
        }
        assert!(!delta.face_writes.is_empty());
        assert!(!delta.edge_writes.is_empty());
    }

    #[test]
    fn erasure_zeroes_slots_and_subsequent_creation_reuses_slots() {
        let mut history = History::new();
        history
            .execute(Command::AddPolyline {
                points: vec![
                    DVec3::new(0.0, 0.0, 0.0),
                    DVec3::new(2.0, 0.0, 0.0),
                    DVec3::new(2.0, 2.0, 0.0),
                    DVec3::new(0.0, 2.0, 0.0),
                ],
                closed: true,
            })
            .unwrap();
        let initial_face = *history.model().faces().keys().next().unwrap();
        let edge_to_erase = *history.model().edges().keys().next().unwrap();
        let erased_endpoints = {
            let e = &history.model().edges()[&edge_to_erase];
            (
                history.model().vertices()[&e.vertices[0]].position,
                history.model().vertices()[&e.vertices[1]].position,
            )
        };

        let (mut cache, init) = MeshCache::rebuild(history.model()).unwrap();
        assert_eq!(init.face_vertices, 6);
        assert_eq!(init.edge_vertices, 8);

        // Erase one edge so the face disappears.
        history
            .execute(Command::EraseEdges(vec![edge_to_erase]))
            .unwrap();
        assert!(history.model().faces().is_empty());
        assert_eq!(history.model().edges().len(), 3);

        let erase_update = cache
            .update(history.model(), &[initial_face], &[edge_to_erase])
            .unwrap();
        assert!(!erase_update.reset);
        assert_eq!(erase_update.face_writes.len(), 1);
        assert_eq!(erase_update.face_writes[0].offset, 0);
        assert_eq!(
            decode_face_vertices(&erase_update.face_writes[0].bytes),
            &[FaceVertex::zeroed(); 6]
        );
        assert_eq!(erase_update.edge_writes.len(), 1);
        let freed_edge_offset = erase_update.edge_writes[0].offset;
        assert_eq!(
            &decode_edge_vertices(&erase_update.edge_writes[0].bytes)[..2],
            &[EdgeVertex::zeroed(); 2]
        );

        assert!(
            decode_edge_vertices(&erase_update.edge_writes[0].bytes)[2..]
                .iter()
                .all(|v| v.normal_a == [0.; 3] && v.normal_b == [0.; 3])
        );

        // Re-close the square; the new face and edge must reuse the freed slots at the same offsets.
        history
            .execute(Command::AddEdge {
                start: erased_endpoints.0,
                end: erased_endpoints.1,
            })
            .unwrap();
        let new_face = *history.model().faces().keys().next().unwrap();
        let new_edge = *history
            .model()
            .edges()
            .keys()
            .find(|id| !cache.edge_slots.contains_key(id))
            .unwrap();

        let reuse_update = cache
            .update(history.model(), &[new_face], &[new_edge])
            .unwrap();
        assert_eq!(reuse_update.face_vertices, 6);
        assert_eq!(reuse_update.edge_vertices, 8);
        assert_eq!(reuse_update.face_writes.len(), 1);
        assert_eq!(reuse_update.face_writes[0].offset, 0);
        assert_eq!(reuse_update.edge_writes.len(), 1);
        assert_eq!(reuse_update.edge_writes[0].offset, freed_edge_offset);
    }

    #[test]
    fn power_of_two_face_slot_padding_and_reuse() {
        let mut history = History::new();
        // Pentagon: 5 vertices -> 3 triangles -> power-of-two slot capacity = 4 triangles (12 vertices).
        let pentagon: Vec<DVec3> = (0..5)
            .map(|i| {
                let angle = (i as f64) * std::f64::consts::TAU / 5.0;
                DVec3::new(angle.cos() * 3.0, angle.sin() * 3.0, 0.0)
            })
            .collect();
        history
            .execute(Command::AddPolyline {
                points: pentagon,
                closed: true,
            })
            .unwrap();
        let pent_face = *history.model().faces().keys().next().unwrap();
        let pent_edges: Vec<EdgeId> = history.model().edges().keys().copied().collect();

        let (mut cache, init) = MeshCache::rebuild(history.model()).unwrap();
        assert_eq!(init.face_vertices, 12); // 4-triangle power-of-two slot
        assert_eq!(init.edge_vertices, 10); // 5 edges
        let verts = decode_face_vertices(&init.face_writes[0].bytes);
        // First 9 vertices (3 triangles) are active; last 3 vertices (1 padding triangle) are zeroed.
        assert!(verts[..9].iter().all(|v| v.color == FACE_COLOR));
        assert_eq!(&verts[9..12], &[FaceVertex::zeroed(); 3]);

        // Erase pentagon and add a hexagon (6 vertices -> 4 triangles -> capacity 4).
        // The hexagon must reuse the 4-triangle slot at offset 0 without growing face_vertices!
        history
            .execute(Command::EraseEdges(pent_edges.clone()))
            .unwrap();
        let _ = cache
            .update(history.model(), &[pent_face], &pent_edges)
            .unwrap();

        let hexagon: Vec<DVec3> = (0..6)
            .map(|i| {
                let angle = (i as f64) * std::f64::consts::TAU / 6.0;
                DVec3::new(angle.cos() * 4.0, angle.sin() * 4.0, 0.0)
            })
            .collect();
        history
            .execute(Command::AddPolyline {
                points: hexagon,
                closed: true,
            })
            .unwrap();
        let hex_face = *history.model().faces().keys().next().unwrap();
        let hex_edges: Vec<EdgeId> = history.model().edges().keys().copied().collect();

        let hex_update = cache
            .update(history.model(), &[hex_face], &hex_edges)
            .unwrap();
        assert_eq!(hex_update.face_vertices, 12);
        assert_eq!(hex_update.face_writes.len(), 1);
        assert_eq!(hex_update.face_writes[0].offset, 0);
        let hex_verts = decode_face_vertices(&hex_update.face_writes[0].bytes);
        assert!(hex_verts.iter().all(|v| v.color == FACE_COLOR));
    }

    #[test]
    fn preview_compatible_open_polyline_and_tilted_face() {
        let mut history = History::new();
        // Open polyline in 3D (like a tool preview or tessellated arc).
        history
            .execute(Command::AddPolyline {
                points: vec![
                    DVec3::new(0.0, 0.0, 1.0),
                    DVec3::new(2.0, 1.0, 2.0),
                    DVec3::new(4.0, 0.0, 3.0),
                    DVec3::new(5.0, -1.0, 3.5),
                ],
                closed: false,
            })
            .unwrap();
        let (mut cache, init) = MeshCache::rebuild(history.model()).unwrap();
        assert_eq!(init.face_vertices, 0);
        assert!(init.face_writes.is_empty());
        assert_eq!(init.edge_vertices, 6);
        let edge_verts = decode_edge_vertices(&init.edge_writes[0].bytes);
        assert!(edge_verts.iter().all(
            |v| v.position.iter().all(|c| c.is_finite()) && v.color.map(f32::abs) == EDGE_COLOR
        ));

        // Add a tilted triangle in 3D and update incrementally.
        let prev_edges: BTreeSet<EdgeId> = history.model().edges().keys().copied().collect();
        history
            .execute(Command::AddPolyline {
                points: vec![
                    DVec3::new(0.0, 0.0, 0.0),
                    DVec3::new(2.0, 0.0, 1.0),
                    DVec3::new(0.0, 2.0, 1.0),
                ],
                closed: true,
            })
            .unwrap();
        let new_faces: Vec<FaceId> = history.model().faces().keys().copied().collect();
        let new_edges: Vec<EdgeId> = history
            .model()
            .edges()
            .keys()
            .copied()
            .filter(|id| !prev_edges.contains(id))
            .collect();
        let delta = cache
            .update(history.model(), &new_faces, &new_edges)
            .unwrap();
        assert_eq!(delta.face_vertices, 3);
        assert_eq!(delta.edge_vertices, 12);
        let tri_verts = decode_face_vertices(&delta.face_writes[0].bytes);
        assert_eq!(tri_verts.len(), 3);
        for v in tri_verts {
            let n = Vec3::from_array(v.normal);
            assert!((n.length() - 1.0).abs() < 1e-5);
            assert!(v.position.iter().all(|c| c.is_finite()));
        }
    }

    #[test]
    fn simulated_gpu_buffer_tracks_incremental_splits_erasure_and_buddy_slot_reuse() {
        fn apply_writes(
            buf: &mut Vec<u8>,
            writes: &[BufferWrite],
            total_bytes: usize,
            reset: bool,
        ) {
            if reset {
                buf.clear();
            }
            if buf.len() < total_bytes {
                buf.resize(total_bytes, 0);
            }
            for w in writes {
                let start = w.offset as usize;
                let end = start + w.bytes.len();
                if buf.len() < end {
                    buf.resize(end, 0);
                }
                buf[start..end].copy_from_slice(&w.bytes);
            }
            buf.truncate(total_bytes);
        }

        let mut history = History::new();
        // Start with a 10x10 outer square with a 4x4 inner hole (annulus = 8 tris, inner = 2 tris).
        history
            .execute(Command::AddPolyline {
                points: vec![
                    DVec3::new(0.0, 0.0, 0.0),
                    DVec3::new(10.0, 0.0, 0.0),
                    DVec3::new(10.0, 10.0, 0.0),
                    DVec3::new(0.0, 10.0, 0.0),
                ],
                closed: true,
            })
            .unwrap();
        history
            .execute(Command::AddPolyline {
                points: vec![
                    DVec3::new(3.0, 3.0, 0.0),
                    DVec3::new(7.0, 3.0, 0.0),
                    DVec3::new(7.0, 7.0, 0.0),
                    DVec3::new(3.0, 7.0, 0.0),
                ],
                closed: true,
            })
            .unwrap();

        let (mut cache, init) = MeshCache::rebuild(history.model()).unwrap();
        let mut face_buf = Vec::new();
        let mut edge_buf = Vec::new();
        apply_writes(
            &mut face_buf,
            &init.face_writes,
            (init.face_vertices as usize) * (FACE_VERTEX_BYTES as usize),
            init.reset,
        );
        apply_writes(
            &mut edge_buf,
            &init.edge_writes,
            (init.edge_vertices as usize) * (EDGE_VERTEX_BYTES as usize),
            init.reset,
        );

        // Erase the entire outer loop so the 8-triangle annulus slot is freed.
        let prev_faces: BTreeSet<FaceId> = history.model().faces().keys().copied().collect();
        let prev_edges: BTreeSet<EdgeId> = history.model().edges().keys().copied().collect();
        let outer_edges: Vec<EdgeId> = history
            .model()
            .edges()
            .iter()
            .filter_map(|(&id, e)| {
                let p = history.model().vertices()[&e.vertices[0]].position;
                (p.x == 0.0 || p.x == 10.0 || p.y == 0.0 || p.y == 10.0).then_some(id)
            })
            .collect();
        history.execute(Command::EraseEdges(outer_edges)).unwrap();

        let all_f: Vec<FaceId> = prev_faces
            .union(&history.model().faces().keys().copied().collect())
            .copied()
            .collect();
        let all_e: Vec<EdgeId> = prev_edges
            .union(&history.model().edges().keys().copied().collect())
            .copied()
            .collect();
        let u1 = cache.update(history.model(), &all_f, &all_e).unwrap();
        apply_writes(
            &mut face_buf,
            &u1.face_writes,
            (u1.face_vertices as usize) * (FACE_VERTEX_BYTES as usize),
            u1.reset,
        );
        apply_writes(
            &mut edge_buf,
            &u1.edge_writes,
            (u1.edge_vertices as usize) * (EDGE_VERTEX_BYTES as usize),
            u1.reset,
        );

        let active_area_1: f32 = decode_face_vertices(&face_buf)
            .as_chunks::<3>()
            .0
            .iter()
            .map(triangle_area)
            .sum();
        assert!((active_area_1 - 16.0).abs() < 1e-3);

        // Add two new 2x2 squares (each 2 triangles); buddy splitting reuses the freed 8-triangle slot!
        let before_faces: BTreeSet<FaceId> = history.model().faces().keys().copied().collect();
        let before_edges: BTreeSet<EdgeId> = history.model().edges().keys().copied().collect();
        for x0 in [20.0, 30.0] {
            history
                .execute(Command::AddPolyline {
                    points: vec![
                        DVec3::new(x0, 0.0, 0.0),
                        DVec3::new(x0 + 2.0, 0.0, 0.0),
                        DVec3::new(x0 + 2.0, 2.0, 0.0),
                        DVec3::new(x0, 2.0, 0.0),
                    ],
                    closed: true,
                })
                .unwrap();
        }
        let added_f: Vec<FaceId> = history
            .model()
            .faces()
            .keys()
            .copied()
            .filter(|id| !before_faces.contains(id))
            .collect();
        let added_e: Vec<EdgeId> = history
            .model()
            .edges()
            .keys()
            .copied()
            .filter(|id| !before_edges.contains(id))
            .collect();
        let u2 = cache.update(history.model(), &added_f, &added_e).unwrap();
        assert_eq!(u2.face_vertices, init.face_vertices);
        apply_writes(
            &mut face_buf,
            &u2.face_writes,
            (u2.face_vertices as usize) * (FACE_VERTEX_BYTES as usize),
            u2.reset,
        );
        apply_writes(
            &mut edge_buf,
            &u2.edge_writes,
            (u2.edge_vertices as usize) * (EDGE_VERTEX_BYTES as usize),
            u2.reset,
        );

        // Total area must be 16 (inner 4x4) + 4 + 4 = 24.0.
        let active_area_2: f32 = decode_face_vertices(&face_buf)
            .as_chunks::<3>()
            .0
            .iter()
            .map(triangle_area)
            .sum();
        assert!((active_area_2 - 24.0).abs() < 1e-3);
        let active_edges_2 = decode_edge_vertices(&edge_buf)
            .as_chunks::<2>()
            .0
            .iter()
            .filter(|pair| pair[0] != EdgeVertex::zeroed())
            .count();
        assert_eq!(active_edges_2, 12);
    }

    #[test]
    fn face_vertex_layout_and_zeroed_slot_normal_distinction() {
        assert_eq!(std::mem::size_of::<FaceVertex>(), 44);
        assert_eq!(FACE_VERTEX_BYTES, 44);

        let zeroed = FaceVertex::zeroed();
        assert_eq!(zeroed.position, [0.0; 3]);
        assert_eq!(zeroed.normal, [0.0; 3]);
        assert_eq!(zeroed.color, [0.0; 4]);
        assert_eq!(zeroed.material, 0.0);

        // Zeroed slot detection uses dot(normal, normal) == 0.0
        let zero_normal_sq =
            zeroed.normal[0].powi(2) + zeroed.normal[1].powi(2) + zeroed.normal[2].powi(2);
        assert_eq!(zero_normal_sq, 0.0);

        // A black face is valid: normal is unit length, color is black [0, 0, 0, 1]
        let black_face = FaceVertex {
            position: [1.0, 2.0, 3.0],
            normal: [0.0, 0.0, 1.0],
            color: [0.0, 0.0, 0.0, 1.0],
            material: 1.0,
        };
        let black_normal_sq = black_face.normal[0].powi(2)
            + black_face.normal[1].powi(2)
            + black_face.normal[2].powi(2);
        assert!((black_normal_sq - 1.0).abs() < 1e-6);
        assert!(black_normal_sq > 0.5);
    }

    #[test]
    fn styled_rebuild_and_update_with_explicit_material_and_color() {
        let mut history = History::new();
        history
            .execute(Command::AddPolyline {
                points: vec![
                    DVec3::new(0.0, 0.0, 0.0),
                    DVec3::new(2.0, 0.0, 0.0),
                    DVec3::new(2.0, 2.0, 0.0),
                    DVec3::new(0.0, 2.0, 0.0),
                ],
                closed: true,
            })
            .unwrap();
        let face_id = *history.model().faces().keys().next().unwrap();

        // 1. Rebuild with styled face: custom blue color and explicit material = true
        let custom_style = FaceStyle {
            color: [0.1, 0.2, 0.9, 0.8],
            explicit_material: true,
            texture_layer: 0,
        };
        let (mut cache, init) =
            MeshCache::rebuild_styled(history.model(), &|_id, _f| custom_style).unwrap();
        assert_eq!(init.face_vertices, 6);
        let verts = decode_face_vertices(&init.face_writes[0].bytes);
        assert_eq!(verts.len(), 6);
        for v in verts {
            assert_eq!(v.color, [0.1, 0.2, 0.9, 0.8]);
            assert_eq!(v.material, 1.0);
        }

        // 2. Calling update_styled with same style is a no-op
        let noop = cache
            .update_styled(history.model(), &[face_id], &[], &|_id, _f| custom_style)
            .unwrap();
        assert!(noop.face_writes.is_empty());

        // 3. Calling update_styled with changed style forces re-triangulation/update of the slot
        let new_style = FaceStyle {
            color: [0.9, 0.1, 0.1, 1.0],
            explicit_material: false,
            texture_layer: 0,
        };
        let delta = cache
            .update_styled(history.model(), &[face_id], &[], &|_id, _f| new_style)
            .unwrap();
        assert_eq!(delta.face_writes.len(), 1);
        let updated_verts = decode_face_vertices(&delta.face_writes[0].bytes);
        assert_eq!(updated_verts.len(), 6);
        for v in updated_verts {
            assert_eq!(v.color, [0.9, 0.1, 0.1, 1.0]);
            assert_eq!(v.material, 0.0);
        }
    }

    #[test]
    fn styled_rebuild_and_update_with_texture_layer() {
        let mut history = History::new();
        history
            .execute(Command::AddPolyline {
                points: vec![
                    DVec3::new(0.0, 0.0, 0.0),
                    DVec3::new(2.0, 0.0, 0.0),
                    DVec3::new(2.0, 2.0, 0.0),
                    DVec3::new(0.0, 2.0, 0.0),
                ],
                closed: true,
            })
            .unwrap();
        let face_id = *history.model().faces().keys().next().unwrap();

        // Layer 5, explicit = true -> packed as 5 * 2 + 1 = 11.0
        let style_tex5 = FaceStyle {
            color: [0.5, 0.5, 0.5, 1.0],
            explicit_material: true,
            texture_layer: 5,
        };
        let (mut cache, init) =
            MeshCache::rebuild_styled(history.model(), &|_id, _f| style_tex5).unwrap();
        let verts = decode_face_vertices(&init.face_writes[0].bytes);
        for v in verts {
            assert_eq!(v.material, 11.0);
        }

        // Updating with changed texture layer 7 (explicit = false -> 7 * 2 = 14.0) updates the slot
        let style_tex7 = FaceStyle {
            color: [0.5, 0.5, 0.5, 1.0],
            explicit_material: false,
            texture_layer: 7,
        };
        let delta = cache
            .update_styled(history.model(), &[face_id], &[], &|_id, _f| style_tex7)
            .unwrap();
        assert_eq!(delta.face_writes.len(), 1);
        let updated_verts = decode_face_vertices(&delta.face_writes[0].bytes);
        for v in updated_verts {
            assert_eq!(v.material, 14.0);
        }
    }
}

#[cfg(test)]
mod profile_tests {
    use super::*;
    use cindral_core::{Command, History};
    #[test]
    fn face_adjacency_changes_refresh_existing_profile_edges_and_undo() {
        let mut history = History::new();
        let square = |x: f64| Command::AddPolyline {
            points: vec![
                DVec3::new(x, 0., 0.),
                DVec3::new(x + 1., 0., 0.),
                DVec3::new(x + 1., 1., 0.),
                DVec3::new(x, 1., 0.),
            ],
            closed: true,
        };
        history.execute(square(0.)).unwrap();
        let shared = *history
            .model()
            .edges()
            .iter()
            .find(|(_, edge)| {
                edge.vertices
                    .iter()
                    .all(|v| history.model().vertices()[v].position.x == 1.)
            })
            .unwrap()
            .0;
        let (mut cache, _) = MeshCache::rebuild(history.model()).unwrap();
        assert!(cache.edge_slots[&shared].vertices[0].color[2] < 0.);
        history.execute(square(1.)).unwrap();
        let change = history.changes();
        cache
            .update(
                history.model(),
                &change.faces.iter().copied().collect::<Vec<_>>(),
                &change.edges.iter().copied().collect::<Vec<_>>(),
            )
            .unwrap();
        let edge = cache.edge_slots[&shared].vertices[0];
        assert!(
            edge.color[2] > 0.,
            "shared coplanar edge is no longer a boundary profile"
        );
        assert!(
            glam::Vec3::from_array(edge.normal_a).dot(glam::Vec3::from_array(edge.normal_b)) > 0.99
        );
        history.undo().unwrap();
        let change = history.changes();
        cache
            .update(
                history.model(),
                &change.faces.iter().copied().collect::<Vec<_>>(),
                &change.edges.iter().copied().collect::<Vec<_>>(),
            )
            .unwrap();
        assert!(cache.edge_slots[&shared].vertices[0].color[2] < 0.);
    }
}
