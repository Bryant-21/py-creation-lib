"""Tests for SWF timeline model."""
from __future__ import annotations

import pytest

from creation_lib.swf.types import MATRIX, RGBA
from creation_lib.swf.timeline import Timeline, Frame, DisplayEntry


class TestTimeline:
    def test_empty_timeline(self):
        tl = Timeline()
        assert tl.frame_count == 0
        assert len(tl.frames) == 0

    def test_add_frame(self):
        tl = Timeline()
        frame = Frame(label="start")
        tl.frames.append(frame)
        assert tl.frame_count == 1
        assert tl.frames[0].label == "start"

    def test_display_list_accumulates(self):
        """Frames build on previous display list (SWF convention)."""
        tl = Timeline()
        f1 = Frame()
        f1.place(depth=1, character_id=10, matrix=MATRIX())
        tl.frames.append(f1)

        f2 = Frame()
        f2.place(depth=2, character_id=20, matrix=MATRIX())
        tl.frames.append(f2)

        # Frame 2's effective display list has both depths
        dl = tl.display_list_at(1)
        assert 1 in dl
        assert 2 in dl


class TestDisplayEntry:
    def test_entry_fields(self):
        entry = DisplayEntry(
            character_id=5,
            matrix=MATRIX(translate_x=100, translate_y=200),
        )
        assert entry.character_id == 5
        assert entry.matrix.translate_x == 100
