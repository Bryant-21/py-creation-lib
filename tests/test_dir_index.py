from creation_lib.db.dir_index import DirectoryIndex


def test_directory_index_cache_hit_resolves_lazily(tmp_path):
    root = tmp_path / "assets"
    file_path = root / "Textures" / "Weapons" / "Demo" / "diffuse.dds"
    file_path.parent.mkdir(parents=True)
    file_path.write_bytes(b"dds")
    cache_dir = tmp_path / "cache"

    cold = DirectoryIndex(root, cache_dir=cache_dir)
    assert cold.file_count == 1
    cold.close()

    warm = DirectoryIndex(root, cache_dir=cache_dir)
    assert warm.file_count == 1
    assert warm._lookup == {}

    resolved = warm.resolve("textures/weapons/demo/diffuse.dds")

    assert resolved == file_path
    assert warm._lookup == {
        "textures/weapons/demo/diffuse.dds": str(file_path)
    }
    warm.close()
