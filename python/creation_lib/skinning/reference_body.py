"""Reference body loading — extract SkinData from NIF files, detect skeletons."""
from __future__ import annotations

import logging
from pathlib import Path

import numpy as np

from .skin_data import SegmentInfo, SkinData, SubSegmentInfo

_log = logging.getLogger("skinning.reference_body")

# Known skeleton profiles per game
SKELETON_PROFILES: dict[str, dict[str, dict]] = {
    "fo4": {
        "human": {
            "display_name": "Human (3rd Person)",
            "category": "Human",
            "skeleton_hkx": "meshes/actors/character/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/character/characterassets/skeleton.nif",
            "body_parts": {
                "male_body": "meshes/actors/character/characterassets/malebody.nif",
                "female_body": "meshes/actors/character/characterassets/femalebody.nif",
                "male_hands": "meshes/actors/character/characterassets/malehands.nif",
                "female_hands": "meshes/actors/character/characterassets/femalehands.nif",
                "male_Helmet": "meshes/actors/character/characterassets/basemalehead.nif",
                "female_Helmet": "meshes/actors/character/characterassets/basefemalehead.nif",
                "male_back_of_Helmet": "meshes/actors/character/characterassets/FaceParts/maleheadRear.nif",
                "female_back_of_Helmet": "meshes/actors/character/characterassets/FaceParts/FemaleheadRear.nif",
                "male_eyes": "meshes/actors/character/characterassets/FaceParts/MaleEyes.nif",
                "female_eyes": "meshes/actors/character/characterassets/FaceParts/FemaleEyes.nif",
            },
            "bone_signatures": ["COM", "Pelvis", "LArm_UpperArm", "Spine1", "Spine2"],
        },
        "human_1st": {
            "display_name": "Human (1st Person)",
            "category": "Human",
            "skeleton_hkx": "meshes/actors/character/_1stperson/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/character/_1stperson/characterassets/skeleton.nif",
            "body_parts": {
                "male_body": "meshes/actors/character/characterassets/1stpersonmalebody.nif",
                "female_body": "meshes/actors/character/characterassets/1stpersonfemalebody.nif",
                "male_hands": "meshes/actors/character/characterassets/1stpersonmalehands.nif",
                "female_hands": "meshes/actors/character/characterassets/1stpersonfemalehands.nif",
            },
            "bone_signatures": ["Camera", "LArm_UpperArm", "RArm_UpperArm"],
        },
        "power_armor": {
            "display_name": "Power Armor",
            "category": "Power Armor",
            "skeleton_hkx": "meshes/actors/powerarmor/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/powerarmor/characterassets/skeleton.nif",
            "variant_labels": {"t45": "T-45", "t51": "T-51", "t60": "T-60", "x01": "X-01"},
            "body_parts": {
                "frame": "meshes/actors/powerarmor/characterassets/Frame.nif",
                "t45_body": "meshes/actors/powerarmor/characterassets/mods/pa_t45_body.nif",
                "t45_left_arm": "meshes/actors/powerarmor/characterassets/mods/pa_t45_larm.nif",
                "t45_right_arm": "meshes/actors/powerarmor/characterassets/mods/pa_t45_rarm.nif",
                "t45_left_leg": "meshes/actors/powerarmor/characterassets/mods/pa_t45_lleg.nif",
                "t45_right_leg": "meshes/actors/powerarmor/characterassets/mods/pa_t45_rleg.nif",
                "t45_head": "meshes/actors/powerarmor/characterassets/mods/pa_t45_Helmet.nif",
                "t51_body": "meshes/actors/powerarmor/characterassets/mods/pa_t51_body.nif",
                "t51_left_arm": "meshes/actors/powerarmor/characterassets/mods/pa_t51_larm.nif",
                "t51_right_arm": "meshes/actors/powerarmor/characterassets/mods/pa_t51_rarm.nif",
                "t51_left_leg": "meshes/actors/powerarmor/characterassets/mods/pa_t51_lleg.nif",
                "t51_right_leg": "meshes/actors/powerarmor/characterassets/mods/pa_t51_rleg.nif",
                "t51_head": "meshes/actors/powerarmor/characterassets/mods/pa_t51_Helmet.nif",
                "t60_body": "meshes/actors/powerarmor/characterassets/mods/pa_t60_body.nif",
                "t60_left_arm": "meshes/actors/powerarmor/characterassets/mods/pa_t60_larm.nif",
                "t60_right_arm": "meshes/actors/powerarmor/characterassets/mods/pa_t60_rarm.nif",
                "t60_left_leg": "meshes/actors/powerarmor/characterassets/mods/pa_t60_lleg.nif",
                "t60_right_leg": "meshes/actors/powerarmor/characterassets/mods/pa_t60_rleg.nif",
                "t60_head": "meshes/actors/powerarmor/characterassets/mods/pa_t60_Helmet.nif",
                "x01_body": "meshes/actors/powerarmor/characterassets/mods/pa_x1_body.nif",
                "x01_left_arm": "meshes/actors/powerarmor/characterassets/mods/pa_x1_larm.nif",
                "x01_right_arm": "meshes/actors/powerarmor/characterassets/mods/pa_x1_rarm.nif",
                "x01_left_leg": "meshes/actors/powerarmor/characterassets/mods/pa_x1_lleg.nif",
                "x01_right_leg": "meshes/actors/powerarmor/characterassets/mods/pa_x1_rleg.nif",
                "x01_head": "meshes/actors/powerarmor/characterassets/mods/pa_x1_Helmet.nif",
            },
            "bone_signatures": ["COM", "Pelvis", "LArm_UpperArm"],
        },
        "super_mutant": {
            "display_name": "Super Mutant",
            "category": "Super Mutant",
            "skeleton_hkx": "meshes/actors/supermutant/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/supermutant/characterassets/skeleton.nif",
            "body_parts": {
                "body": "meshes/actors/supermutant/characterassets/supermutantbody.nif",
                "head": "meshes/actors/supermutant/characterassets/supermutanthead.nif",
            },
            "bone_signatures": ["COM", "Pelvis", "LArm_UpperArm"],
        },
        "deathclaw": {
            "display_name": "Deathclaw",
            "category": "Creature",
            "skeleton_hkx": "meshes/actors/deathclaw/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/deathclaw/characterassets/skeleton.nif",
            "body_parts": {
                "body": "meshes/actors/deathclaw/deathclaw.nif",
            },
            "bone_signatures": ["NPC COM", "Pelvis"],
        },
        "dogmeat": {
            "display_name": "Dogmeat",
            "category": "Creature",
            "skeleton_hkx": "meshes/actors/dogmeat/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/dogmeat/characterassets/skeleton.nif",
            "body_parts": {
                "body": "meshes/actors/dogmeat/characterassets/dogmeat.nif",
            },
            "bone_signatures": ["NPC COM", "Pelvis"],
        },
        "alien": {
            "display_name": "Alien",
            "category": "Creature",
            "skeleton_hkx": "meshes/actors/alien/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/alien/characterassets/skeleton.nif",
            "body_parts": {
                "body": "meshes/actors/alien/characterassets/alien_body.nif",
            },
            "bone_signatures": ["NPC COM", "Pelvis"],
        },
        "bloatfly": {
            "display_name": "Bloatfly",
            "category": "Creature",
            "skeleton_hkx": "meshes/actors/bloatfly/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/bloatfly/characterassets/skeleton.nif",
            "body_parts": {
                "body": "meshes/actors/bloatfly/characterassets/bloatfly.nif",
            },
            "bone_signatures": ["NPC COM", "Pelvis"],
        },
        "brahmin": {
            "display_name": "Brahmin",
            "category": "Creature",
            "skeleton_hkx": "meshes/actors/brahmin/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/brahmin/characterassets/skeleton.nif",
            "body_parts": {
                "body": "meshes/actors/brahmin/characterassets/brahmin.nif",
            },
            "bone_signatures": ["NPC COM", "Pelvis"],
        },
        "cat": {
            "display_name": "Cat",
            "category": "Creature",
            "skeleton_hkx": "meshes/actors/cat/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/cat/characterassets/skeleton.nif",
            "body_parts": {
                "body": "meshes/actors/cat/characterassets/cat.nif",
            },
            "bone_signatures": ["NPC COM", "Pelvis"],
        },
        "create_a_bot": {
            "display_name": "Modular Robot",
            "category": "Robot",
            "skeleton_hkx": "meshes/actors/createabot/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/createabot/characterassets/skeleton.nif",
            "variant_labels": {
                "assaultron": "Assaultron",
                "protectron": "Protectron",
                "sentry": "Sentry Bot",
            },
            "body_parts": {
                "assaultron_torso": "meshes/actors/robot/parts/TorsoAssaultron.nif",
                "assaultron_head": "meshes/actors/robot/parts/HeadAssaultron.nif",
                "assaultron_legs": "meshes/actors/robot/parts/LegsAssaultron.nif",
                "assaultron_left_arm": "meshes/actors/robot/parts/ArmLeftAssaultron.nif",
                "assaultron_right_arm": "meshes/actors/robot/parts/ArmRightAssaultron.nif",
                "protectron_torso": "meshes/actors/robot/parts/TorsoProtectron.nif",
                "protectron_head": "meshes/actors/robot/parts/HeadProtectron.nif",
                "protectron_legs": "meshes/actors/robot/parts/LegsProtectron.nif",
                "protectron_left_arm": "meshes/actors/robot/parts/ArmLeftProtectron.nif",
                "protectron_right_arm": "meshes/actors/robot/parts/ArmRightProtectron.nif",
                "sentry_torso": "meshes/actors/robot/parts/TorsoSentryBot.nif",
                "sentry_head": "meshes/actors/robot/parts/HeadSentryType1.nif",
                "sentry_legs": "meshes/actors/robot/parts/LegsSentryBot.nif",
                "sentry_left_arm": "meshes/actors/robot/parts/ArmLeftSentrybot.nif",
                "sentry_right_arm": "meshes/actors/robot/parts/ArmRightSentrybot.nif",
            },
            "bone_signatures": ["COM", "Pelvis"],
        },
        "eyebot": {
            "display_name": "EyeBot",
            "category": "Robot",
            "skeleton_hkx": "meshes/actors/eyebot/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/eyebot/characterassets/skeleton.nif",
            "body_parts": {
                "body": "meshes/actors/eyebot/characterassets/eyebot.nif",
            },
            "bone_signatures": ["NPC COM", "Pelvis"],
        },
        "fev_hound": {
            "display_name": "FEV Hound",
            "category": "Creature",
            "skeleton_hkx": "meshes/actors/fevhound/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/fevhound/characterassets/skeleton.nif",
            "body_parts": {
                "body": "meshes/actors/fevhound/characterassets/fevhound.nif",
            },
            "bone_signatures": ["NPC COM", "Pelvis"],
        },
        "feral_ghoul": {
            "display_name": "Feral Ghoul",
            "category": "Creature",
            "skeleton_hkx": "meshes/actors/feralghoul/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/feralghoul/characterassets/skeleton.nif",
            "body_parts": {
                "body": "meshes/actors/feralghoul/characterassets/feralghoulbase.nif",
            },
            "bone_signatures": ["NPC COM", "Pelvis"],
        },
        "gorilla": {
            "display_name": "Gorilla",
            "category": "Creature",
            "skeleton_hkx": "meshes/actors/gorilla/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/gorilla/characterassets/skeleton.nif",
            "body_parts": {
                "body": "meshes/actors/gorilla/characterassets/gorilla.nif",
            },
            "bone_signatures": ["NPC COM", "Pelvis"],
        },
        "liberty_prime": {
            "display_name": "Liberty Prime",
            "category": "Robot",
            "skeleton_hkx": "meshes/actors/libertyprime/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/libertyprime/characterassets/skeleton.nif",
            "body_parts": {
                "body": "meshes/actors/libertyprime/characterassets/libertyprime.nif",
            },
            "bone_signatures": ["NPC COM", "Pelvis"],
        },
        "mirelurk": {
            "display_name": "Mirelurk",
            "category": "Creature",
            "skeleton_hkx": "meshes/actors/mirelurk/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/mirelurk/characterassets/skeleton.nif",
            "body_parts": {
                "body": "meshes/actors/mirelurk/characterassets/mirelurk.nif",
            },
            "bone_signatures": ["NPC COM", "Pelvis"],
        },
        "mirelurk_hunter": {
            "display_name": "Mirelurk Hunter",
            "category": "Creature",
            "skeleton_hkx": "meshes/actors/mirelurkHunter/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/mirelurkHunter/characterassets/skeleton.nif",
            "body_parts": {
                "body": "meshes/actors/mirelurkHunter/characterassets/mirelurkHunter.nif",
            },
            "bone_signatures": ["NPC COM", "Pelvis"],
        },
        "mirelurk_king": {
            "display_name": "Mirelurk King",
            "category": "Creature",
            "skeleton_hkx": "meshes/actors/mirelurkKing/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/mirelurkKing/characterassets/skeleton.nif",
            "body_parts": {
                "body": "meshes/actors/mirelurkKing/characterassets/mirelurkKing.nif",
            },
            "bone_signatures": ["NPC COM", "Pelvis"],
        },
        "mirelurk_queen": {
            "display_name": "Mirelurk Queen",
            "category": "Creature",
            "skeleton_hkx": "meshes/actors/mirelurkQueen/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/mirelurkQueen/characterassets/skeleton.nif",
            "body_parts": {
                "body": "meshes/actors/mirelurkQueen/characterassets/mirelurkQueen.nif",
            },
            "bone_signatures": ["NPC COM", "Pelvis"],
        },
        "molerat": {
            "display_name": "Molerat",
            "category": "Creature",
            "skeleton_hkx": "meshes/actors/molerat/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/molerat/characterassets/skeleton.nif",
            "body_parts": {
                "body": "meshes/actors/molerat/characterassets/molerat.nif",
            },
            "bone_signatures": ["NPC COM", "Pelvis"],
        },
        "mosquito": {
            "display_name": "Mosquito (Blood Bug)",
            "category": "Creature",
            "skeleton_hkx": "meshes/actors/mosquito/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/mosquito/characterassets/skeleton.nif",
            "body_parts": {
                "body": "meshes/actors/mosquito/characterassets/bloodbugadultbase.nif",
            },
            "bone_signatures": ["NPC COM", "Pelvis"],
        },
        "rad_roach": {
            "display_name": "Rad Roach",
            "category": "Creature",
            "skeleton_hkx": "meshes/actors/radroach/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/radroach/characterassets/skeleton.nif",
            "body_parts": {
                "body": "meshes/actors/radroach/characterassets/radroach.nif",
            },
            "bone_signatures": ["NPC COM", "Pelvis"],
        },
        "rad_stag": {
            "display_name": "Rad Stag",
            "category": "Creature",
            "skeleton_hkx": "meshes/actors/radstag/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/radstag/characterassets/skeleton.nif",
            "body_parts": {
                "body": "meshes/actors/radstag/characterassets/radstag.nif",
            },
            "bone_signatures": ["NPC COM", "Pelvis"],
        },
        "radscorpion": {
            "display_name": "Radscorpion",
            "category": "Creature",
            "skeleton_hkx": "meshes/actors/radscorpion/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/radscorpion/characterassets/skeleton.nif",
            "body_parts": {
                "body": "meshes/actors/radscorpion/characterassets/radscorpion.nif",
            },
            "bone_signatures": ["NPC COM", "Pelvis"],
        },
        "mr_handy": {
            "display_name": "Mr. Handy",
            "category": "Robot",
            "skeleton_hkx": "meshes/actors/robot/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/robot/characterassets/skeleton.nif",
            "body_parts": {
                "torso": "meshes/actors/robot/parts/TorsoHandy.nif",
                "legs": "meshes/actors/robot/parts/LegsHandyThruster.nif",
                "front_armor": "meshes/actors/robot/parts/HandyFrontArmor.nif",
                "rear_armor": "meshes/actors/robot/parts/HandyRearArmor.nif",
            },
            "bone_signatures": ["COM", "Pelvis"],
        },
        "stingwing": {
            "display_name": "Stingwing",
            "category": "Creature",
            "skeleton_hkx": "meshes/actors/stingwing/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/stingwing/characterassets/skeleton.nif",
            "body_parts": {
                "body": "meshes/actors/stingwing/characterassets/stingwing.nif",
            },
            "bone_signatures": ["NPC COM", "Pelvis"],
        },
        "super_mutant_behemoth": {
            "display_name": "Super Mutant Behemoth",
            "category": "Super Mutant",
            "skeleton_hkx": "meshes/actors/supermutantbehemoth/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/supermutantbehemoth/characterassets/skeleton.nif",
            "body_parts": {
                "body": "meshes/actors/supermutantbehemoth/characterassets/supermutantbehemoth.nif",
            },
            "bone_signatures": ["COM", "Pelvis", "LArm_UpperArm"],
        },
        "synth_gen1": {
            "display_name": "Synth (Gen 1)",
            "category": "Synth",
            "skeleton_hkx": "meshes/actors/character/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/character/characterassets/skeleton.nif",
            "body_parts": {
                "body": "meshes/actors/synths/synthgen1body.nif",
            },
            "bone_signatures": ["COM", "Pelvis", "LArm_UpperArm", "Spine1", "Spine2"],
        },
        "synth_gen2": {
            "display_name": "Synth (Gen 2)",
            "category": "Synth",
            "skeleton_hkx": "meshes/actors/character/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/character/characterassets/skeleton.nif",
            "body_parts": {
                "body": "meshes/actors/synths/synthgen2body.nif",
                "hands": "meshes/actors/synths/synthgen2hands.nif",
            },
            "bone_signatures": ["COM", "Pelvis", "LArm_UpperArm", "Spine1", "Spine2"],
        },
        "turret_standing": {
            "display_name": "Turret (Standing / Tripod)",
            "category": "Turret",
            "skeleton_hkx": "meshes/actors/turret/characterassets/turretstandingskeleton.hkx",
            "skeleton_nif": "meshes/actors/turret/characterassets/turretstandingskeleton.nif",
            "body_parts": {
                "body": "meshes/actors/turret/characterassets/turretstanding.nif",
            },
            "bone_signatures": ["NPC COM", "Pelvis"],
        },
        "turret_mounted": {
            "display_name": "Turret (Mounted / Bubble)",
            "category": "Turret",
            "skeleton_hkx": "meshes/actors/turret/characterassets/turretmountedskeleton.hkx",
            "skeleton_nif": "meshes/actors/turret/characterassets/turretmountedskeleton.nif",
            "body_parts": {
                "body": "meshes/actors/turret/characterassets/turretmounted.nif",
            },
            "bone_signatures": ["NPC COM", "Pelvis"],
        },
        "turret_workshop": {
            "display_name": "Turret (Workshop / Spotlight)",
            "category": "Turret",
            "skeleton_hkx": "meshes/actors/turret/characterassets/skeletonturretworkshop.hkx",
            "skeleton_nif": "meshes/actors/turret/characterassets/skeletonturretworkshop.nif",
            "body_parts": {
                "body": "meshes/actors/turret/characterassets/turretworkshop.nif",
            },
            "bone_signatures": ["NPC COM", "Pelvis"],
        },
        "vertibird": {
            "display_name": "Vertibird",
            "category": "Robot",
            "skeleton_hkx": "meshes/actors/vertibird/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/vertibird/characterassets/skeleton.nif",
            "body_parts": {
                "body": "meshes/actors/vertibird/characterassets/vertbird.nif",
            },
            "bone_signatures": ["NPC COM", "Pelvis"],
        },
        "vicious_dog": {
            "display_name": "Vicious Dog",
            "category": "Creature",
            "skeleton_hkx": "meshes/actors/dogmeat/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/dogmeat/characterassets/skeleton.nif",
            "body_parts": {
                "body": "meshes/actors/viciousdog/viciousdog.nif",
            },
            "bone_signatures": ["NPC COM", "Pelvis"],
        },
        "yao_guai": {
            "display_name": "Yao Guai",
            "category": "Creature",
            "skeleton_hkx": "meshes/actors/yaoguai/characterassets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/yaoguai/characterassets/skeleton.nif",
            "body_parts": {
                "body": "meshes/actors/yaoguai/characterassets/yaoguai.nif",
            },
            "bone_signatures": ["NPC COM", "Pelvis"],
        },
    },
    "skyrimse": {
        "Human": {
            "display_name": "Human",
            "category": "Human",
            "skeleton_hkx": "meshes/actors/character/character assets/skeleton.hkx",
            "skeleton_nif": "meshes/actors/character/character assets/skeleton.nif",
            "body_parts": {
                "male_body": "meshes/actors/character/character assets/malebody_1.nif",
                "female_body": "meshes/actors/character/character assets/femalebody_1.nif",
            },
            "bone_signatures": ["NPC COM", "NPC Pelvis", "NPC L UpperArm"],
        },
    },
}


