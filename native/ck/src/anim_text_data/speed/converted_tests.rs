use super::*;

#[test]
#[ignore = "requires converted Auto Axe graph and animation fixtures"]
fn autoaxe_metadata_probe() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let converted = repo.join("mods/SeventySix/data/Meshes");
    let base = repo.join("extracted/fo4/meshes");
    let routes: serde_json::Value = serde_json::from_slice(
        &std::fs::read(repo.join("mods/B21_FO76AutoAxeMovementProbe/testing/autoaxe-routes.json"))
            .unwrap(),
    )
    .unwrap();
    let core = routes[0]["graph"].as_str().unwrap();
    let chain: Vec<String> = routes[0]["paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|path| path.as_str().unwrap().to_string())
        .collect();
    let roots = [converted.as_path(), base.as_path()];
    let recipe = build_speed_info_recipe_weapon(core, &roots, &chain).unwrap();
    let assets = evaluation_assets(&recipe, &roots, &chain).unwrap();
    for request in &recipe.requests {
        let EvaluationRequest::SpeedSampled(request) = request else {
            continue;
        };
        let (file, options) = replay_options(&request.replay).unwrap();
        let mut evaluator = BehaviorEvaluator::load(
            &assets.behaviors[&file],
            &animation_sources(&assets),
            options,
        )
        .unwrap();
        for (angle, expected) in [
            (0.0, [0.0, 1.0]),
            (std::f32::consts::FRAC_PI_2, [-1.0, 0.0]),
            (-std::f32::consts::FRAC_PI_2, [1.0, 0.0]),
            (std::f32::consts::PI, [0.0, -1.0]),
        ] {
            evaluator
                .set_variable(
                    &request.domain.direction_variable,
                    EvaluatorVariableValue::Real(angle * request.direction_input_scale),
                )
                .unwrap();
            evaluator
                .set_variable(
                    &request.domain.speed_variable,
                    EvaluatorVariableValue::Real(request.domain.speed_max),
                )
                .unwrap();
            evaluator
                .advance_repeated(request.domain.timestep, request.domain.warmup_updates)
                .unwrap();
            let motion = evaluator.advance(request.domain.timestep).unwrap();
            assert!(
                motion.x * expected[0] + motion.y * expected[1] > 1.0,
                "angle {angle}: {motion:?}"
            );
        }
    }
    let evaluations = evaluate_recipe(&recipe, &roots, &chain).unwrap();
    let generated = evaluated_speed_info(&recipe, &evaluations).unwrap();
    eprintln!(
        "recipe {:?}; emitted {:?}",
        recipe.stats(),
        generated.stats()
    );
    assert!(
        generated
            .roots
            .iter()
            .any(|root| root.state_machine_path.ends_with("/Ready_IdleLocomotion"))
    );
    assert!(
        generated
            .roots
            .iter()
            .any(|root| root.state_machine_path.ends_with("/Relaxed_IdleLocomotion"))
    );
    for root in &generated.roots {
        if let Contour::SpeedSampled(contour) = &root.contour {
            assert!(contour.curves.len() >= 24);
            for curve in &contour.curves {
                assert!(
                    curve
                        .samples
                        .iter()
                        .all(|sample| sample.output.is_finite() && sample.output > 1.0)
                );
                let maximum = curve
                    .samples
                    .iter()
                    .map(|sample| sample.output)
                    .fold(0.0_f32, f32::max);
                assert!(
                    maximum > 90.0,
                    "{} angle {} max {maximum}",
                    root.state_machine_path,
                    curve.angle
                );
            }
        }
    }
    let body = encode_speed_info(&generated).unwrap();
    assert_eq!(decode_speed_info(&body).unwrap(), generated);
    let loops = speed_info_leaf_basenames(core, &roots, &chain);
    assert!(loops.contains("wpnrunleftready"));
    assert!(loops.contains("wpnrunrightrelaxed"));
    assert!(!loops.contains("wpnmeleeshredder"));
    let mut resolver = super::super::graph::GraphResolver::new(
        roots.iter().map(|root| root.to_path_buf()).collect(),
    );
    let offsets = super::super::offsets::build_subgraph_offsets_body_weapon(
        &mut resolver,
        core,
        &roots,
        &chain,
        &loops,
    )
    .unwrap();
    let output = repo.join("mods/B21_FO76AutoAxeMovementProbe/data/Meshes/AnimTextData");
    for (bucket, bytes) in [("AnimationSpeedInfo", body), ("AnimationOffsets", offsets)] {
        let dir = output.join(bucket);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("{}.txt", routes[0]["id"].as_str().unwrap())),
            bytes,
        )
        .unwrap();
    }
    std::fs::write(
        repo.join("tmp/autoaxe-voice/candidate-speedinfo.txt"),
        format!("{generated:#?}"),
    )
    .unwrap();
}
