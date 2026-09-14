"""Game profile system for multi-game NIF support.

Each game has a frozen dataclass containing all per-game configuration.
Profiles are registered at module load and looked up by ID or BS version.
"""

from __future__ import annotations

from dataclasses import dataclass, field

# Game units per Havok unit. Skyrim/FO4-era engines use 69.99125; the
# Gamebryo-era games (Oblivion, FO3, FNV) use a tenth of that. Matches pynifly's
# `game_collision_sf` and is confirmed by FNV collision hulls registering exactly
# against their visible meshes at this factor.
MODERN_HAVOK_SCALE = 69.99125
LEGACY_HAVOK_SCALE = MODERN_HAVOK_SCALE / 10.0


@dataclass(frozen=True)
class RemixProfile:
    """FO76 texture-channel remix configuration shared with cdb_to_bgsm.

    `role_formats` is a tuple-of-tuples (not a dict) so the dataclass stays
    hashable/frozen-friendly. Use :meth:`role_format_map` for dict access.
    """

    ao_multiplier: float = 1.0
    specular_multiplier: float = 1.0
    gloss_multiplier: float = 1.0
    spec_offset: float = 0.0
    role_formats: tuple[tuple[str, str], ...] = ()

    def role_format_map(self) -> dict[str, str]:
        return dict(self.role_formats)


@dataclass(frozen=True)
class GameProfile:
    id: str
    display_name: str
    bs_version_range: tuple[int, int]
    nif_version: tuple[int, int, int, int]
    user_version: int

    # Textures
    texture_slot_map: dict[str, int]
    texture_suffixes: dict[str, str]
    material_format: str  # "bgsm" | "mat"
    material_model: str  # "spec-gloss" | "metallic-roughness"
    normal_has_blue_channel: bool

    # Shaders
    shader_modules: list[str]
    effect_shader_modules: list[str]

    # Physics
    havok_scale: float | None
    havok_version: str | None
    collision_layer_enum: str
    physics_material_enum: str

    # Archives
    archive_format: str  # "ba2" | "bsa"
    archive_extensions: list[str]

    # Paths
    env_var_name: str
    default_texture_prefix: str
    default_cubemap_path: str

    # Lighting defaults
    default_lighting_preset: str
    default_lighting_tuning: str

    # Papyrus scripting
    papyrus_compiler_dir: str | None = (
        None  # Relative dir within game install (e.g. "Papyrus Compiler")
    )
    papyrus_flags: str | None = (
        None  # Flags file name (e.g. "Institute_Papyrus_Flags.flg")
    )
    papyrus_script_db: str | None = (
        None  # Script DB filename in db/data/ (e.g. "fo4_scripts.db")
    )
    papyrus_source_subpath: str | None = (
        None  # Path from game root to scripts source dir (game-specific)
    )

    # Havok version conversion
    havok_version_id: int | None = (
        None  # SDK version ID (e.g. 53 for FO4, 46 for Skyrim SE)
    )
    has_havok_behaviors: bool = (
        False  # True if game uses Havok behavior graphs (FO4, FO76, Starfield)
    )

    # Multi-game tooling
    engine: str = "creation1"  # "gamebryo" | "creation1" | "creation2"
    master_esm: str | None = None  # "Fallout4.esm" | "Skyrim.esm" | etc.
    executable_name: str | None = None  # "Fallout4.exe" | "SkyrimSE.exe" | etc.
    steam_app_id: int | None = None  # 377160 | 489830 | etc.
    is_moddable: bool = True  # False for FO76 (asset source only)
    wiki_dir: str | None = None  # "fo4_wiki" | "skyrim_wiki" | "fo3_nv_wiki" | None

    # FO76 conversion (refs integration)
    texture_remix: RemixProfile | None = (
        None  # Per-game DDS channel remix; None disables.
    )
    bgsm_version: int = 2  # BGSM header version this game writes
    bgem_version: int = 2  # BGEM header version this game writes
    asset_prefix: str = ""

    # Voice / dialogue audio
    voice_official_masters: tuple[str, ...] = ()
    voice_container: str = "wav"  # "fuz" | "ogg" | "wav"
    voice_lip: str | None = None  # "embedded" | "sidecar" | None
    facefx_game: str | None = None  # FaceFXWrapper's <Type> argument


# ---------------------------------------------------------------------------
# Profile constants
# ---------------------------------------------------------------------------

