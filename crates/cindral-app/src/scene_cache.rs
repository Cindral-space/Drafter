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
}
fn style(doc: &DocumentHistory, _id: FaceId, face: &Face) -> FaceStyle {
    if let Some(material) = face.material.and_then(|id| doc.materials().get(&id)) {
        return FaceStyle {
            color: [
                material.color[0],
                material.color[1],
                material.color[2],
                material.opacity,
            ],
            explicit_material: true,
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
    }
}
impl SceneCache {
    pub(crate) fn definitions(&self) -> Vec<DefinitionId> {
        self.meshes.keys().copied().collect()
    }
    pub(crate) fn occurrences(&self) -> &[Occurrence] {
        &self.occurrences
    }
    pub(crate) fn sync(&mut self, doc: &DocumentHistory) -> Result<SceneUpdate, KernelError> {
        let changes = doc.document_changes();
        let instances = !self.initialized || changes.instances;
        if instances {
            self.occurrences = doc.occurrences()?;
        }
        let visible: BTreeSet<_> = self.occurrences.iter().map(|o| o.definition).collect();
        let mut update = SceneUpdate::default();
        let removed = self
            .meshes
            .keys()
            .filter(|id| !visible.contains(id) || doc.definition_model(**id).is_none())
            .copied()
            .collect::<Vec<_>>();
        for id in &removed {
            self.meshes.remove(id);
        }
        update.removed = removed;
        for id in &visible {
            let model = doc
                .definition_model(*id)
                .ok_or(KernelError::MissingEntity)?;
            if let std::collections::btree_map::Entry::Vacant(entry) = self.meshes.entry(*id) {
                let (cache, mesh) =
                    MeshCache::rebuild_styled(model, &|face_id, face| style(doc, face_id, face))?;
                entry.insert(cache);
                update.geometry.push(DefinitionUpdate {
                    definition: *id,
                    mesh: Arc::new(mesh),
                });
                continue;
            }
            let dirty = changes.geometry.get(id);
            if dirty.is_some() || changes.appearance {
                let mut faces = dirty.map(|d| d.faces.clone()).unwrap_or_default();
                let edges = dirty
                    .map(|d| d.edges.iter().copied().collect::<Vec<_>>())
                    .unwrap_or_default();
                if changes.appearance {
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
                    .update_styled(
                        model,
                        &faces.into_iter().collect::<Vec<_>>(),
                        &edges,
                        &|face_id, face| style(doc, face_id, face),
                    )?;
                update.geometry.push(DefinitionUpdate {
                    definition: *id,
                    mesh: Arc::new(mesh),
                });
            }
        }
        if instances || changes.appearance {
            let mut batches: BTreeMap<DefinitionId, Vec<InstanceData>> = BTreeMap::new();
            for item in &self.occurrences {
                let world = item.world.as_mat4();
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
                batches
                    .entry(item.definition)
                    .or_default()
                    .push(InstanceData::new(world, material, dim));
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
}