def detect_skeleton(bone_names: list[str], game: str = "fo4") -> str | None:
    """Return the ``game`` skeleton profile key (e.g. "Human") whose
    bone_signatures best match ``bone_names``, or None.
    """
    profiles = SKELETON_PROFILES.get(game, {})
    bone_set = set(bone_names)

    best_match: str | None = None
    best_score = 0

    for profile_name, profile in profiles.items():
        sigs = profile.get("bone_signatures", [])
        if not sigs:
            continue
        matches = sum(1 for s in sigs if s in bone_set)
        if matches > best_score:
            best_score = matches
            best_match = profile_name

    # Require at least half of the signatures to match
    if best_match:
        sigs = profiles[best_match].get("bone_signatures", [])
        if best_score >= len(sigs) / 2:
            return best_match

    return None


def _nif_local_matrix(block) -> np.ndarray:
    """Build a 4x4 local transform matrix from a NIF block's T/R/S fields.

    NIF rotation dict uses m[col][row] naming (m11,m21,m31 = row 0).
    """
    t = block.get_field("Translation") or {}
    r = block.get_field("Rotation") or {}
    s_raw = block.get_field("Scale")
    s = float(s_raw) if s_raw is not None else 1.0

    tx = float(t.get("x", 0.0))
    ty = float(t.get("y", 0.0))
    tz = float(t.get("z", 0.0))

    r00 = float(r.get("m11", 1.0)); r01 = float(r.get("m12", 0.0)); r02 = float(r.get("m13", 0.0))
    r10 = float(r.get("m21", 0.0)); r11 = float(r.get("m22", 1.0)); r12 = float(r.get("m23", 0.0))
    r20 = float(r.get("m31", 0.0)); r21 = float(r.get("m32", 0.0)); r22 = float(r.get("m33", 1.0))

    m = np.array([
        [r00 * s, r01 * s, r02 * s, tx],
        [r10 * s, r11 * s, r12 * s, ty],
        [r20 * s, r21 * s, r22 * s, tz],
        [0.0,     0.0,     0.0,     1.0],
    ], dtype=np.float32)
    return m


