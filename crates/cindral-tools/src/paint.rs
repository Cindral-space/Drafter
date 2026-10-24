//! SketchUp-style Paint Bucket scope resolution (pure model queries).
use cindral_core::{FaceId, MaterialId, Model};
use std::collections::BTreeSet;

/// Faces connected to `start` through shared edges whose explicit face
/// material matches `start`'s (fill adjacent). Missing faces yield nothing.
pub fn fill_adjacent_same_material(model: &Model, start: FaceId) -> Vec<FaceId> {
    let Some(material) = model.faces().get(&start).map(|f| f.material) else {
        return Vec::new();
    };
    let mut visited = BTreeSet::from([start]);
    let mut frontier = vec![start];
    while let Some(face) = frontier.pop() {
        for edge in model.face_edges(face) {
            for (neighbor, neighbor_face) in model.edge_faces(edge) {
                if neighbor_face.material == material && visited.insert(neighbor) {
                    frontier.push(neighbor);
                }
            }
        }
    }
    visited.into_iter().collect()
}

/// Every face whose explicit material equals `material` (replace scope).
pub fn faces_with_material(model: &Model, material: Option<MaterialId>) -> Vec<FaceId> {
    model
        .faces()
        .iter()
        .filter(|(_, face)| face.material == material)
        .map(|(id, _)| *id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cindral_core::{Command, DVec3, DocumentHistory, OrganizationCommand};

    /// Three-unit strip: [0,1]x[0,1], [1,2]x[0,1] and a separate [3,4]x[3,4] square.
    fn model() -> DocumentHistory {
        let mut h = DocumentHistory::new();
        for x in [0., 1.] {
            h.execute(Command::AddPolyline {
                points: vec![
                    DVec3::new(x, 0., 0.),
                    DVec3::new(x + 1., 0., 0.),
                    DVec3::new(x + 1., 1., 0.),
                    DVec3::new(x, 1., 0.),
                ],
                closed: true,
            })
            .unwrap();
        }
        h.execute(Command::AddPolyline {
            points: vec![
                DVec3::new(3., 3., 0.),
                DVec3::new(4., 3., 0.),
                DVec3::new(4., 4., 0.),
                DVec3::new(3., 4., 0.),
            ],
            closed: true,
        })
        .unwrap();
        h
    }
    fn face_containing(h: &DocumentHistory, point: DVec3) -> FaceId {
        *h.model()
            .faces()
            .iter()
            .find(|(_, f)| {
                let mut min = DVec3::splat(f64::INFINITY);
                let mut max = DVec3::splat(f64::NEG_INFINITY);
                for &v in &f.outer {
                    let p = h.model().vertices()[&v].position;
                    min = min.min(p);
                    max = max.max(p);
                }
                min.cmple(point).all() && max.cmpge(point).all()
            })
            .unwrap()
            .0
    }

    #[test]
    fn fill_adjacent_stops_at_material_boundaries_and_gaps() {
        let mut h = model();
        let a = face_containing(&h, DVec3::new(0.5, 0.5, 0.));
        let b = face_containing(&h, DVec3::new(1.5, 0.5, 0.));
        let far = face_containing(&h, DVec3::new(3.5, 3.5, 0.));
        h.organization(OrganizationCommand::CreateMaterial(
            cindral_core::Material {
                name: "M".into(),
                color: [1, 2, 3],
                opacity: 1.,
                texture: None,
            },
        ))
        .unwrap();
        let material = *h.materials().keys().next().unwrap();
        h.organization(OrganizationCommand::PaintFaces {
            faces: vec![b],
            material: Some(material),
        })
        .unwrap();
        // Painting face A fills only A: B carries a different material, far is disconnected.
        assert_eq!(fill_adjacent_same_material(h.model(), a), vec![a]);
        // From B (the painted one) the region stays B alone.
        assert_eq!(fill_adjacent_same_material(h.model(), b), vec![b]);
        // Clearing B's material makes the two-strip one fill-adjacent region.
        h.organization(OrganizationCommand::PaintFaces {
            faces: vec![b],
            material: None,
        })
        .unwrap();
        assert_eq!(
            fill_adjacent_same_material(h.model(), a),
            vec![a.min(b), a.max(b)]
        );
        assert_eq!(fill_adjacent_same_material(h.model(), far), vec![far]);
    }

    #[test]
    fn faces_with_material_lists_exact_members() {
        let mut h = model();
        let a = face_containing(&h, DVec3::new(0.5, 0.5, 0.));
        let b = face_containing(&h, DVec3::new(1.5, 0.5, 0.));
        let far = face_containing(&h, DVec3::new(3.5, 3.5, 0.));
        assert_eq!(faces_with_material(h.model(), None), vec![a, b, far]);
        h.organization(OrganizationCommand::CreateMaterial(
            cindral_core::Material {
                name: "M".into(),
                color: [1, 2, 3],
                opacity: 1.,
                texture: None,
            },
        ))
        .unwrap();
        let material = *h.materials().keys().next().unwrap();
        h.organization(OrganizationCommand::PaintFaces {
            faces: vec![a, far],
            material: Some(material),
        })
        .unwrap();
        assert_eq!(
            faces_with_material(h.model(), Some(material)),
            vec![a.min(far), a.max(far)]
        );
        assert_eq!(faces_with_material(h.model(), None), vec![b]);
    }
}
