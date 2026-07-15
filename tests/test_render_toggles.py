from creation_lib.renderer.render_toggles import RenderToggles

def test_defaults():
    t = RenderToggles()
    assert t.diffuse is True
    assert t.normal is True
    assert t.specular is True
    assert t.env_map is True
    assert t.vertex_colors is True
    assert t.ssao is False
    assert t.shadows is False
    assert t.mesh_alpha == 1.0

def test_mutation():
    t = RenderToggles()
    t.diffuse = False
    assert t.diffuse is False
    t.mesh_alpha = 0.4
    assert t.mesh_alpha == 0.4
