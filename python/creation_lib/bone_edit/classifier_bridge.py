"""Adapter over BoneClassifier that returns serializable chain data.

Adds L/R mirror-pair detection on top of the classifier so the auto-rig
script can emit selection sets and fuel the mirror tool.
"""

from __future__ import annotations

from typing import Any

from creation_lib.bone_edit.bone_classifier import BoneClassifier


# (left, right, kind) — kind is "prefix" or "suffix".
# Order matters: more specific prefixes (FO4 multi-char) come before generic ones.
_L_R_AFFIXES: list[tuple[str, str, str]] = [
    ("LArm_", "RArm_", "prefix"),
    ("LLeg_", "RLeg_", "prefix"),
    ("LHand_", "RHand_", "prefix"),
    ("LFoot_", "RFoot_", "prefix"),
    ("L_", "R_", "prefix"),
    ("Left", "Right", "prefix"),
    ("_L", "_R", "suffix"),
]


def _mirror_partner(name: str) -> str | None:
    """Return the mirrored name for a bone, or None if it doesn't match any L/R affix."""
    for left, right, kind in _L_R_AFFIXES:
        if kind == "prefix":
            if name.startswith(left):
                return right + name[len(left):]
        else:  # suffix
            if name.endswith(left):
                return name[: -len(left)] + right
    return None


def classify_skeleton_to_dict(
    bone_names: list[str],
    parent_indices: list[int],
) -> dict[str, Any]:
    """Return {chains, mirror_pairs, categories} as JSON-safe types."""
    classifier = BoneClassifier()
    chains = classifier.detect_chains(bone_names, parent_indices)
    categories = classifier.classify_all(bone_names, chains=chains)

    chain_list = []
    for tip_name, chain in chains.items():
        chain_list.append(
            {
                "root": chain.root,
                "mid": chain.mid,
                "tip": chain.tip,
                "pole_offset": [float(v) for v in chain.pole_offset.tolist()],
                "category": categories[tip_name].value,
            }
        )

    category_map = {name: cat.value for name, cat in categories.items()}
    return {
        "chains": chain_list,
        "mirror_pairs": detect_mirror_pairs(bone_names),
        "categories": category_map,
    }


def detect_mirror_pairs(bone_names: list[str]) -> list[list[str]]:
    """Return [[left_name, right_name], ...] from L/R naming."""
    names = set(bone_names)
    pairs: list[list[str]] = []
    seen: set[str] = set()
    for name in bone_names:
        if name in seen:
            continue
        partner = _mirror_partner(name)
        if partner is not None and partner in names and partner not in seen:
            pairs.append([name, partner])
            seen.add(name)
            seen.add(partner)
    return pairs
