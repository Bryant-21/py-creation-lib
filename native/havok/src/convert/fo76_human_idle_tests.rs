use super::*;

fn field<'a>(object: &'a HkxObject, name: &str) -> &'a HkxValue {
    &object
        .members
        .iter()
        .find(|member| member.name == name)
        .unwrap()
        .value
}

fn animation(file: &HkxFile) -> &HkxObject {
    file.objects()
        .iter()
        .find(|object| {
            matches!(
                object.class_name.as_str(),
                "hkaSplineCompressedAnimation" | "hkaInterleavedUncompressedAnimation"
            )
        })
        .unwrap()
}

#[test]
fn fo76_human_idle_jaw_tracks_and_annotations_follow_fo4_bones() {
    let object = |class_name: &str, members: Vec<(&str, HkxValue)>| HkxObject {
        name: Some(class_name.into()),
        offset: 0,
        signature: 0,
        class_name: class_name.into(),
        members: members
            .into_iter()
            .map(|(name, value)| HkxMember {
                name: name.into(),
                value,
            })
            .collect(),
    };
    let fixture = HkxFile::from_tagxml(
        11,
        "hk_2015.1.0-r1",
        vec![
            object(
                "hkaInterleavedUncompressedAnimation",
                vec![
                    ("duration", HkxValue::F32(1.0 / 30.0)),
                    ("numberOfTransformTracks", HkxValue::I32(97)),
                    ("numberOfFloatTracks", HkxValue::I32(0)),
                    (
                        "transforms",
                        HkxValue::Array(
                            (0..194)
                                .map(|index| {
                                    HkxValue::F32List(vec![
                                        (index % 97) as f32,
                                        (index / 97) as f32,
                                        0.0,
                                        0.0,
                                        0.0,
                                        0.0,
                                        0.0,
                                        1.0,
                                        1.0,
                                        1.0,
                                        1.0,
                                        0.0,
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                    (
                        "annotationTracks",
                        HkxValue::Array((0..97).map(HkxValue::I32).collect()),
                    ),
                ],
            ),
            object(
                "hkaAnimationBinding",
                vec![
                    (
                        "originalSkeletonName",
                        HkxValue::String {
                            value: "Root".into(),
                            is_null: false,
                        },
                    ),
                    ("animation", HkxValue::Pointer(Some(0))),
                    (
                        "transformTrackToBoneIndices",
                        HkxValue::Array((0..97).map(HkxValue::I32).collect()),
                    ),
                ],
            ),
        ],
    );
    for (keep_aim_source, compressed) in
        [(false, false), (false, true), (true, false), (true, true)]
    {
        let mut file = fixture.clone();
        if compressed {
            recompress_animations(&mut file).unwrap();
        }
        auto_fix_human_bone_tracks(&mut file, keep_aim_source);
        decompress_spline_animations(&mut file).unwrap();
        let tracks = 95 + usize::from(keep_aim_source);
        let anim = animation(&file);
        assert_eq!(
            direct_member_as_i32(field(anim, "numberOfTransformTracks")),
            Some(tracks as i32)
        );
        let HkxValue::Array(transforms) = field(anim, "transforms") else {
            panic!("transforms")
        };
        assert_eq!(transforms.len(), 2 * tracks);
        let HkxValue::Array(annotations) = field(anim, "annotationTracks") else {
            panic!("annotations")
        };
        assert_eq!(annotations.len(), tracks);
        let HkxValue::Array(bones) = field(&file.objects()[1], "transformTrackToBoneIndices")
        else {
            panic!("binding")
        };
        assert_eq!(
            *bones,
            (0..tracks as i32).map(HkxValue::I32).collect::<Vec<_>>()
        );
        let aim_source = keep_aim_source.then_some((95, 12));
        for (target, source) in [(0, 0), (12, 13), (19, 20), (48, 53), (94, 95)]
            .into_iter()
            .chain(aim_source)
        {
            assert_eq!(annotations[target], HkxValue::I32(source as i32));
            for frame in 0..2 {
                let HkxValue::F32List(transform) = &transforms[frame * tracks + target] else {
                    panic!("transform")
                };
                assert!((transform[0] - source as f32).abs() < 0.001);
                assert!((transform[1] - frame as f32).abs() < 0.001);
            }
        }
    }
    let mut custom = fixture;
    custom.objects_mut()[1].members[0].value = HkxValue::String {
        value: "Creature".into(),
        is_null: false,
    };
    let before = custom.objects().to_vec();
    auto_fix_human_bone_tracks(&mut custom, false);
    assert_eq!(custom.objects(), before);
}
