//! RGB pixel clustering via linfa-clustering's KMeans.
//!
//! Thin wrapper that adapts our f32 PyO3 surface to linfa's f64 KMeans.
//! Deterministic given a seed; uses linfa's native `n_runs` for multi-restart.

use linfa::DatasetBase;
use linfa::traits::{Fit, Predict};
use linfa_clustering::KMeans;
use ndarray::{Array1, Array2, ArrayView2};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

const MAX_ITER: u64 = 100;
const TOL: f64 = 1e-4;

pub fn cluster_rgb(
    fit_pixels: ArrayView2<f32>,
    predict_pixels: ArrayView2<f32>,
    n_clusters: usize,
    seed: u64,
    n_init: u32,
) -> (Array2<f32>, Array1<i32>) {
    assert!(n_init >= 1, "n_init must be >= 1");
    assert!(n_clusters >= 1, "n_clusters must be >= 1");

    let fit_f64: Array2<f64> = fit_pixels.mapv(|x| x as f64);
    let predict_f64: Array2<f64> = predict_pixels.mapv(|x| x as f64);

    let rng = ChaCha8Rng::seed_from_u64(seed);
    let dataset = DatasetBase::from(fit_f64);

    let model = KMeans::params_with_rng(n_clusters, rng)
        .n_runs(n_init as usize)
        .max_n_iterations(MAX_ITER)
        .tolerance(TOL)
        .fit(&dataset)
        .expect("kmeans fit failed");

    let centers: Array2<f32> = model.centroids().mapv(|x| x as f32);
    let labels: Array1<i32> = model.predict(&predict_f64).mapv(|x| x as i32);

    (centers, labels)
}
