# tests/test_bone_edit_spline.py
import struct
import numpy as np


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


def test_decode_48bit_quaternion_and_encode_decode_roundtrip():
    """Decoding a raw 48-bit quaternion yields a unit quaternion; encode/decode round-trips."""
    from creation_lib.bone_edit.spline_patcher import (
        decode_quaternion, encode_quaternion,
    )

    # Identity-ish quaternion encoded as 48-bit (3x uint16)
    # For identity (0,0,0,1): all three stored components ~ 0, reconstructed = 1
    # Center value = 0x3FFF = 16383 (maps to 0.0)
    center = 0x3FFF
    data = struct.pack("<3H", center, center, center)
    q = decode_quaternion(data, 0, rot_quant=4)
    assert len(q) == 4
    np.testing.assert_allclose(np.linalg.norm(q), 1.0, atol=1e-3)

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