def _build_shape_world_transforms(nif) -> dict[int, np.ndarray]:
    """Walk the NIF tree from roots and return ``{id(shape_block): world_mat4}``.

    Only BSTriShape (and subtypes) are recorded. Parent NiNode transforms are
    multiplied into each shape's local transform so that multiplying the
    returned matrix against a shape-local vertex produces its world-space
    position.
    """
    result: dict[int, np.ndarray] = {}

    def _walk(block_id: int, parent_world: np.ndarray) -> None:
        block = nif.get_block(block_id)
        if block is None:
            return
        local = _nif_local_matrix(block)
        world = parent_world @ local
        if nif.schema.is_subtype_of(block.type_name, "BSTriShape"):
            result[id(block)] = world
        children = block.get_field("Children") or []
        for cid in children:
            try:
                cid_int = int(cid)
            except (TypeError, ValueError):
                continue
            if cid_int >= 0:
                _walk(cid_int, world)

    eye = np.eye(4, dtype=np.float32)
    roots = list(getattr(nif, "_footer_roots", None) or [])
    if not roots and nif.blocks:
        roots = [0]
    for root_id in roots:
        _walk(int(root_id), eye)
    return result


def extract_skin_data_from_nif(nif_or_path) -> SkinData:
    """Extract SkinData from a skinned NIF path or an already loaded ``NifFile``.

    Reads BSTriShape blocks with BSSkin::Instance: vertex positions, normals,
    UVs, bone weights, bone indices, and triangles. Unskinned shapes
    (``Skin == -1``) get their parent NiNode -> BSTriShape transform chain baked
    into vertices and normals so they land in world space (e.g. for the Cloth
    Maker brush); skinned shapes stay in bind-pose space for the caller's
    bind-pose pipeline. Raises FileNotFoundError for a missing path and
    ValueError when no shapes are found.
    """
    from creation_lib.nif import NifFile

    if isinstance(nif_or_path, NifFile):
        nif = nif_or_path
        source_label = Path(getattr(nif, "_filepath", "") or "<memory>")
    else:
        nif_path = Path(nif_or_path)
        if not nif_path.exists():
            raise FileNotFoundError(f"NIF file not found: {nif_path}")
        nif = NifFile.load(str(nif_path))
        source_label = nif_path

    # Find all BSTriShape blocks (including subtypes like BSSubIndexTriShape)
    shapes = [
        block for block in nif.blocks
        if nif.schema.is_subtype_of(block.type_name, "BSTriShape")
    ]
    if not shapes:
        raise ValueError(f"No BSTriShape blocks found in {source_label}")

    # Precompute world transforms for unskinned shapes
    shape_world_transforms = _build_shape_world_transforms(nif)

    # Collect all skinned shape data and merge
    all_verts: list[np.ndarray] = []
    all_normals: list[np.ndarray] = []
    all_uvs: list[np.ndarray] = []
    all_tris: list[np.ndarray] = []
    all_weights: list[np.ndarray] = []
    all_bone_indices: list[np.ndarray] = []
    all_bone_names: list[str] = []
    all_inv_binds: list[np.ndarray] = []
    all_segment_ids: list[np.ndarray] = []
    all_segments: list[SegmentInfo] = []
    all_vertex_colors: list[np.ndarray] = []
    has_any_vertex_colors = False
    all_ssf: str = ""
    bone_name_to_idx: dict[str, int] = {}
    vertex_offset = 0
    tri_offset = 0

    for shape in shapes:
        vertex_data_list = shape.get_field("Vertex Data") or []
        triangles_list = shape.get_field("Triangles") or []

        if not vertex_data_list:
            continue

        n_verts = len(vertex_data_list)

        # Extract per-vertex data (same pattern as skinned_renderer.py)
        positions = np.zeros((n_verts, 3), dtype=np.float32)
        normals = np.zeros((n_verts, 3), dtype=np.float32)
        uvs = np.zeros((n_verts, 2), dtype=np.float32)
        weights = np.zeros((n_verts, 4), dtype=np.float32)
        bone_idx_arr = np.zeros((n_verts, 4), dtype=np.int32)
        vc_arr = np.ones((n_verts, 4), dtype=np.float32)  # Default white/opaque
        shape_has_vc = False

        for i, vd in enumerate(vertex_data_list):
            v = vd.get("Vertex") or {}
            positions[i] = [
                float(v.get("x", 0)),
                float(v.get("y", 0)),
                float(v.get("z", 0)),
            ]

            n = vd.get("Normal")
            if n:
                normals[i] = [
                    float(n.get("x", 0)),
                    float(n.get("y", 0)),
                    float(n.get("z", 0)),
                ]

            uv = vd.get("UV")
            if uv:
                uvs[i] = [float(uv.get("u", 0)), float(uv.get("v", 0))]

            # Vertex colors (ByteColor4: RGBA 0-255)
            vc = vd.get("Vertex Colors")
            if vc and isinstance(vc, dict):
                vc_arr[i] = [
                    float(vc.get("r", 255)) / 255.0,
                    float(vc.get("g", 255)) / 255.0,
                    float(vc.get("b", 255)) / 255.0,
                    float(vc.get("a", 255)) / 255.0,
                ]
                shape_has_vc = True

            # Bone weights — two formats:
            # 1. Combined: [{"index": N, "weight": F}, ...] (NiSkinInstance)
            # 2. Separate: "Bone Weights" = [f, ...], "Bone Indices" = [i, ...] (BSSkin)
            bw_list = vd.get("Bone Weights") or vd.get("BoneWeights") or []
            bi_list = vd.get("Bone Indices") or []
            if isinstance(bw_list, list):
                if bw_list and isinstance(bw_list[0], dict):
                    for j, bw in enumerate(bw_list[:4]):
                        bone_idx_arr[i, j] = int(bw.get("index", bw.get("Index", 0)))
                        weights[i, j] = float(bw.get("weight", bw.get("Weight", 0)))
                else:
                    for j in range(min(4, len(bw_list))):
                        weights[i, j] = float(bw_list[j])
                    for j in range(min(4, len(bi_list))):
                        bone_idx_arr[i, j] = int(bi_list[j])

        # For unskinned shapes, bake the parent->shape world transform into
        # positions/normals so downstream tools see world-space coordinates.
        # Skinned shapes are left alone; the caller's bind-pose pipeline
        # handles their world placement.
        skin_ref = shape.get_field("Skin")
        has_skin = skin_ref is not None and int(skin_ref) >= 0
        if not has_skin:
            world_mat = shape_world_transforms.get(id(shape))
            if world_mat is not None and not np.allclose(world_mat, np.eye(4, dtype=np.float32)):
                homog = np.concatenate(
                    [positions, np.ones((n_verts, 1), dtype=np.float32)], axis=1,
                )
                positions = (homog @ world_mat.T)[:, :3].astype(np.float32)

                rot3 = world_mat[:3, :3]
                normals = (normals @ rot3.T).astype(np.float32)
                n_lens = np.linalg.norm(normals, axis=1, keepdims=True)
                n_lens[n_lens < 1e-8] = 1.0
                normals = (normals / n_lens).astype(np.float32)

        # Extract bone names and inv_bind transforms from BSSkin
        shape_bone_names = _get_skin_bone_names(nif, shape)
        shape_inv_binds = _get_skin_inv_binds(nif, shape)

        # Remap local bone indices to global bone indices
        local_to_global: dict[int, int] = {}
        for local_idx, bname in enumerate(shape_bone_names):
            if bname not in bone_name_to_idx:
                bone_name_to_idx[bname] = len(all_bone_names)
                all_bone_names.append(bname)
                # Store inv_bind for this bone (first occurrence wins)
                if local_idx < len(shape_inv_binds):
                    all_inv_binds.append(shape_inv_binds[local_idx])
                else:
                    all_inv_binds.append(np.eye(4, dtype=np.float32))
            local_to_global[local_idx] = bone_name_to_idx[bname]

        # Remap bone indices
        for i in range(n_verts):
            for j in range(4):
                local_bi = int(bone_idx_arr[i, j])
                if local_bi in local_to_global and weights[i, j] > 0:
                    bone_idx_arr[i, j] = local_to_global[local_bi]
                elif weights[i, j] <= 0:
                    bone_idx_arr[i, j] = 0

        # Extract triangles with offset
        tris = []
        for tri in triangles_list:
            if isinstance(tri, dict):
                tris.append([
                    int(tri.get("v1", tri.get("V1", 0))) + vertex_offset,
                    int(tri.get("v2", tri.get("V2", 0))) + vertex_offset,
                    int(tri.get("v3", tri.get("V3", 0))) + vertex_offset,
                ])
            elif isinstance(tri, (list, tuple)) and len(tri) >= 3:
                tris.append([
                    int(tri[0]) + vertex_offset,
                    int(tri[1]) + vertex_offset,
                    int(tri[2]) + vertex_offset,
                ])

        n_shape_tris = len(tris)
        shape_seg_ids = np.full(n_shape_tris, -1, dtype=np.int32)

        # --- Extract segment data ---
        shape_segments, shape_ssf = _extract_fo4_segments(nif, shape, n_shape_tris, tri_offset)
        if shape_segments:
            # FO4 BSSubIndexTriShape: assign segment indices
            seg_id_offset = len(all_segments)
            _assign_segment_ids(shape_seg_ids, shape_segments, seg_id_offset)
            all_segments.extend(shape_segments)
            if shape_ssf:
                all_ssf = shape_ssf
        else:
            # Try Skyrim BSDismemberSkinInstance path
            dismember_parts = _extract_dismember_partitions(nif, shape, n_shape_tris)
            if dismember_parts is not None:
                shape_seg_ids = dismember_parts

        all_segment_ids.append(shape_seg_ids)
        all_verts.append(positions)
        all_normals.append(normals)
        all_uvs.append(uvs)
        all_weights.append(weights)
        all_bone_indices.append(bone_idx_arr)
        all_vertex_colors.append(vc_arr)
        if shape_has_vc:
            has_any_vertex_colors = True
        if tris:
            all_tris.append(np.array(tris, dtype=np.uint32))

        vertex_offset += n_verts
        tri_offset += n_shape_tris

    if not all_verts:
        raise ValueError(f"No vertex data found in {source_label}")

    # Concatenate all arrays
    merged_verts = np.concatenate(all_verts, axis=0)
    merged_normals = np.concatenate(all_normals, axis=0)
    merged_uvs = np.concatenate(all_uvs, axis=0)
    merged_weights = np.concatenate(all_weights, axis=0)
    merged_bone_indices = np.concatenate(all_bone_indices, axis=0)
    merged_tris = np.concatenate(all_tris, axis=0) if all_tris else np.empty((0, 3), dtype=np.uint32)
    merged_seg_ids = np.concatenate(all_segment_ids, axis=0) if all_segment_ids else np.empty((0,), dtype=np.int32)
    merged_vc = np.concatenate(all_vertex_colors, axis=0) if has_any_vertex_colors else None

    # Extract bone parent-child hierarchy from the NIF node tree
    bone_parents = _extract_bone_parents(nif, all_bone_names)

    return SkinData(
        vertices=merged_verts,
        triangles=merged_tris,
        normals=merged_normals,
        uvs=merged_uvs,
        bone_names=all_bone_names,
        weights=merged_weights,
        bone_indices=merged_bone_indices,
        segment_ids=merged_seg_ids,
        max_bones_per_vertex=4,
        inv_bind_transforms=all_inv_binds,
        segments=all_segments,
        ssf_file=all_ssf,
        vertex_colors=merged_vc,
        bone_parents=bone_parents,
    )


