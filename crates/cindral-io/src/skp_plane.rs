//! Least-squares plane fitting in local meter coordinates.
use glam::DVec3;

pub(crate) fn fit(points: &[DVec3]) -> (DVec3, DVec3, f64, f64) {
    let center = points.iter().copied().sum::<DVec3>() / points.len() as f64;
    let mut covariance = [[0.; 3]; 3];
    let mut size: f64 = 0.;
    for p in points {
        let d = (*p - center).to_array();
        size = size.max((*p - center).length() * 2.);
        for i in 0..3 {
            for j in 0..3 {
                covariance[i][j] += d[i] * d[j];
            }
        }
    }
    let mut vectors = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    for _ in 0..32 {
        let (p, q) = [(0, 1), (0, 2), (1, 2)]
            .into_iter()
            .max_by(|&(a, b), &(c, d)| covariance[a][b].abs().total_cmp(&covariance[c][d].abs()))
            .unwrap();
        if covariance[p][q].abs() <= 1e-30 {
            break;
        }
        let angle = 0.5 * (2. * covariance[p][q]).atan2(covariance[q][q] - covariance[p][p]);
        let (s, c) = angle.sin_cos();
        for row in &mut covariance {
            let a = row[p];
            let b = row[q];
            row[p] = c * a - s * b;
            row[q] = s * a + c * b;
        }
        let row_p = covariance[p];
        let row_q = covariance[q];
        for (k, (&a, &b)) in row_p.iter().zip(&row_q).enumerate() {
            covariance[p][k] = c * a - s * b;
            covariance[q][k] = s * a + c * b;
        }
        for row in &mut vectors {
            let a = row[p];
            let b = row[q];
            row[p] = c * a - s * b;
            row[q] = s * a + c * b;
        }
    }
    let axis = (0..3)
        .min_by(|&a, &b| covariance[a][a].total_cmp(&covariance[b][b]))
        .unwrap();
    let normal = DVec3::new(vectors[0][axis], vectors[1][axis], vectors[2][axis]).normalize();
    let deviation = points
        .iter()
        .map(|p| normal.dot(*p - center).abs())
        .fold(0., f64::max);
    (center, normal, deviation, size)
}