FO4_PROFILE = GameProfile(
    id="fo4",
    display_name="Fallout 4",
    bs_version_range=(130, 139),
    nif_version=(20, 2, 0, 7),
    user_version=12,
    texture_slot_map={
        "diffuse": 0,
        "normal": 1,
        "glow": 2,
        "greyscale": 3,
        "cubemap": 4,
        "envmask": 5,
        "subsurface": 6,
        "specular": 7,
    },
    texture_suffixes={
        "diffuse": "_d",
        "normal": "_n",
        "glow": "_g",
        "specular": "_s",
        "subsurface": "_sk",
    },
    material_format="bgsm",
    material_model="spec-gloss",
    normal_has_blue_channel=False,
    shader_modules=[
        "common",
        "lighting",
        "normal_decode",
        "fresnel",
        "tonemapping",
        "cubemap",
        "specgloss",
    ],
    effect_shader_modules=["common", "effect_emissive"],
    havok_scale=69.99125,
    havok_version=None,
    collision_layer_enum="Fallout4Layer",
    physics_material_enum="Fallout4HavokMaterial",
    archive_format="ba2",
    archive_extensions=[".ba2"],
    env_var_name="FO4_EXTRACTED_DIR",
    default_texture_prefix="Textures/",
    default_cubemap_path="Textures/Shared/Cubemaps/",
    default_lighting_preset="studio",
    default_lighting_tuning="Fallout 4",
    papyrus_compiler_dir="Papyrus Compiler",
    papyrus_flags="Institute_Papyrus_Flags.flg",
    papyrus_script_db="fo4_scripts.db",
    papyrus_source_subpath="Data/Scripts/Source",
    havok_version_id=53,
    has_havok_behaviors=True,
    engine="creation1",
    master_esm="Fallout4.esm",
    executable_name="Fallout4.exe",
    steam_app_id=377160,
    is_moddable=True,
    wiki_dir="fo4_wiki",
    voice_official_masters=(
        "Fallout4.esm",
        "DLCRobot.esm",
        "DLCworkshop01.esm",
        "DLCCoast.esm",
        "DLCworkshop02.esm",
        "DLCworkshop03.esm",
        "DLCNukaWorld.esm",
    ),
    voice_container="fuz",
    voice_lip="embedded",
    facefx_game="Fallout4",
)

SKYRIMSE_PROFILE = GameProfile(
    id="skyrimse",
    display_name="Skyrim Special Edition",
    bs_version_range=(100, 109),
    nif_version=(20, 2, 0, 7),
    user_version=12,
    texture_slot_map={
        "diffuse": 0,
        "normal": 1,
        "glow": 2,
        "greyscale": 3,
        "cubemap": 4,
        "envmask": 5,
        "subsurface": 6,
        "specular": 7,
    },
    texture_suffixes={
        "diffuse": "_d",
        "normal": "_n",
        "glow": "_g",
        "specular": "_s",
        "subsurface": "_sk",
    },
    material_format="bgsm",
    material_model="spec-gloss",
    normal_has_blue_channel=False,
    shader_modules=[
        "common",
        "lighting",
        "normal_decode",
        "fresnel",
        "tonemapping",
        "cubemap",
        "specgloss",
    ],
    effect_shader_modules=["common", "effect_emissive"],
    havok_scale=69.99125,
    havok_version=None,
    collision_layer_enum="SkyrimLayer",
    physics_material_enum="SkyrimHavokMaterial",
    archive_format="bsa",
    archive_extensions=[".bsa"],
    env_var_name="SKYRIMSE_EXTRACTED_DIR",
    default_texture_prefix="Textures/",
    default_cubemap_path="Textures/Cubemaps/",
    default_lighting_preset="studio",
    default_lighting_tuning="Skyrim SE",
    papyrus_compiler_dir="Papyrus Compiler",
    papyrus_flags="TESV_Papyrus_Flags.flg",
    papyrus_script_db="skyrimse_scripts.db",
    papyrus_source_subpath="Data/Source/Scripts",
    havok_version_id=46,
    engine="creation1",
    master_esm="Skyrim.esm",
    executable_name="SkyrimSE.exe",
    steam_app_id=489830,
    is_moddable=True,
    wiki_dir="skyrim_wiki",
    voice_official_masters=(
        "Skyrim.esm",
        "Update.esm",
        "Dawnguard.esm",
        "HearthFires.esm",
        "Dragonborn.esm",
        "ccBGSSSE001-Fish.esm",
        "ccBGSSSE025-AdvDSGS.esm",
    ),
    voice_container="fuz",
    voice_lip="embedded",
    facefx_game="Skyrim",
)

