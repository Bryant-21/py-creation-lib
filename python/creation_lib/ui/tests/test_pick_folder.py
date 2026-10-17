import importlib
from unittest.mock import MagicMock

import pytest


picker = importlib.import_module("creation_lib.ui.widgets.pick_folder")


@pytest.mark.parametrize("default_path", ["", "C:/Mods/MyMod"])
@pytest.mark.parametrize("result", ["", "C:/Mods/SelectedMod"])
def test_folder_picker_only_forces_configured_path(monkeypatch, default_path, result):
    dialog = MagicMock()
    dialog.ready.return_value = True
    dialog.result.return_value = result
    select_folder = MagicMock(return_value=dialog)
    monkeypatch.setattr(picker.pfd, "select_folder", select_folder)

    assert picker.pick_folder("Deploy Folder", default_path) == (result or None)

    options = picker.pfd.opt.force_path if default_path else picker.pfd.opt.none
    select_folder.assert_called_once_with("Deploy Folder", default_path, options)


def test_folder_picker_starts_in_parent_for_existing_file(monkeypatch, tmp_path):
    source = tmp_path / "image.png"
    source.touch()
    dialog = MagicMock()
    dialog.ready.return_value = True
    dialog.result.return_value = ""
    select_folder = MagicMock(return_value=dialog)
    monkeypatch.setattr(picker.pfd, "select_folder", select_folder)

    assert picker.pick_folder("Input Folder", str(source)) is None

    select_folder.assert_called_once_with("Input Folder", str(tmp_path), picker.pfd.opt.force_path)
