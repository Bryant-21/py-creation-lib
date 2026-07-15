from creation_lib.bone_edit.classifier_bridge import detect_mirror_pairs


def test_detect_mirror_pairs_l_r_prefix_with_underscore():
    bones = ["L_UpperArm", "R_UpperArm", "L_ForeArm1", "R_ForeArm1", "Spine1"]
    pairs = detect_mirror_pairs(bones)
    assert sorted(pairs) == [["L_ForeArm1", "R_ForeArm1"], ["L_UpperArm", "R_UpperArm"]]


def test_detect_mirror_pairs_ignores_center_bones():
    bones = ["Spine1", "Spine2", "Head", "Pelvis"]
    assert detect_mirror_pairs(bones) == []


def test_detect_mirror_pairs_handles_suffix_convention():
    bones = ["Arm_L", "Arm_R", "Leg_L", "Leg_R"]
    pairs = detect_mirror_pairs(bones)
    assert sorted(pairs) == [["Arm_L", "Arm_R"], ["Leg_L", "Leg_R"]]


def test_detect_mirror_pairs_skips_unmatched():
    bones = ["L_Hand", "L_Foot", "R_Hand"]
    pairs = detect_mirror_pairs(bones)
    assert pairs == [["L_Hand", "R_Hand"]]


def test_detect_mirror_pairs_fo4_arm_prefix():
    bones = ["LArm_UpperArm", "RArm_UpperArm", "LArm_ForeArm1", "RArm_ForeArm1", "Pelvis"]
    pairs = detect_mirror_pairs(bones)
    assert sorted(pairs) == [
        ["LArm_ForeArm1", "RArm_ForeArm1"],
        ["LArm_UpperArm", "RArm_UpperArm"],
    ]


def test_detect_mirror_pairs_fo4_leg_prefix():
    bones = ["LLeg_Thigh", "RLeg_Thigh", "LLeg_Calf", "RLeg_Calf"]
    pairs = detect_mirror_pairs(bones)
    assert sorted(pairs) == [
        ["LLeg_Calf", "RLeg_Calf"],
        ["LLeg_Thigh", "RLeg_Thigh"],
    ]


def test_detect_mirror_pairs_fo4_hand_foot_prefix():
    bones = ["LHand_Finger1", "RHand_Finger1", "LFoot_Toe", "RFoot_Toe"]
    pairs = detect_mirror_pairs(bones)
    assert sorted(pairs) == [
        ["LFoot_Toe", "RFoot_Toe"],
        ["LHand_Finger1", "RHand_Finger1"],
    ]


def test_detect_mirror_pairs_fo4_no_false_match_on_non_prefix_bones():
    # Bones that happen to start with L/R but aren't valid FO4 prefixes should not pair.
    bones = ["Lorem", "Roman"]
    pairs = detect_mirror_pairs(bones)
    assert pairs == []


from creation_lib.bone_edit.classifier_bridge import classify_skeleton_to_dict


def _simple_human_skel():
    bones = [
        "Root",          # 0
        "Spine1",        # 1 (parent=0)
        "L_Collarbone",  # 2 (parent=1)
        "L_UpperArm",    # 3 (parent=2)
        "L_ForeArm1",    # 4 (parent=3)
        "L_ForeArm2",    # 5 (parent=4)
        "L_Hand",        # 6 (parent=5)
        "R_Collarbone",  # 7 (parent=1)
        "R_UpperArm",    # 8 (parent=7)
        "R_ForeArm1",    # 9 (parent=8)
        "R_Hand",        # 10 (parent=9)
    ]
    parents = [-1, 0, 1, 2, 3, 4, 5, 1, 7, 8, 9]
    return bones, parents


def test_classify_skeleton_to_dict_shape():
    bones, parents = _simple_human_skel()
    out = classify_skeleton_to_dict(bone_names=bones, parent_indices=parents)
    assert set(out.keys()) == {"chains", "mirror_pairs", "categories"}


def test_classify_skeleton_to_dict_chains_are_json_safe():
    import json
    bones, parents = _simple_human_skel()
    out = classify_skeleton_to_dict(bone_names=bones, parent_indices=parents)
    json.dumps(out)  # raises if any numpy arrays leaked


def test_classify_skeleton_to_dict_finds_left_arm_chain():
    bones, parents = _simple_human_skel()
    out = classify_skeleton_to_dict(bone_names=bones, parent_indices=parents)
    chain_by_tip = {c["tip"]: c for c in out["chains"]}
    assert "L_Hand" in chain_by_tip
    chain = chain_by_tip["L_Hand"]
    assert chain["root"] == "L_UpperArm"
    assert chain["mid"] == "L_ForeArm1"
    assert len(chain["pole_offset"]) == 3


def test_classify_skeleton_to_dict_mirror_pairs_populated():
    bones, parents = _simple_human_skel()
    out = classify_skeleton_to_dict(bone_names=bones, parent_indices=parents)
    names = {tuple(p) for p in out["mirror_pairs"]}
    assert ("L_UpperArm", "R_UpperArm") in names or ("R_UpperArm", "L_UpperArm") in names


def test_classify_skeleton_to_dict_categories_covers_all_bones():
    bones, parents = _simple_human_skel()
    out = classify_skeleton_to_dict(bone_names=bones, parent_indices=parents)
    assert set(out["categories"].keys()) == set(bones)