FO76_PROFILE = GameProfile(
    id="fo76",
    display_name="Fallout 76",
    bs_version_range=(150, 159),
    nif_version=(20, 2, 0, 7),
    user_version=12,
    texture_slot_map={
        "diffuse": 0,
        "normal": 1,
        "glow": 2,
        "greyscale": 3,
        "cubemap": 4,
        "envmask": 5,
        "subsurface": 6,
        "specular": 7,
        "reflectivity": 8,
        "lighting": 9,
    },
    texture_suffixes={
        "diffuse": "_d",
        "normal": "_n",
        "glow": "_g",
        "lighting": "_l",
        "reflectivity": "_r",
    },
    material_format="bgsm",
    material_model="metallic-roughness",
    normal_has_blue_channel=True,
    shader_modules=[
        "common",
        "lighting",
        "normal_decode",
        "fresnel",
        "tonemapping",
        "cubemap",
        "metalrough",
    ],
    effect_shader_modules=["common", "effect_emissive"],
    havok_scale=69.99125,
    havok_version=None,
    collision_layer_enum="Fallout76Layer",
    physics_material_enum="Fallout76HavokMaterial",
    archive_format="ba2",
    archive_extensions=[".ba2"],
    env_var_name="FO76_EXTRACTED_DIR",
    default_texture_prefix="Textures/",
    default_cubemap_path="Textures/Shared/Cubemaps/",
    default_lighting_preset="studio",
    default_lighting_tuning="Fallout 76",
    papyrus_compiler_dir=None,
    papyrus_flags=None,
    papyrus_script_db="fo76_scripts.db",
    havok_version_id=56,
    has_havok_behaviors=True,
    engine="creation1",
    master_esm="SeventySix.esm",
    executable_name="Fallout76.exe",
    steam_app_id=1151340,
    is_moddable=False,
    wiki_dir=None,
    texture_remix=RemixProfile(
        ao_multiplier=0.5,
        specular_multiplier=1.0,
        gloss_multiplier=1.0,
        spec_offset=0.8,
        role_formats=(
            ("d", "BC7_UNORM_SRGB"),
            ("n", "BC5_UNORM"),
            ("r", "BC7_UNORM"),
            ("l", "BC4_UNORM"),
            ("e", "BC7_UNORM_SRGB"),
            ("m", "BC4_UNORM"),
            ("g", "BC7_UNORM_SRGB"),
            ("s", "BC7_UNORM"),
        ),
    ),
    bgsm_version=20,
    bgem_version=20,
)