def _extract_fo4_segments(
    nif, shape, n_shape_tris: int, tri_offset: int,
) -> tuple[list[SegmentInfo], str]:
    """Extract FO4 BSSubIndexTriShape segment hierarchy.

    Returns (segments, ssf_file). Empty list if this shape has no segments.
    """
    if not nif.schema.is_subtype_of(shape.type_name, "BSSubIndexTriShape"):
        return [], ""

    segment_list = shape.get_field("Segment") or []
    if not segment_list:
        return [], ""

    segments: list[SegmentInfo] = []
    for seg_data in segment_list:
        if not isinstance(seg_data, dict):
            continue
        start_idx = int(seg_data.get("Start Index", 0))
        # Start Index is in units of indices (3 per triangle)
        tri_start = start_idx // 3
        num_prims = int(seg_data.get("Num Primitives", 0))

        sub_segments: list[SubSegmentInfo] = []
        sub_seg_list = seg_data.get("Sub Segment", [])
        if isinstance(sub_seg_list, list):
            for ss in sub_seg_list:
                if isinstance(ss, dict):
                    ss_start = int(ss.get("Start Index", 0)) // 3
                    ss_num = int(ss.get("Num Primitives", 0))
                    sub_segments.append(SubSegmentInfo(
                        start_index=ss_start + tri_offset,
                        num_primitives=ss_num,
                    ))

        segments.append(SegmentInfo(
            start_index=tri_start + tri_offset,
            num_primitives=num_prims,
            sub_segments=sub_segments,
        ))

    # Read shared segment data (SSF file, per-segment user indices / bone IDs)
    ssf_file = ""
    seg_shared = shape.get_field("Segment Data")
    if isinstance(seg_shared, dict):
        ssf_file = str(seg_shared.get("SSF File", ""))
        per_seg_data = seg_shared.get("Per Segment Data", [])
        if isinstance(per_seg_data, list):
            # Per Segment Data has one entry per segment (parent) plus one
            # entry per sub-segment.  For segments with sub-segments the
            # layout is: [parent_entry, sub0, sub1, ...].  For segments
            # without sub-segments it is just: [segment_entry].
            flat_idx = 0
            for seg in segments:
                if seg.sub_segments:
                    # Skip the parent segment entry
                    if flat_idx < len(per_seg_data):
                        flat_idx += 1
                    # Then read one entry per sub-segment
                    for ss in seg.sub_segments:
                        if flat_idx < len(per_seg_data):
                            psd = per_seg_data[flat_idx]
                            if isinstance(psd, dict):
                                ss.user_index = int(psd.get("User Index", 0))
                                ss.bone_id = int(psd.get("Bone ID", 0xFFFFFFFF))
                                co = psd.get("Cut Offsets", [])
                                if isinstance(co, list):
                                    ss.cut_offsets = [float(x) for x in co]
                            flat_idx += 1
                else:
                    # Segment without sub-segments gets one entry
                    if flat_idx < len(per_seg_data):
                        psd = per_seg_data[flat_idx]
                        if isinstance(psd, dict):
                            seg.user_index = int(psd.get("User Index", 0))
                        flat_idx += 1

    _log.info(
        "FO4 segments: %d segments, %d total sub-segments from %s",
        len(segments),
        sum(len(s.sub_segments) for s in segments),
        shape.get_field("Name") or shape.type_name,
    )
    return segments, ssf_file


