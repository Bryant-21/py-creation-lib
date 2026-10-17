from creation_lib.build.deployer import deploy_xse_files


def test_remove_paths_deletes_stale_files_after_copy(tmp_path):
    mod = tmp_path / "mod"
    (mod / "F4SE" / "Plugins").mkdir(parents=True)
    (mod / "F4SE" / "Plugins" / "new.dll").write_bytes(b"x")
    data = tmp_path / "Data"
    stale = data / "F4SE" / "Plugins" / "Cat" / "old.json"
    stale.parent.mkdir(parents=True)
    stale.write_text("{}")
    keep = data / "F4SE" / "Plugins" / "Cat" / "live.json"
    keep.write_text("{}")

    copied, _ = deploy_xse_files(mod, data, game="fo4", remove_paths=["F4SE/Plugins/Cat/old.json", "F4SE/missing.json"])

    assert copied == 1
    assert not stale.exists()
    assert keep.is_file()
    assert (data / "F4SE" / "Plugins" / "new.dll").is_file()
