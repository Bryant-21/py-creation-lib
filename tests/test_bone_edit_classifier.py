import pytest


# Synthetic skeleton bone names mimicking FO4 third-person conventions
_FO4_3P_BONES = [
    "COM_Twin", "Pelvis", "Pelvis_skin",
    "SPINE1", "SPINE2", "Chest",
    "LArm_Collarbone", "LArm_UpperArm",
    "LArm_ForeArm1", "LArm_ForeArm2", "LArm_ForeArm3",
    "LArm_Hand", "LArm_Finger00", "LArm_Finger01",
    "RArm_Collarbone", "RArm_UpperArm",
    "RArm_ForeArm1", "RArm_ForeArm2", "RArm_ForeArm3",
    "RArm_Hand", "RArm_Finger00", "RArm_Thumb01",
    "Neck", "HEAD",
    "LLeg_Thigh", "LLeg_Calf", "LLeg_Foot", "LLeg_Foot_Toe",
    "RLeg_Thigh", "RLeg_Calf", "RLeg_Foot", "RLeg_Foot_Toe",
    "Camera", "Camera Control",
    "Body_skin", "AnimObjectA", "Weapon",
]


def test_classifier_assigns_ik_tip_to_hands_and_feet():
    from creation_lib.bone_edit.bone_classifier import BoneClassifier, BoneCategory
    c = BoneClassifier()
    cats = c.classify_all(_FO4_3P_BONES)
    assert cats["RArm_Hand"] == BoneCategory.IK_TIP
    assert cats["LArm_Hand"] == BoneCategory.IK_TIP
    assert cats["LLeg_Foot"] == BoneCategory.IK_TIP
    assert cats["RLeg_Foot"] == BoneCategory.IK_TIP


def test_classifier_assigns_ik_pole_to_forearm1_and_calf():
    from creation_lib.bone_edit.bone_classifier import BoneClassifier, BoneCategory
    c = BoneClassifier()
    cats = c.classify_all(_FO4_3P_BONES)
    assert cats["RArm_ForeArm1"] == BoneCategory.IK_POLE
    assert cats["LArm_ForeArm1"] == BoneCategory.IK_POLE
    assert cats["RLeg_Calf"] == BoneCategory.IK_POLE
    assert cats["LLeg_Calf"] == BoneCategory.IK_POLE


def test_classifier_assigns_limb_segment_to_upper_arms_thighs_twists_spine():
    from creation_lib.bone_edit.bone_classifier import BoneClassifier, BoneCategory
    c = BoneClassifier()
    cats = c.classify_all(_FO4_3P_BONES)
    for n in ["RArm_UpperArm", "LArm_UpperArm", "RLeg_Thigh", "LLeg_Thigh"]:
        assert cats[n] == BoneCategory.LIMB_SEGMENT, n
    for n in ["RArm_ForeArm2", "RArm_ForeArm3", "LArm_ForeArm2", "LArm_ForeArm3"]:
        assert cats[n] == BoneCategory.LIMB_SEGMENT, n
    for n in ["SPINE1", "SPINE2", "Chest", "Neck", "RArm_Collarbone", "LArm_Collarbone"]:
        assert cats[n] == BoneCategory.LIMB_SEGMENT, n


def test_classifier_assigns_mount_to_root_fingers_toe():
    from creation_lib.bone_edit.bone_classifier import BoneClassifier, BoneCategory
    c = BoneClassifier()
    cats = c.classify_all(_FO4_3P_BONES)
    # Head rotates about the neck joint — rotation-only (LIMB_SEGMENT).
    assert cats["HEAD"] == BoneCategory.LIMB_SEGMENT
    assert cats["COM_Twin"] == BoneCategory.MOUNT
    assert cats["Pelvis"] == BoneCategory.MOUNT
    assert cats["RArm_Finger00"] == BoneCategory.MOUNT
    assert cats["RArm_Thumb01"] == BoneCategory.MOUNT
    assert cats["LLeg_Foot_Toe"] == BoneCategory.MOUNT
    assert cats["Camera"] == BoneCategory.MOUNT


def test_classifier_hides_skin_and_animobject_bones():
    from creation_lib.bone_edit.bone_classifier import BoneClassifier
    c = BoneClassifier()
    hidden = c.hidden_bones(_FO4_3P_BONES)
    assert "Pelvis_skin" in hidden
    assert "Body_skin" in hidden
    assert "AnimObjectA" in hidden