def _assign_segment_ids(
    segment_ids: np.ndarray,
    segments: list[SegmentInfo],
    seg_id_offset: int = 0,
) -> None:
    """Populate per-triangle segment indices from FO4 segment hierarchy.

    Each triangle is assigned its parent segment's index (offset by
    seg_id_offset for multi-shape merging).  Sub-segment detail (user_index,
    bone_id) lives in the SegmentInfo hierarchy, not in this array.
    """
    for seg_idx, seg in enumerate(segments):
        seg_id = seg_idx + seg_id_offset
        for ti in range(seg.num_primitives):
            idx = seg.start_index + ti
            if 0 <= idx < len(segment_ids):
                segment_ids[idx] = seg_id


def _extract_dismember_partitions(
    nif, shape, n_shape_tris: int,
) -> np.ndarray | None:
    """Extract per-triangle partition IDs from BSDismemberSkinInstance + NiSkinPartition.

    Used for Skyrim NIFs. Returns None if no dismember data found.
    """
    # Get the skin instance
    skin_ref = shape.get_field("Skin Instance") or shape.get_field("Skin")
    if skin_ref is None or (isinstance(skin_ref, int) and skin_ref < 0):
        return None

    skin_id = int(skin_ref) if isinstance(skin_ref, (int, float)) else -1
    if skin_id < 0:
        return None

    skin_block = nif.get_block(skin_id)
    if skin_block is None:
        return None

    # Must be a BSDismemberSkinInstance
    if not nif.schema.is_subtype_of(skin_block.type_name, "BSDismemberSkinInstance"):
        return None

    # Read the body part list from BSDismemberSkinInstance
    bp_list = skin_block.get_field("Partitions") or []
    if not bp_list:
        return None

    # Build partition_id -> body_part mapping from BodyPartList entries
    part_body_parts: list[int] = []
    for bp in bp_list:
        if isinstance(bp, dict):
            part_body_parts.append(int(bp.get("Body Part", 0)))
        else:
            part_body_parts.append(0)

    # Get the NiSkinPartition block
    skin_partition_ref = skin_block.get_field("Skin Partition")
    if skin_partition_ref is None or (isinstance(skin_partition_ref, int) and skin_partition_ref < 0):
        # Also check via NiSkinData -> Skin Partition
        data_ref = skin_block.get_field("Data")
        if data_ref is not None and isinstance(data_ref, int) and data_ref >= 0:
            data_block = nif.get_block(int(data_ref))
            if data_block:
                skin_partition_ref = data_block.get_field("Skin Partition")
    if skin_partition_ref is None or (isinstance(skin_partition_ref, int) and skin_partition_ref < 0):
        return None

    sp_block = nif.get_block(int(skin_partition_ref))
    if sp_block is None:
        return None

    # Read partitions from NiSkinPartition
    sp_partitions = sp_block.get_field("Partitions") or sp_block.get_field("Partition") or []
    if not sp_partitions:
        return None

    partitions = np.full(n_shape_tris, -1, dtype=np.int32)

    # Build a lookup from frozenset(v0,v1,v2) -> triangle index for O(1) matching
    triangles_list = shape.get_field("Triangles") or []
    tri_lookup: dict[frozenset, int] = {}
    for i, tri in enumerate(triangles_list):
        if isinstance(tri, dict):
            key = frozenset((
                int(tri.get("v1", tri.get("V1", 0))),
                int(tri.get("v2", tri.get("V2", 0))),
                int(tri.get("v3", tri.get("V3", 0))),
            ))
        elif isinstance(tri, (list, tuple)) and len(tri) >= 3:
            key = frozenset((int(tri[0]), int(tri[1]), int(tri[2])))
        else:
            continue
        tri_lookup[key] = i

    for part_idx, sp in enumerate(sp_partitions):
        if not isinstance(sp, dict):
            continue

        # Get body part ID from BSDismemberSkinInstance
        body_part = part_body_parts[part_idx] if part_idx < len(part_body_parts) else 0

        # Get the vertex map (local -> global vertex indices)
        vertex_map = sp.get("Vertex Map", [])

        # Get triangles in this partition (local vertex indices)
        sp_tris = sp.get("Triangles", [])
        for tri in sp_tris:
            if isinstance(tri, dict):
                v0 = int(tri.get("v1", tri.get("V1", 0)))
                v1 = int(tri.get("v2", tri.get("V2", 0)))
                v2 = int(tri.get("v3", tri.get("V3", 0)))
            elif isinstance(tri, (list, tuple)) and len(tri) >= 3:
                v0, v1, v2 = int(tri[0]), int(tri[1]), int(tri[2])
            else:
                continue

            # Remap local indices to global via vertex map
            if vertex_map:
                v0 = int(vertex_map[v0]) if v0 < len(vertex_map) else v0
                v1 = int(vertex_map[v1]) if v1 < len(vertex_map) else v1
                v2 = int(vertex_map[v2]) if v2 < len(vertex_map) else v2

            tri_idx = tri_lookup.get(frozenset((v0, v1, v2)))
            if tri_idx is not None and 0 <= tri_idx < n_shape_tris:
                partitions[tri_idx] = body_part

    assigned = int(np.sum(partitions >= 0))
    _log.info(
        "Dismember partitions: %d/%d triangles assigned from %d partitions",
        assigned, n_shape_tris, len(sp_partitions),
    )
    return partitions


