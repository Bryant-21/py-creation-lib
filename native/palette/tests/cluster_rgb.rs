use ndarray::Array2;
use palette_native::clustering::cluster_rgb;

#[test]
fn clusters_three_separated_groups() {
    let mut data: Vec<f32> = Vec::with_capacity(300 * 3);
    for _ in 0..100 {
        data.extend_from_slice(&[10.0, 10.0, 10.0]);
    }
    for _ in 0..100 {
        data.extend_from_slice(&[200.0, 200.0, 200.0]);
    }
    for _ in 0..100 {
        data.extend_from_slice(&[100.0, 50.0, 150.0]);
    }
    let pixels = Array2::from_shape_vec((300, 3), data).unwrap();

    let (centers, labels) = cluster_rgb(pixels.view(), pixels.view(), 3, 42, 3);

    assert_eq!(centers.shape(), &[3, 3]);
    assert_eq!(labels.len(), 300);

    for i in 1..100 {
        assert_eq!(labels[i], labels[0], "group 1 split at {}", i);
    }
    for i in 101..200 {
        assert_eq!(labels[i], labels[100], "group 2 split at {}", i);
    }
    for i in 201..300 {
        assert_eq!(labels[i], labels[200], "group 3 split at {}", i);
    }
    assert_ne!(labels[0], labels[100]);
    assert_ne!(labels[100], labels[200]);
    assert_ne!(labels[0], labels[200]);
}

#[test]
fn predict_on_holdout_returns_expected_shape_and_assignments() {
    let mut fit: Vec<f32> = Vec::new();
    for _ in 0..50 {
        fit.extend_from_slice(&[10.0, 10.0, 10.0]);
    }
    for _ in 0..50 {
        fit.extend_from_slice(&[200.0, 200.0, 200.0]);
    }
    let fit_arr = Array2::from_shape_vec((100, 3), fit).unwrap();

    let predict = Array2::from_shape_vec(
        (4, 3),
        vec![
            10.0, 10.0, 10.0, 200.0, 200.0, 200.0, 15.0, 12.0, 8.0, 198.0, 205.0, 199.0,
        ],
    )
    .unwrap();

    let (_, labels) = cluster_rgb(fit_arr.view(), predict.view(), 2, 42, 3);

    assert_eq!(labels.len(), 4);
    assert_eq!(labels[0], labels[2], "near-10 points should share a label");
    assert_eq!(labels[1], labels[3], "near-200 points should share a label");
    assert_ne!(labels[0], labels[1]);
}
