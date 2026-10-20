//! Non-destructive half-space math and closed-shell cross-section triangulation.
use crate::{DVec3, KernelError, Model, SectionPlane, TOLERANCE};
use std::collections::{BTreeMap, BTreeSet};
impl SectionPlane {
    /// Construct a normalized world plane through a point.
    pub fn through(name: String, point: DVec3, normal: DVec3) -> Option<Self> {
        let normal = normal.try_normalize()?;
        point.is_finite().then(|| Self { name, normal: normal.to_array(), distance: normal.dot(point), visible: true })
    }
    /// Signed distance, positive on the retained side.
    pub fn signed_distance(&self, point: DVec3) -> f64 {
        DVec3::from_array(self.normal).dot(point) - self.distance
    }
    /// Whether a point survives this view cut.
    pub fn retains(&self, point: DVec3) -> bool { self.signed_distance(point) >= -TOLERANCE }
    /// Reverse the retained half-space without moving the widget.
    pub fn flip(&mut self) {
        self.normal = (-DVec3::from_array(self.normal)).to_array();
        self.distance = -self.distance;
    }
    /// Transform a world plane into a local affine frame, including mirrors/shears.
    pub fn in_frame(&self, local_to_world: glam::DMat4) -> Option<Self> {
        let equation = local_to_world.transpose() * DVec3::from_array(self.normal).extend(-self.distance);
        let length = equation.truncate().length();
        (length.is_finite() && length > 0.).then(|| Self {
            normal: (equation.truncate()/length).to_array(), distance: -equation.w/length, ..self.clone()
        })
    }
    /// Clip a line segment to the retained half-space.
    pub fn clip_segment(&self, mut a: DVec3, mut b: DVec3) -> Option<[DVec3; 2]> {
        let da = self.signed_distance(a); let db = self.signed_distance(b);
        if da < -TOLERANCE && db < -TOLERANCE { return None; }
        if (da < 0.) != (db < 0.) {
            let p = a + (b-a) * (da / (da-db));
            if da < 0. { a = p; } else { b = p; }
        }
        Some([a,b])
    }
}
/// Generate separate view-only cap triangles for a closed solid model.
/// Open/nonmanifold shells are deliberately not filled. Nested loops use even/odd
/// containment, preserving through holes and disconnected shells, independent of winding.
pub fn section_cap(model: &Model, plane: &SectionPlane) -> Result<Vec<[DVec3; 3]>, KernelError> {
    let mut incidence = BTreeMap::new();
    for face in model.faces().values() {
        for ring in std::iter::once(&face.outer).chain(&face.holes) {
            for i in 0..ring.len() {
                let mut edge = [ring[i],ring[(i+1)%ring.len()]]; edge.sort();
                *incidence.entry(edge).or_insert(0usize) += 1;
            }
        }
    }
    if incidence.is_empty() || incidence.values().any(|n| *n != 2) { return Ok(Vec::new()); }
    let n = DVec3::from_array(plane.normal);
    let u = n.any_orthonormal_vector(); let v = n.cross(u);
    let origin = n * plane.distance;
    let mut points: Vec<DVec3> = Vec::new();
    let mut buckets: BTreeMap<(i64,i64), Vec<usize>> = BTreeMap::new();
    let mut intern = |p: DVec3| {
        let relative = p-origin;
        let key = ((relative.dot(u)/TOLERANCE).floor() as i64,(relative.dot(v)/TOLERANCE).floor() as i64);
        for x in -1..=1 { for y in -1..=1 {
            if let Some(ids) = buckets.get(&(key.0+x,key.1+y)) {
                if let Some(id) = ids.iter().find(|id| points[**id].distance(p) <= TOLERANCE * 2.) { return *id; }
            }
        }}
        let id = points.len(); points.push(p); buckets.entry(key).or_default().push(id); id
    };
    let mut segments = BTreeSet::new();
    for id in model.faces().keys() {
        for tri in model.triangulate(*id)? {
            let d = tri.map(|p| plane.signed_distance(p));
            if d.iter().all(|d| d.abs() <= TOLERANCE) { continue; }
            let mut hits = Vec::new();
            for i in 0..3 {
                let j = (i+1)%3;
                if (d[i] < 0.) != (d[j] < 0.) {
                    let p = tri[i] + (tri[j]-tri[i]) * (d[i]/(d[i]-d[j]));
                    let id = intern(p); if !hits.contains(&id) { hits.push(id); }
                }
            }
            if hits.len() == 2 { let mut pair = [hits[0],hits[1]]; pair.sort(); segments.insert(pair); }
        }
    }
    let mut adjacency: BTreeMap<usize,Vec<usize>> = BTreeMap::new();
    for [a,b] in &segments { adjacency.entry(*a).or_default().push(*b); adjacency.entry(*b).or_default().push(*a); }
    if adjacency.values().any(|a| a.len()!=2) { return Ok(Vec::new()); }
    let mut loops = Vec::new();
    while let Some(&[start,next]) = segments.first() {
        let mut ring = vec![start]; let mut previous = start; let mut current = next;
        loop {
            let mut pair = [previous,current]; pair.sort(); segments.remove(&pair);
            if current == start { break; }
            ring.push(current);
            let neighbors = &adjacency[&current];
            let next = if neighbors[0] == previous { neighbors[1] } else { neighbors[0] };
            previous=current; current=next;
            if ring.len()>points.len() { return Ok(Vec::new()); }
        }
        if ring.len()>=3 { loops.push(ring); }
    }
    let projected: Vec<_> = points.iter().map(|p| { let p=*p-origin; glam::DVec2::new(p.dot(u),p.dot(v)) }).collect();
    let contains = |ring: &[usize], p: glam::DVec2| {
        let mut inside=false;
        for i in 0..ring.len() { let a=projected[ring[i]]; let b=projected[ring[(i+1)%ring.len()]];
            if (a.y>p.y)!=(b.y>p.y) && p.x<(b.x-a.x)*(p.y-a.y)/(b.y-a.y)+a.x { inside=!inside; }
        } inside
    };
    let containers: Vec<Vec<usize>> = loops.iter().enumerate().map(|(i,r)| loops.iter().enumerate().filter_map(|(j,other)| (i!=j && contains(other,projected[r[0]])).then_some(j)).collect()).collect();
    let mut result=Vec::new();
    for (i,ring) in loops.iter().enumerate() {
        if containers[i].len()%2!=0 { continue; }
        let mut ids=ring.clone(); let mut holes=Vec::new();
        for (j,hole) in loops.iter().enumerate() {
            if containers[j].len()==containers[i].len()+1 && containers[j].contains(&i) { holes.push(ids.len()); ids.extend(hole); }
        }
        let flat: Vec<_> = ids.iter().flat_map(|id| projected[*id].to_array()).collect();
        let indices = earcutr::earcut(&flat,&holes,2).map_err(|_| KernelError::Triangulation)?;
        for tri in indices.chunks_exact(3) {
            let mut t=[points[ids[tri[0]]],points[ids[tri[1]]],points[ids[tri[2]]]];
            if (t[1]-t[0]).cross(t[2]-t[0]).dot(n)>0. { t.swap(1,2); }
            result.push(t);
        }
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Command, History};
    fn cube() -> History {
        let mut h=History::new();
        h.execute(Command::AddPolyline { points: vec![DVec3::ZERO,DVec3::X,DVec3::X+DVec3::Y,DVec3::Y],closed:true }).unwrap();
        let face=*h.model().faces().keys().next().unwrap();
        h.execute(Command::PushPull { face,distance:1. }).unwrap(); h
    }
    #[test]
    fn plane_transform_flip_and_segment() {
        let mut p=SectionPlane::through("cut".into(),DVec3::Z,DVec3::Z*2.).unwrap();
        assert!(p.retains(DVec3::Z*2.)); assert!(!p.retains(DVec3::ZERO));
        assert_eq!(p.clip_segment(DVec3::ZERO,DVec3::Z*2.).unwrap()[0],DVec3::Z);
        let frame=glam::DMat4::from_scale_rotation_translation(DVec3::new(-2.,3.,4.),glam::DQuat::IDENTITY,DVec3::Z);
        let local=p.in_frame(frame).unwrap(); assert!(local.signed_distance(DVec3::ZERO).abs()<1e-12);
        p.flip(); assert!(p.retains(DVec3::ZERO)); assert!(!p.retains(DVec3::Z*2.));
    }
    #[test]
    fn cube_cap_area_and_view_only() {
        let h=cube(); let before=h.model().to_data();
        for normal in [DVec3::X,DVec3::Y,DVec3::Z] {
            let p=SectionPlane::through("cut".into(),DVec3::splat(0.5),normal).unwrap();
            let cap=section_cap(h.model(),&p).unwrap();
            let area:f64=cap.iter().map(|t|(t[1]-t[0]).cross(t[2]-t[0]).length()*0.5).sum();
            assert!((area-1.).abs()<1e-8);
            assert!(cap.iter().flatten().all(|pnt|p.signed_distance(*pnt).abs()<1e-8));
        }
        assert_eq!(before,h.model().to_data());
        let outside=SectionPlane::through("out".into(),DVec3::Z*2.,DVec3::Z).unwrap();
        assert!(section_cap(h.model(),&outside).unwrap().is_empty());
    }
    #[test]
    fn open_surface_has_no_cap() {
        let mut h=cube(); let edges=h.model().edges().keys().copied().take(1).collect();
        h.execute(Command::EraseEdges(edges)).unwrap();
        let p=SectionPlane::through("cut".into(),DVec3::splat(0.5),DVec3::X).unwrap();
        assert!(section_cap(h.model(),&p).unwrap().is_empty());
    }
}