def _get_skin_bone_names(nif, shape_block) -> list[str]:
    """Extract bone names from BSSkin::Instance attached to a shape.

    Follows the same pattern as skinned_renderer.py's _get_skin_bone_names.
    """
    # Try "Skin Instance" first (FO4), then "Skin" (older NIFs)
    skin_ref = shape_block.get_field("Skin Instance")
    if skin_ref is None or (isinstance(skin_ref, int) and skin_ref < 0):
        skin_ref = shape_block.get_field("Skin")
    if skin_ref is None or (isinstance(skin_ref, int) and skin_ref < 0):
        return []

    skin_id = int(skin_ref) if isinstance(skin_ref, (int, float)) else -1
    if skin_id < 0:
        return []

    skin_block = nif.get_block(skin_id)
    if skin_block is None:
        return []

    bone_refs = skin_block.get_field("Bones") or []
    names: list[str] = []
    for ref in bone_refs:
        bone_id = int(ref) if isinstance(ref, (int, float)) else -1
        if bone_id >= 0:
            bone_block = nif.get_block(bone_id)
            if bone_block:
                name = bone_block.get_field("Name") or f"Bone_{bone_id}"
                if isinstance(name, int):
                    name = nif.get_string(name) or f"Bone_{bone_id}"
                names.append(str(name))
        else:
            names.append(f"Bone_{len(names)}")
    return names