STARFIELD_PROFILE = GameProfile(
    id="starfield",
    display_name="Starfield",
    bs_version_range=(170, 179),
    nif_version=(20, 2, 0, 7),
    user_version=12,
    texture_slot_map={
        # Keyed by source suffix, not role name (unlike the other profiles) --
        # the remix dispatches on file suffix directly. Values are the FO4
        # BSTriShape slot each suffix's data lands in (0=diffuse, 1=normal,
        # 2=glow, 7=specular). _rough/_metal merge into the specular/gloss
        # slot; _ao multiplies into diffuse rather than owning a slot.
        "_color": 0,
        "_normal": 1,
        "_rough": 7,
        "_metal": 7,
        "_ao": 0,
        "_emissive": 2,
    },
    texture_suffixes={
        "diffuse": "_color",
        "normal": "_normal",
        "roughness": "_rough",
        "metallic": "_metal",
        "ao": "_ao",
        "glow": "_emissive",
    },
    material_format="mat",
    material_model="metallic-roughness",
    normal_has_blue_channel=True,
    shader_modules=[
        "common",
        "lighting",
        "normal_decode",
        "fresnel",
        "tonemapping",
        "cubemap",
        "starfield_layered",
    ],
    effect_shader_modules=["common", "effect_emissive"],
    havok_scale=1.0,  # Tagged format (Havok 2019+): vertices already in NIF-space
    havok_version=None,
    collision_layer_enum="StarfieldLayer",
    physics_material_enum="StarfieldHavokMaterial",
    archive_format="ba2",
    archive_extensions=[".ba2"],
    env_var_name="STARFIELD_EXTRACTED_DIR",
    default_texture_prefix="Textures/",
    default_cubemap_path="Textures/Cubemaps/",
    default_lighting_preset="studio",
    default_lighting_tuning="Starfield",
    papyrus_compiler_dir="Tools/Papyrus Compiler",
    papyrus_flags="Starfield_Papyrus_Flags.flg",
    papyrus_script_db="starfield_scripts.db",
    papyrus_source_subpath="Data/Scripts/Source",
    has_havok_behaviors=True,
    engine="creation2",
    master_esm="Starfield.esm",
    executable_name="Starfield.exe",
    steam_app_id=1716740,
    is_moddable=True,
    wiki_dir=None,
    # Starfield CDB materials are the same CE2Material shape FO76 uses
    # (see cdb_to_bgsm.py); reuse FO76's proven PBR->spec-gloss tuning and
    # BC-format table until real Starfield render-compare data says otherwise.
    texture_remix=RemixProfile(
        ao_multiplier=0.5,
        specular_multiplier=1.0,
        gloss_multiplier=1.0,
        spec_offset=0.8,
        role_formats=(
            ("d", "BC7_UNORM_SRGB"),
            ("n", "BC5_UNORM"),
            ("r", "BC7_UNORM"),
            ("l", "BC4_UNORM"),
            ("e", "BC7_UNORM_SRGB"),
            ("m", "BC4_UNORM"),
            ("g", "BC7_UNORM_SRGB"),
            ("s", "BC7_UNORM"),
        ),
    ),
    voice_official_masters=(
        "Starfield.esm",
        "BlueprintShips-Starfield.esm",
        "OldMars.esm",
        "SFBGS003.esm",
        "SFBGS004.esm",
        "SFBGS006.esm",
        "SFBGS007.esm",
        "SFBGS008.esm",
        "SFBGS00D.esm",
        "SFBGS047.esm",
    ),
    voice_container="wav",
)

OBLIVION_PROFILE = GameProfile(
    id="oblivion",
    display_name="Oblivion",
    bs_version_range=(10, 10),
    nif_version=(20, 0, 0, 5),
    user_version=11,
    texture_slot_map={
        "diffuse": 0,
        "normal": 1,
        "glow": 2,
        "greyscale": 3,
    },
    texture_suffixes={"diffuse": "_d", "normal": "_n"},
    material_format="bgsm",
    material_model="spec-gloss",
    normal_has_blue_channel=False,
    shader_modules=["common", "lighting", "normal_decode", "specgloss"],
    effect_shader_modules=["common", "effect_emissive"],
    havok_scale=LEGACY_HAVOK_SCALE,
    havok_version=None,
    collision_layer_enum="OblivionLayer",
    physics_material_enum="OblivionHavokMaterial",
    archive_format="bsa",
    archive_extensions=[".bsa"],
    env_var_name="OBLIVION_EXTRACTED_DIR",
    default_texture_prefix="Textures/",
    default_cubemap_path="Textures/",
    default_lighting_preset="studio",
    default_lighting_tuning="Oblivion",
    papyrus_compiler_dir=None,
    papyrus_flags=None,
    papyrus_script_db=None,
    havok_version_id=None,
    has_havok_behaviors=False,
    engine="gamebryo",
    master_esm="Oblivion.esm",
    executable_name="Oblivion.exe",
    steam_app_id=22330,
    is_moddable=True,
    wiki_dir=None,
)

