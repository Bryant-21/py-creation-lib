import numpy as np

from creation_lib.material_tools.pbr_convert import PBRToSpecGlossParams, pbr_to_specgloss

REFERENCE_PARAMS = PBRToSpecGlossParams(ao_multiplier=0.5, spec_offset=0.8)


def test_dielectric_uses_fo76_reference_fill():
    albedo = np.full((2, 2, 3), [1.0, 0.0, 0.0], dtype=np.float32)
    metallic = np.zeros((2, 2), dtype=np.float32)
    roughness = np.full((2, 2), 0.5, dtype=np.float32)
    ao = np.ones((2, 2), dtype=np.float32)
    diff, spec, gloss = pbr_to_specgloss(
        albedo, metallic, roughness, ao, REFERENCE_PARAMS
    )
    np.testing.assert_allclose(diff[0, 0], [1.0, 0.0, 0.0], atol=1e-5)
    np.testing.assert_allclose(spec[0, 0], [0.22, 0.22, 0.22], atol=1e-5)
    assert abs(gloss[0, 0] - 0.5) < 1e-5


def test_full_reflectivity_reaches_white_specular_and_diffuse():
    albedo = np.full((2, 2, 3), [0.8, 0.8, 0.9], dtype=np.float32)
    metallic = np.ones((2, 2), dtype=np.float32)
    roughness = np.zeros((2, 2), dtype=np.float32)
    diff, spec, gloss = pbr_to_specgloss(
        albedo, metallic, roughness, None, REFERENCE_PARAMS
    )
    np.testing.assert_allclose(diff[0, 0], [1.0, 1.0, 1.0], atol=1e-5)
    np.testing.assert_allclose(spec[0, 0], [1.0, 1.0, 1.0], atol=1e-5)
    assert abs(gloss[0, 0] - 1.0) < 1e-5


def test_reflectivity_below_spec_offset_threshold_stays_dielectric():
    albedo = np.full((1, 1, 3), 0.6, dtype=np.float32)
    metallic = np.full((1, 1), 0.1, dtype=np.float32)
    roughness = np.full((1, 1), 0.25, dtype=np.float32)
    _, spec, gloss = pbr_to_specgloss(
        albedo, metallic, roughness, None, REFERENCE_PARAMS
    )
    np.testing.assert_allclose(spec[0, 0], [0.22, 0.22, 0.22], atol=1e-5)
    assert gloss[0, 0] == np.float32(0.75)


def test_ao_multiplier_lerps_between_white_and_ao():
    albedo = np.full((1, 1, 3), 0.8, dtype=np.float32)
    metallic = np.zeros((1, 1), dtype=np.float32)
    roughness = np.full((1, 1), 0.5, dtype=np.float32)
    ao = np.full((1, 1), 0.2, dtype=np.float32)
    diff, _, _ = pbr_to_specgloss(
        albedo, metallic, roughness, ao, REFERENCE_PARAMS
    )
    np.testing.assert_allclose(diff[0, 0], [0.48, 0.48, 0.48], atol=1e-5)


def test_multipliers_honored():
    albedo = np.full((1, 1, 3), 0.5, dtype=np.float32)
    metallic = np.zeros((1, 1), dtype=np.float32)
    roughness = np.full((1, 1), 0.4, dtype=np.float32)
    params = PBRToSpecGlossParams(
        gloss_multiplier=0.5, specular_multiplier=0.5, spec_offset=0.8,
    )
    _, spec, gloss = pbr_to_specgloss(albedo, metallic, roughness, None, params)
    assert abs(gloss[0, 0] - 0.3) < 1e-5  # (1-0.4)*0.5
    assert abs(spec[0, 0, 0] - 0.11) < 1e-5