def _get_skin_inv_binds(nif, shape_block) -> list[np.ndarray]:
    """Extract inverse bind pose matrices from BSSkin::BoneData."""
    skin_ref = shape_block.get_field("Skin Instance") or shape_block.get_field("Skin")
    if skin_ref is None or (isinstance(skin_ref, int) and skin_ref < 0):
        return []

    skin_id = int(skin_ref) if isinstance(skin_ref, int) else -1
    if skin_id < 0:
        return []

    skin_block = nif.get_block(skin_id)
    if skin_block is None:
        return []

    data_ref = skin_block.get_field("Data")
    if data_ref is None or (isinstance(data_ref, int) and data_ref < 0):
        return []

    data_block = nif.get_block(int(data_ref))
    if data_block is None:
        return []

    bone_list = data_block.get_field("Bone List") or []
    transforms = []
    for bone_data in bone_list:
        mat = np.eye(4, dtype=np.float32)
        rot = bone_data.get("Rotation", {})
        trans = bone_data.get("Translation", {})

        # NIF Matrix33 uses column-major naming: mCR = col C, row R
        mat[0, 0] = float(rot.get("m11", 1))
        mat[0, 1] = float(rot.get("m21", 0))
        mat[0, 2] = float(rot.get("m31", 0))
        mat[1, 0] = float(rot.get("m12", 0))
        mat[1, 1] = float(rot.get("m22", 1))
        mat[1, 2] = float(rot.get("m32", 0))
        mat[2, 0] = float(rot.get("m13", 0))
        mat[2, 1] = float(rot.get("m23", 0))
        mat[2, 2] = float(rot.get("m33", 1))

        mat[0, 3] = float(trans.get("x", 0))
        mat[1, 3] = float(trans.get("y", 0))
        mat[2, 3] = float(trans.get("z", 0))

        transforms.append(mat)
    return transforms