def test_classifier_unknown_bone_falls_back_to_limb_segment():
    from creation_lib.bone_edit.bone_classifier import BoneClassifier, BoneCategory
    c = BoneClassifier()
    cats = c.classify_all(["MysteryBone"])
    assert cats["MysteryBone"] == BoneCategory.LIMB_SEGMENT


def test_classifier_override_changes_category():
    from creation_lib.bone_edit.bone_classifier import BoneClassifier, BoneCategory
    c = BoneClassifier()
    c.set_override("RArm_Hand", BoneCategory.MOUNT)
    cats = c.classify_all(["RArm_Hand"])
    assert cats["RArm_Hand"] == BoneCategory.MOUNT


def test_classifier_detects_arm_chain():
    from creation_lib.bone_edit.bone_classifier import BoneClassifier
    c = BoneClassifier()
    parent_indices = _build_synthetic_arm_parents()
    chains = c.detect_chains(_FO4_3P_BONES, parent_indices)
    assert "RArm_Hand" in chains
    chain = chains["RArm_Hand"]
    assert chain.tip == "RArm_Hand"
    assert chain.mid == "RArm_ForeArm1"
    assert chain.root == "RArm_UpperArm"


def test_classifier_detects_leg_chain():
    from creation_lib.bone_edit.bone_classifier import BoneClassifier
    c = BoneClassifier()
    parent_indices = _build_synthetic_arm_parents()
    chains = c.detect_chains(_FO4_3P_BONES, parent_indices)
    assert "RLeg_Foot" in chains
    chain = chains["RLeg_Foot"]
    assert chain.tip == "RLeg_Foot"
    assert chain.mid == "RLeg_Calf"
    assert chain.root == "RLeg_Thigh"


def test_classifier_falls_back_to_mount_when_no_chain():
    """An IK_TIP without a recognizable mid+root should be reclassified to MOUNT."""
    from creation_lib.bone_edit.bone_classifier import BoneClassifier, BoneCategory
    c = BoneClassifier()
    bones = ["FloatingHand"]
    parent_indices = [-1]
    c.set_override("FloatingHand", BoneCategory.IK_TIP)
    chains = c.detect_chains(bones, parent_indices)
    cats = c.classify_all(bones, chains=chains)
    assert "FloatingHand" not in chains
    assert cats["FloatingHand"] == BoneCategory.MOUNT


def _build_synthetic_arm_parents() -> list[int]:
    """Parent indices for the synthetic FO4 3P bone list above."""
    name_to_idx = {name: i for i, name in enumerate(_FO4_3P_BONES)}
    parents = [-1] * len(_FO4_3P_BONES)
    edges = {
        "Pelvis": "COM_Twin", "Pelvis_skin": "Pelvis",
        "SPINE1": "Pelvis", "SPINE2": "SPINE1", "Chest": "SPINE2",
        "LArm_Collarbone": "Chest", "LArm_UpperArm": "LArm_Collarbone",
        "LArm_ForeArm1": "LArm_UpperArm", "LArm_ForeArm2": "LArm_ForeArm1",
        "LArm_ForeArm3": "LArm_ForeArm2", "LArm_Hand": "LArm_ForeArm3",
        "LArm_Finger00": "LArm_Hand", "LArm_Finger01": "LArm_Finger00",
        "RArm_Collarbone": "Chest", "RArm_UpperArm": "RArm_Collarbone",
        "RArm_ForeArm1": "RArm_UpperArm", "RArm_ForeArm2": "RArm_ForeArm1",
        "RArm_ForeArm3": "RArm_ForeArm2", "RArm_Hand": "RArm_ForeArm3",
        "RArm_Finger00": "RArm_Hand", "RArm_Thumb01": "RArm_Hand",
        "Neck": "Chest", "HEAD": "Neck",
        "LLeg_Thigh": "Pelvis", "LLeg_Calf": "LLeg_Thigh",
        "LLeg_Foot": "LLeg_Calf", "LLeg_Foot_Toe": "LLeg_Foot",
        "RLeg_Thigh": "Pelvis", "RLeg_Calf": "RLeg_Thigh",
        "RLeg_Foot": "RLeg_Calf", "RLeg_Foot_Toe": "RLeg_Foot",
        "Body_skin": "COM_Twin", "AnimObjectA": "RArm_Hand", "Weapon": "RArm_Hand",
        "Camera": "Chest", "Camera Control": "Camera",
    }
    for child, parent in edges.items():
        if child in name_to_idx and parent in name_to_idx:
            parents[name_to_idx[child]] = name_to_idx[parent]
    return parents
