//! Definition-scoped CPU caches and scene batching, independent of a GPU device.
use cindral_core::{DefinitionId, DocumentHistory, Face, FaceId, KernelError, Occurrence};
use cindral_render::{
    mesh::{FaceStyle, MeshCache},
    scene::{DefinitionUpdate, InstanceBatch, InstanceData, SceneUpdate},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
/// Cached geometry is stored once per definition, regardless of occurrence count.
#[derive(Default)]
pub(crate) struct SceneCache {
    meshes: BTreeMap<DefinitionId, MeshCache>,
    occurrences: Vec<Occurrence>,
    initialized: bool,
    baselines: BTreeMap<DefinitionId, glam::DMat4>,
    placements: BTreeMap<DefinitionId, glam::DMat4>,
    revision: Option<u64>,
    texture_layers: BTreeMap<String, u32>,
    textures: Option<Arc<Vec<cindral_render::texture::TexturePixels>>>,
}
fn style(
    doc: &DocumentHistory,
    _id: FaceId,
    face: &Face,
    layers: &BTreeMap<String, u32>,
) -> FaceStyle {
    if let Some(material) = face.material.and_then(|id| doc.materials().get(&id)) {
        return FaceStyle {
            color: [
                material.color[0],
                material.color[1],
                material.color[2],
                material.opacity,
            ],
            explicit_material: true,
            texture_layer: material
                .texture
                .as_ref()
                .and_then(|key| layers.get(key))
                .copied()
                .unwrap_or_default(),
        };
    }
    let color = if face.color == [160, 177, 193, 255] {
        [0.58, 0.63, 0.69, 1.]
    } else {
        [
            face.color[0] as f32 / 255.,
            face.color[1] as f32 / 255.,
            face.color[2] as f32 / 255.,
            face.color[3] as f32 / 255.,
        ]
    };
    FaceStyle {
        color,
        explicit_material: face.color != [160, 177, 193, 255],
        texture_layer: 0,
    }
}
impl SceneCache {
    pub(crate) fn set_textures(
        &mut self,
        layers: BTreeMap<String, u32>,
        pixels: Vec<cindral_render::texture::TexturePixels>,
    ) {
        self.texture_layers = layers;
        self.textures = Some(Arc::new(pixels));
        self.revision = None;
    }
    pub(crate) fn definitions(&self) -> Vec<DefinitionId> {
        self.meshes.keys().copied().collect()
    }
    pub(crate) fn occurrences(&self) -> &[Occurrence] {
        &self.occurrences
    }
    #[tracing::instrument(level = "info", skip_all, name = "mesh.scene_sync")]
    pub(crate) fn sync(&mut self, doc: &DocumentHistory) -> Result<SceneUpdate, KernelError> {
        let changes = doc.document_changes();
        if self.initialized && self.revision == Some(changes.revision) {
            return Ok(SceneUpdate::default());
        }
        let textures_changed = self.textures.is_some();
        let mut translated = false;
        let instances = !self.initialized || changes.instances;
        if instances {
            self.occurrences = doc.occurrences()?;
        }
        let visible: BTreeSet<_> = self.occurrences.iter().map(|o| o.definition).collect();
        let mut update = SceneUpdate {
            textures: self.textures.take(),
            ..Default::default()
        };
        let removed = self
            .meshes
            .keys()
            .filter(|id| !visible.contains(id) || doc.definition_model(**id).is_none())
            .copied()
            .collect::<Vec<_>>();
        for id in &removed {
            self.meshes.remove(id);
            self.baselines.remove(id);
            self.placements.remove(id);
        }
        update.removed = removed;
        for id in &visible {
            let model = doc
                .definition_model(*id)
                .ok_or(KernelError::MissingEntity)?;
            if let std::collections::btree_map::Entry::Vacant(entry) = self.meshes.entry(*id) {
                let (cache, mesh) = MeshCache::rebuild_styled(model, &|face_id, face| {
                    style(doc, face_id, face, &self.texture_layers)
                })?;
                entry.insert(cache);
                self.baselines.insert(*id, model.geometry_frame());
                self.placements.insert(*id, glam::DMat4::IDENTITY);
                update.geometry.push(DefinitionUpdate {
                    definition: *id,
                    mesh: Arc::new(mesh),
                });
                continue;
            }
            let placement = model.geometry_frame()
                * self
                    .baselines
                    .get(id)
                    .copied()
                    .unwrap_or(glam::DMat4::IDENTITY)
                    .inverse();
            translated |= self.placements.insert(*id, placement) != Some(placement);
            let offset = placement.w_axis.truncate();
            let dirty = changes.geometry.get(id);
            if dirty.is_some_and(|d| d.rigid_translation)
                && !changes.appearance
                && !textures_changed
            {
                continue;
            }
            if dirty.is_some() || changes.appearance || textures_changed {
                if placement.x_axis != glam::DVec4::X
                    || placement.y_axis != glam::DVec4::Y
                    || placement.z_axis != glam::DVec4::Z
                {
                    let (cache, mesh) = MeshCache::rebuild_styled(model, &|face_id, face| {
                        style(doc, face_id, face, &self.texture_layers)
                    })?;
                    self.meshes.insert(*id, cache);
                    self.baselines.insert(*id, model.geometry_frame());
                    self.placements.insert(*id, glam::DMat4::IDENTITY);
                    translated = true;
                    update.geometry.push(DefinitionUpdate {
                        definition: *id,
                        mesh: Arc::new(mesh),
                    });
                    continue;
                }
                let mut faces = dirty.map(|d| d.faces.clone()).unwrap_or_default();
                let edges = dirty
                    .map(|d| d.edges.iter().copied().collect::<Vec<_>>())
                    .unwrap_or_default();
                if changes.appearance || textures_changed {
                    faces.extend(
                        model
                            .faces()
                            .iter()
                            .filter_map(|(id, f)| f.material.map(|_| *id)),
                    );
                }
                let mesh = self
                    .meshes
                    .get_mut(id)
                    .ok_or(KernelError::MissingEntity)?
                    .update_styled_in_frame(
                        model,
                        &faces.into_iter().collect::<Vec<_>>(),
                        &edges,
                        offset,
                        &|face_id, face| style(doc, face_id, face, &self.texture_layers),
                    )?;
                update.geometry.push(DefinitionUpdate {
                    definition: *id,
                    mesh: Arc::new(mesh),
                });
            }
        }
        if instances || changes.appearance || translated || textures_changed {
            let mut batches: BTreeMap<DefinitionId, Vec<InstanceData>> = BTreeMap::new();
            for item in &self.occurrences {
                let world = (item.world
                    * self
                        .placements
                        .get(&item.definition)
                        .copied()
                        .unwrap_or(glam::DMat4::IDENTITY))
                .as_mat4();
                if !world.is_finite() {
                    return Err(KernelError::InvalidPosition);
                }
                let material = item
                    .material
                    .and_then(|id| doc.materials().get(&id))
                    .map(|m| [m.color[0], m.color[1], m.color[2], m.opacity]);
                let dim = if item.path.starts_with(doc.context()) {
                    1.
                } else {
                    0.25
                };
                batches.entry(item.definition).or_default().push(
                    InstanceData::new(world, material, dim).with_texture(
                        item.material
                            .and_then(|id| doc.materials().get(&id))
                            .and_then(|m| m.texture.as_ref())
                            .and_then(|key| self.texture_layers.get(key))
                            .copied()
                            .unwrap_or_default(),
                    ),
                );
            }
            for id in self.meshes.keys() {
                batches.entry(*id).or_default();
            }
            update.batches = Some(
                batches
                    .into_iter()
                    .map(|(definition, instances)| InstanceBatch {
                        definition,
                        instances,
                    })
                    .collect(),
            );
        }
        self.initialized = true;
        self.revision = Some(changes.revision);
        Ok(update)
    }
}
/// Merge ordered updates queued before the next egui GPU callback.
pub(crate) fn merge(pending: &mut Option<Arc<SceneUpdate>>, mut next: SceneUpdate) {
    if let Some(previous) = pending {
        let previous = Arc::make_mut(previous);
        for id in &next.removed {
            previous.geometry.retain(|g| g.definition != *id);
            if !previous.removed.contains(id) {
                previous.removed.push(*id);
            }
        }
        for definition in next.geometry.drain(..) {
            previous.removed.retain(|id| *id != definition.definition);
            if let Some(existing) = previous
                .geometry
                .iter_mut()
                .find(|g| g.definition == definition.definition)
            {
                if definition.mesh.reset {
                    existing.mesh = definition.mesh;
                } else {
                    let target = Arc::make_mut(&mut existing.mesh);
                    target
                        .face_writes
                        .extend(definition.mesh.face_writes.clone());
                    target
                        .edge_writes
                        .extend(definition.mesh.edge_writes.clone());
                    target.face_vertices = definition.mesh.face_vertices;
                    target.edge_vertices = definition.mesh.edge_vertices;
                }
            } else {
                previous.geometry.push(definition);
            }
        }
        if next.textures.is_some() {
            previous.textures = next.textures;
        }
        if next.batches.is_some() {
            previous.batches = next.batches;
        }
    } else {
        *pending = Some(Arc::new(next));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use cindral_core::*;
    #[test]
    fn shared_edit_uses_one_definition_cache_and_matrix_only_change_has_no_mesh_writes() {
        let mut doc = DocumentHistory::new();
        doc.execute(Command::AddPolyline {
            points: vec![DVec3::ZERO, DVec3::X, DVec3::X + DVec3::Y, DVec3::Y],
            closed: true,
        })
        .unwrap();
        doc.organization(OrganizationCommand::Create {
            kind: DefinitionKind::Component,
            name: "Part".into(),
            vertices: doc.model().vertices().keys().copied().collect(),
        })
        .unwrap();
        let node = doc
            .instances(DefinitionId(0))
            .unwrap()
            .values()
            .next()
            .unwrap()
            .clone();
        for x in [3., 6.] {
            doc.organization(OrganizationCommand::AddInstance {
                definition: node.definition,
                name: "Copy".into(),
                transform: glam::DMat4::from_translation(DVec3::X * x),
            })
            .unwrap();
        }
        let mut cache = SceneCache::default();
        let update = cache.sync(&doc).unwrap();
        assert_eq!(update.geometry.len(), 2);
        assert_eq!(
            update
                .batches
                .unwrap()
                .iter()
                .find(|b| b.definition == node.definition)
                .unwrap()
                .instances
                .len(),
            3
        );
        doc.enter(vec![node.id]).unwrap();
        cache.sync(&doc).unwrap();
        let face = *doc.model().faces().keys().next().unwrap();
        doc.execute(Command::PushPull { face, distance: 1. })
            .unwrap();
        let update = cache.sync(&doc).unwrap();
        assert_eq!(update.geometry.len(), 1);
        assert!(update.batches.is_none());
        assert_eq!(update.geometry[0].definition, node.definition);
        doc.organization(OrganizationCommand::Transform {
            path: vec![node.id],
            transform: glam::DMat4::from_translation(DVec3::Z),
        })
        .unwrap();
        let update = cache.sync(&doc).unwrap();
        assert!(update.geometry.is_empty());
        assert!(update.batches.is_some());
        let paths = doc
            .instances(DefinitionId(0))
            .unwrap()
            .keys()
            .map(|id| vec![*id])
            .collect::<Vec<_>>();
        for path in paths {
            doc.organization(OrganizationCommand::Hide { path, hidden: true })
                .unwrap();
        }
        let hidden = cache.sync(&doc).unwrap();
        assert!(hidden.removed.contains(&node.definition));
        doc.organization(OrganizationCommand::Hide {
            path: vec![node.id],
            hidden: false,
        })
        .unwrap();
        let shown = cache.sync(&doc).unwrap();
        assert!(shown
            .geometry
            .iter()
            .any(|g| g.definition == node.definition && g.mesh.reset));
    }
    #[test]
    fn raw_translation_keeps_mesh_and_later_face_edits_use_the_same_render_frame() {
        let mut doc = DocumentHistory::new();
        doc.execute(Command::AddPolyline {
            points: vec![DVec3::ZERO, DVec3::X, DVec3::X + DVec3::Y, DVec3::Y],
            closed: true,
        })
        .unwrap();
        let mut cache = SceneCache::default();
        cache.sync(&doc).unwrap();
        let vertices = doc.model().vertices().keys().copied().collect();
        doc.preview(Command::Transform {
            vertices,
            matrix: glam::DMat4::from_translation(DVec3::X * 3.),
            copy: false,
        })
        .unwrap();
        let update = cache.sync(&doc).unwrap();
        assert!(update.geometry.is_empty());
        let batch = update.batches.unwrap();
        assert_eq!(batch[0].instances[0].model[3][0], 3.);
        assert_eq!(doc.model().bounds().unwrap().0.x, 3.);
        doc.commit_preview().unwrap();
        doc.execute(Command::AddEdge {
            start: DVec3::X * 3.,
            end: DVec3::new(4., 1., 0.),
        })
        .unwrap();
        let update = cache.sync(&doc).unwrap();
        assert!(update.geometry.iter().all(|g| !g.mesh.reset));
        let moved_vertices = update
            .geometry
            .iter()
            .flat_map(|g| &g.mesh.face_writes)
            .flat_map(|w| {
                w.bytes
                    .as_chunks::<{ std::mem::size_of::<cindral_render::mesh::FaceVertex>() }>()
                    .0
                    .iter()
            });
        for v in moved_vertices {
            let x = f32::from_ne_bytes(v[..4].try_into().unwrap());
            assert!((0.0..=1.).contains(&x));
        }
        doc.undo().unwrap();
        cache.sync(&doc).unwrap();
        doc.undo().unwrap();
        let update = cache.sync(&doc).unwrap();
        assert!(update.geometry.is_empty());
        assert_eq!(update.batches.unwrap()[0].instances[0].model[3][0], 0.);
    }
    #[test]
    fn changing_texture_table_updates_material_slots_and_queued_assets() {
        let mut doc = DocumentHistory::new();
        doc.execute(Command::AddPolyline {
            points: vec![DVec3::ZERO, DVec3::X, DVec3::X + DVec3::Y, DVec3::Y],
            closed: true,
        })
        .unwrap();
        let material = MaterialId(doc.to_data().next_id);
        doc.organization(OrganizationCommand::CreateMaterial(Material {
            name: "Textured".into(),
            color: [1.; 3],
            opacity: 1.,
            texture: Some("asset".into()),
        }))
        .unwrap();
        doc.organization(OrganizationCommand::PaintFaces {
            faces: doc.model().faces().keys().copied().collect(),
            material: Some(material),
        })
        .unwrap();
        let mut cache = SceneCache::default();
        cache.sync(&doc).unwrap();
        cache.set_textures(
            BTreeMap::from([("asset".into(), 1)]),
            vec![cindral_render::texture::TexturePixels::new(
                1,
                1,
                vec![20, 40, 60, 128],
            )],
        );
        let update = cache.sync(&doc).unwrap();
        assert_eq!(update.textures.as_ref().unwrap().len(), 1);
        let bytes = &update.geometry[0].mesh.face_writes[0].bytes;
        assert_eq!(f32::from_ne_bytes(bytes[40..44].try_into().unwrap()), 3.);
        let mut pending = None;
        merge(&mut pending, update);
        cache.set_textures(BTreeMap::new(), Vec::new());
        merge(&mut pending, cache.sync(&doc).unwrap());
        assert!(pending.unwrap().textures.as_ref().unwrap().is_empty());
        assert!(cache.sync(&doc).unwrap().textures.is_none());
    }
}