def _extract_bone_parents(nif, bone_names: list[str]) -> list[int]:
    """Return each bone's nearest ancestor index within ``bone_names`` from the
    NIF node tree, or -1 when it has none.
    """
    if not bone_names:
        return []

    bone_name_to_idx = {name: idx for idx, name in enumerate(bone_names)}

    # Build block_id -> name mapping for all NiNode-like blocks
    block_name: dict[int, str] = {}
    for block in nif.blocks:
        name = block.get_field("Name")
        if name and isinstance(name, str):
            block_name[block.block_id] = name

    # Build parent map: for each block, find which block has it as a child
    block_parent: dict[int, int] = {}
    for block in nif.blocks:
        children = block.get_field("Children")
        if isinstance(children, list):
            for child_ref in children:
                if isinstance(child_ref, int) and child_ref >= 0:
                    block_parent[child_ref] = block.block_id

    # Map bone names to block IDs
    name_to_block_id: dict[str, int] = {}
    for bid, name in block_name.items():
        if name in bone_name_to_idx:
            name_to_block_id[name] = bid

    # Build parent list
    parents = [-1] * len(bone_names)
    for bone_idx, bname in enumerate(bone_names):
        bid = name_to_block_id.get(bname)
        if bid is None:
            continue
        # Walk up the parent chain until we find another bone in our list
        current = bid
        while current in block_parent:
            parent_bid = block_parent[current]
            parent_name = block_name.get(parent_bid, "")
            if parent_name in bone_name_to_idx:
                parents[bone_idx] = bone_name_to_idx[parent_name]
                break
            current = parent_bid

    return parents


def load_reference_body(
    extracted_dir: str | Path,
    game: str = "fo4",
    skeleton_type: str = "Human",
    gender: str = "female",
    parts: list[str] | None = None,
) -> SkinData:
    """Load and composite reference body meshes into a single SkinData.

    Each part loads via extract_skin_data_from_nif(); vertices are concatenated
    and triangle indices remapped. ``parts`` (e.g. ["female_body",
    "female_hands"]) defaults to every part matching the ``gender`` prefix.
    Raises ValueError for an unknown game/skeleton/gender combination.
    """
    extracted_dir = Path(extracted_dir)

    profiles = SKELETON_PROFILES.get(game)
    if not profiles:
        raise ValueError(f"No skeleton profiles for game: {game}")

    profile = profiles.get(skeleton_type)
    if not profile:
        raise ValueError(f"No skeleton profile '{skeleton_type}' for game: {game}")

    body_parts = profile.get("body_parts", {})

    # Filter parts
    if parts is not None:
        selected = {k: v for k, v in body_parts.items() if k in parts}
    else:
        # Auto-select by gender
        prefix = gender.lower() + "_"
        selected = {k: v for k, v in body_parts.items() if k.startswith(prefix)}

    if not selected:
        raise ValueError(
            f"No body parts found for gender='{gender}' in {game}/{skeleton_type}. "
            f"Available: {list(body_parts.keys())}"
        )

    # Load and merge
    merged_parts: list[SkinData] = []
    for part_key, rel_path in selected.items():
        nif_path = extracted_dir / rel_path
        if not nif_path.exists():
            _log.warning("Body part NIF not found, skipping: %s", nif_path)
            continue

        try:
            skin = extract_skin_data_from_nif(nif_path)
            _log.info(
                "Loaded %s: %d verts, %d tris, %d bones",
                part_key, skin.num_vertices, skin.num_triangles, len(skin.bone_names),
            )
            merged_parts.append(skin)
        except Exception as e:
            _log.warning("Failed to load %s: %s", part_key, e)

    if not merged_parts:
        raise ValueError("No body parts could be loaded")

    if len(merged_parts) == 1:
        return merged_parts[0]

    # Merge multiple SkinData
    return _merge_skin_data(merged_parts)


def _merge_skin_data(parts: list[SkinData]) -> SkinData:
    """Merge multiple SkinData instances into one.

    Concatenates vertices, remaps triangle indices and bone indices to a
    unified bone name list.
    """
    all_bone_names: list[str] = []
    bone_name_to_idx: dict[str, int] = {}

    # First pass: build unified bone name list + inv_bind
    all_inv_binds: list[np.ndarray] = []
    for part in parts:
        for local_idx, name in enumerate(part.bone_names):
            if name not in bone_name_to_idx:
                bone_name_to_idx[name] = len(all_bone_names)
                all_bone_names.append(name)
                if local_idx < len(part.inv_bind_transforms):
                    all_inv_binds.append(part.inv_bind_transforms[local_idx])
                else:
                    all_inv_binds.append(np.eye(4, dtype=np.float32))

    # Second pass: remap and concatenate
    all_verts: list[np.ndarray] = []
    all_normals: list[np.ndarray] = []
    all_uvs: list[np.ndarray] = []
    all_tris: list[np.ndarray] = []
    all_weights: list[np.ndarray] = []
    all_bi: list[np.ndarray] = []
    all_parts_arr: list[np.ndarray] = []
    all_segments: list[SegmentInfo] = []
    merged_ssf = ""

    vertex_offset = 0
    for part in parts:
        all_verts.append(part.vertices)
        all_normals.append(part.normals)
        all_uvs.append(part.uvs)
        all_weights.append(part.weights)
        all_parts_arr.append(part.segment_ids)
        all_segments.extend(part.segments)
        if part.ssf_file:
            merged_ssf = part.ssf_file

        # Remap bone indices via lookup table (single pass to avoid overlap corruption)
        max_local = int(part.bone_indices.max()) + 1 if part.bone_indices.size > 0 else len(part.bone_names)
        remap = np.arange(max(max_local, len(part.bone_names)), dtype=np.int32)
        for local_idx, name in enumerate(part.bone_names):
            remap[local_idx] = bone_name_to_idx[name]
        bi = remap[part.bone_indices]
        all_bi.append(bi)

        # Offset triangle indices
        if part.num_triangles > 0:
            tris = part.triangles.copy().astype(np.uint32)
            tris += vertex_offset
            all_tris.append(tris)

        vertex_offset += part.num_vertices

    return SkinData(
        vertices=np.concatenate(all_verts, axis=0),
        triangles=np.concatenate(all_tris, axis=0) if all_tris else np.empty((0, 3), dtype=np.uint32),
        normals=np.concatenate(all_normals, axis=0),
        uvs=np.concatenate(all_uvs, axis=0),
        bone_names=all_bone_names,
        weights=np.concatenate(all_weights, axis=0),
        bone_indices=np.concatenate(all_bi, axis=0),
        segment_ids=np.concatenate(all_parts_arr, axis=0) if all_parts_arr else np.empty((0,), dtype=np.int32),
        max_bones_per_vertex=4,
        inv_bind_transforms=all_inv_binds,
        segments=all_segments,
        ssf_file=merged_ssf,
    )