FO3_PROFILE = GameProfile(
    id="fo3",
    display_name="Fallout 3",
    bs_version_range=(11, 11),
    nif_version=(20, 2, 0, 7),
    user_version=11,
    texture_slot_map={
        "diffuse": 0,
        "normal": 1,
        "glow": 2,
        "greyscale": 3,
    },
    texture_suffixes={"diffuse": "_d", "normal": "_n"},
    material_format="bgsm",
    material_model="spec-gloss",
    normal_has_blue_channel=False,
    shader_modules=["common", "lighting", "normal_decode", "specgloss"],
    effect_shader_modules=["common", "effect_emissive"],
    havok_scale=LEGACY_HAVOK_SCALE,
    havok_version=None,
    collision_layer_enum="Fallout3Layer",
    physics_material_enum="Fallout3HavokMaterial",
    archive_format="bsa",
    archive_extensions=[".bsa"],
    env_var_name="FO3_EXTRACTED_DIR",
    default_texture_prefix="Textures/",
    default_cubemap_path="Textures/",
    default_lighting_preset="studio",
    default_lighting_tuning="Fallout 3",
    papyrus_compiler_dir=None,
    papyrus_flags=None,
    papyrus_script_db=None,
    havok_version_id=None,
    has_havok_behaviors=False,
    engine="gamebryo",
    master_esm="Fallout3.esm",
    executable_name="Fallout3.exe",
    steam_app_id=22370,
    is_moddable=True,
    wiki_dir="fo3_nv_wiki",
    voice_official_masters=(
        "Fallout3.esm",
        "Anchorage.esm",
        "ThePitt.esm",
        "BrokenSteel.esm",
        "PointLookout.esm",
        "Zeta.esm",
    ),
    voice_container="ogg",
    voice_lip="sidecar",
    # FaceFXWrapper has no FO3/FNV type; its Skyrim generator writes the same
    # version-1 lip header these games use. Verified structurally only - in-game
    # facial animation is unconfirmed. Reuse of the original .lip is preferred
    # when regenerating an existing line.
    facefx_game="Skyrim",
)

FNV_PROFILE = GameProfile(
    id="fnv",
    display_name="Fallout: New Vegas",
    bs_version_range=(34, 34),
    nif_version=(20, 2, 0, 7),
    user_version=11,
    texture_slot_map={
        "diffuse": 0,
        "normal": 1,
        "glow": 2,
        "greyscale": 3,
    },
    texture_suffixes={"diffuse": "_d", "normal": "_n"},
    material_format="bgsm",
    material_model="spec-gloss",
    normal_has_blue_channel=False,
    shader_modules=["common", "lighting", "normal_decode", "specgloss"],
    effect_shader_modules=["common", "effect_emissive"],
    havok_scale=LEGACY_HAVOK_SCALE,
    havok_version=None,
    collision_layer_enum="FalloutNVLayer",
    physics_material_enum="FalloutNVHavokMaterial",
    archive_format="bsa",
    archive_extensions=[".bsa"],
    env_var_name="FONV_EXTRACTED_DIR",
    default_texture_prefix="Textures/",
    default_cubemap_path="Textures/",
    default_lighting_preset="studio",
    default_lighting_tuning="Fallout: New Vegas",
    papyrus_compiler_dir=None,
    papyrus_flags=None,
    papyrus_script_db=None,
    havok_version_id=None,
    has_havok_behaviors=False,
    engine="gamebryo",
    master_esm="FalloutNV.esm",
    executable_name="FalloutNV.exe",
    steam_app_id=22380,
    is_moddable=True,
    wiki_dir="fo3_nv_wiki",
    voice_official_masters=(
        "FalloutNV.esm",
        "DeadMoney.esm",
        "HonestHearts.esm",
        "OldWorldBlues.esm",
        "LonesomeRoad.esm",
        "GunRunnersArsenal.esm",
        "ClassicPack.esm",
        "MercenaryPack.esm",
        "TribalPack.esm",
        "CaravanPack.esm",
    ),
    voice_container="ogg",
    voice_lip="sidecar",
    # FaceFXWrapper has no FO3/FNV type; its Skyrim generator writes the same
    # version-1 lip header these games use. Verified structurally only - in-game
    # facial animation is unconfirmed. Reuse of the original .lip is preferred
    # when regenerating an existing line.
    facefx_game="Skyrim",
)

# ---------------------------------------------------------------------------
# Registry
# ---------------------------------------------------------------------------

GAME_PROFILES: dict[str, GameProfile] = {}


def register_game(profile: GameProfile) -> None:
    """Register a game profile in the global registry."""
    GAME_PROFILES[profile.id] = profile


def get_profile(game_id: str) -> GameProfile:
    """Look up a profile by ID. Raises KeyError if not found."""
    return GAME_PROFILES[game_id]


def detect_game(bs_version: int) -> GameProfile | None:
    """Detect game from a BS version number. Returns None if unknown."""
    for profile in GAME_PROFILES.values():
        lo, hi = profile.bs_version_range
        if lo <= bs_version <= hi:
            return profile
    return None


# Register all built-in profiles
for _p in (
    FO4_PROFILE,
    SKYRIMSE_PROFILE,
    FO76_PROFILE,
    STARFIELD_PROFILE,
    OBLIVION_PROFILE,
    FO3_PROFILE,
    FNV_PROFILE,
):
    register_game(_p)
