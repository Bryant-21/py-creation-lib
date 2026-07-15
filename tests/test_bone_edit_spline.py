# tests/test_bone_edit_spline.py
import pytest
import struct
import numpy as np
from pathlib import Path

SKELETON_HKX = Path("resource/skeleton.hkx")


def _get_test_spline_hkx_file():
    """Find a spline-compressed FO4 animation and return its parsed HKXFile."""
    import glob
    from creation_lib.hkxpack import load_hkx

    patterns = [
        "extracted/fo4/meshes/actors/character/_1stperson/animations/minigun/wpnchargeup.hkx",
        "extracted/fo4/meshes/actors/character/_1stperson/animations/minigun/wpnequip.hkx",
        "extracted/fo4/meshes/actors/character/_1stperson/animations/*/wpnchargeup.hkx",
        "extracted/fo4/meshes/actors/character/_1stperson/animations/*/wpnequip.hkx",
    ]
    for pattern in patterns:
        for match in glob.glob(pattern):
            try:
                hkx_file, _ = load_hkx(match)
            except Exception:
                continue
            for obj in hkx_file.objects:
                if "SplineCompressed" in obj.class_name:
                    return hkx_file
    pytest.skip("No spline-compressed FO4 animation files available")


def test_parse_track_masks():
    """Parse mask bytes and extract quantization info."""
    from creation_lib.bone_edit.spline_patcher import parse_track_mask

    # Construct a mask: pos_quant=1(16-bit), rot_quant=4(48-bit, stored as 4-2=2),
    # scale_quant=0(8-bit)
    # Byte 0: pos_quant=1 (bits 0-1), rot_quant=2 (bits 2-5, decoded: 2+2=format 4 i.e. 48-bit), pos=1 (bits 0-1, 16-bit)
    b0 = 0b00_0010_01  # scale=0 (bits 6-7), rot raw=2 (bits 2-5, decoded: 2+2=format 4 i.e. 48-bit), pos=1 (bits 0-1, 16-bit)
    # Byte 1: position has spline X,Y,Z (bits 4-6 = 0x70)
    b1 = 0x70  # spline X|Y|Z
    # Byte 2: rotation has spline (0xF0)
    b2 = 0xF0
    # Byte 3: scale is identity (0x00)
    b3 = 0x00

    mask = parse_track_mask(bytes([b0, b1, b2, b3]))
    assert mask.pos_quant == 1  # 16-bit
    assert mask.rot_quant == 4  # 48-bit (2 + 2)
    assert mask.scale_quant == 0  # 8-bit
    assert mask.pos_type == "spline"
    assert mask.rot_type == "spline"
    assert mask.scale_type == "identity"


def test_decode_48bit_quaternion():
    """Decode a 48-bit quaternion and verify result is unit length."""
    from creation_lib.bone_edit.spline_patcher import decode_quaternion

    # Identity-ish quaternion encoded as 48-bit (3x uint16)
    # For identity (0,0,0,1): all three stored components ~ 0, reconstructed = 1
    # Center value = 0x3FFF = 16383 (maps to 0.0)
    center = 0x3FFF
    data = struct.pack("<3H", center, center, center)
    q = decode_quaternion(data, 0, rot_quant=4)
    assert len(q) == 4
    np.testing.assert_allclose(np.linalg.norm(q), 1.0, atol=1e-3)


def test_encode_decode_roundtrip_quat():
    """Encode then decode a quaternion preserves approximate values."""
    from creation_lib.bone_edit.spline_patcher import encode_quaternion, decode_quaternion

    q_orig = np.array([0.0, 0.0, 0.7071068, 0.7071068])  # 90 deg around Z
    encoded = encode_quaternion(q_orig, rot_quant=4)  # 48-bit
    assert len(encoded) == 6
    q_decoded = decode_quaternion(encoded, 0, rot_quant=4)
    np.testing.assert_allclose(q_decoded, q_orig, atol=0.01)


def test_encode_decode_roundtrip_scalar_16bit():
    """16-bit scalar encode/decode round-trip."""
    from creation_lib.bone_edit.spline_patcher import encode_scalar, decode_scalar

    val = 3.14159
    mn, mx = 0.0, 10.0
    encoded = encode_scalar(val, mn, mx, quant=1)  # 16-bit
    assert len(encoded) == 2
    decoded = decode_scalar(encoded, 0, mn, mx, quant=1)
    np.testing.assert_allclose(decoded, val, atol=0.001)


def test_patch_translation_offset():
    """Apply translation offset to a track's control points."""
    from creation_lib.bone_edit.spline_patcher import SplinePatcher

    hkx_file = _get_test_spline_hkx_file()
    patcher = SplinePatcher.from_hkx_file(hkx_file)
    # Find a track that has spline translation
    track_idx = None
    for i, mask in enumerate(patcher.track_masks):
        if mask.pos_type == "spline":
            track_idx = i
            break
    if track_idx is None:
        pytest.skip("No spline translation tracks in test animation")

    # Get original control points
    orig_cps = patcher.get_translation_cps(track_idx)
    assert len(orig_cps) > 0

    # Build delta that only affects dynamic/static axes (not identity axes)
    from creation_lib.bone_edit.spline_patcher import _SPLINE_X, _STATIC_X
    mask = patcher.track_masks[track_idx]
    delta = np.zeros(3)
    for comp in range(3):
        if mask.pos_flags & (_SPLINE_X << comp) or mask.pos_flags & (_STATIC_X << comp):
            delta[comp] = 1.0
            break  # just offset one axis for testing
    assert np.any(delta != 0), "Need at least one non-identity axis"

    patcher.offset_translation(track_idx, delta)
    new_cps = patcher.get_translation_cps(track_idx)

    # Each CP should be shifted by delta on the affected axis
    for orig, new in zip(orig_cps, new_cps):
        np.testing.assert_allclose(new - orig, delta, atol=0.05)


def test_walk_every_track_40bit_rotation():
    """Regression: `_rot_alignment(ROTQT_40BIT)` must be 1 (not 4).

    On `AttackSprinting.hkx` every transform track except track 1 uses
    40-bit rotation splines. With the old alignment-4 rule, walking to
    any track after track 2 produced wildly out-of-range read offsets
    (>300KB into a 15KB blob), causing
    `offset_translation` to raise `struct.error`.
    """
    from creation_lib.bone_edit.spline_patcher import SplinePatcher
    from creation_lib.hkxpack import load_hkx

    candidates = [
        Path("extracted/fo4/Meshes/Actors/Character/Animations/1HM/AttackSprinting.hkx"),
        Path("extracted/fo4/meshes/actors/character/animations/1HM/AttackSprinting.hkx"),
    ]
    src = next((c for c in candidates if c.exists()), None)
    if src is None:
        pytest.skip("AttackSprinting.hkx not in extracted dump")

    hkx_file, _ = load_hkx(str(src))
    patcher = SplinePatcher.from_hkx_file(hkx_file)

    # Apply a zero-delta translation to every track — the walk must not
    # raise, and must not skip any track whose position is spline/static.
    for i in range(patcher.num_transform_tracks):
        patcher.offset_translation(i, np.zeros(3))
        patcher.offset_rotation(i, np.array([0.0, 0.0, 0.0, 1.0]))
