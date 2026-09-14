"""Auto-generated detector schema for oblivion.

# @schema_forge:generated
# regen_at: 2026-08-07T13:56:34Z
# source_hash: d2387a2ab3ebda5457d290d9db846b26d5883deaddbcf1eed1bd0947a14ce6a2
"""
from __future__ import annotations

from creation_lib.esp.schema.base import ArraySpec, ConditionSpec, EnumDef, FieldSpec, GameSchema, RecordFlagBit, RecordFlagsSpec, RecordSpec, SubrecordSpec, TargetMapEntry, UnionVariantSpec
from creation_lib.esp.schema.common import COMMON_ENUMS, COMMON_RECORDS
from creation_lib.esp.schema.kinds import FieldKind

GAME_ID = 'oblivion'
HEADER_VERSION = 1.0
LOCALIZED_SUPPORT = False
EXTENDS = None


def build_schema() -> GameSchema:
    records: dict[str, RecordSpec] = dict(COMMON_RECORDS)
    enums: dict[str, EnumDef] = dict(COMMON_ENUMS)

    enums['ALCH.ENIT.flags'] = EnumDef(
        name='ALCH.ENIT.flags',
        values=((1, 'no_auto_calculate'), (2, 'food_item')),
        labels=((1, 'No Auto-Calculate'), (2, 'Food Item')),
        storage_kind='flags',
        byte_width=1,
    )
    enums['APPA.DATA.type'] = EnumDef(
        name='APPA.DATA.type',
        values=((0, 'mortar_pestle'), (1, 'alembic'), (2, 'calcinator'), (3, 'retort')),
        labels=((0, 'Mortar & Pestle'), (1, 'Alembic'), (2, 'Calcinator'), (3, 'Retort')),
        byte_width=1,
        default_value=0,
    )
    enums['ARMO.BMDT.general_flags'] = EnumDef(
        name='ARMO.BMDT.general_flags',
        values=((1, 'hide_rings'), (2, 'hide_amulets'), (64, 'non_playable'), (128, 'heavy_armor')),
        labels=((1, 'Hide Rings'), (2, 'Hide Amulets'), (64, 'Non-Playable'), (128, 'Heavy armor')),
        storage_kind='flags',
        byte_width=1,
    )
    enums['BOOK.DATA.flags'] = EnumDef(
        name='BOOK.DATA.flags',
        values=((1, 'scroll'), (2, 'can_t_be_taken')),
        labels=((1, 'Scroll'), (2, 'Can\'t be taken')),
        storage_kind='flags',
        byte_width=1,
    )
    enums['CELL.DATA.flags'] = EnumDef(
        name='CELL.DATA.flags',
        values=((1, 'is_interior_cell'), (2, 'has_water'), (4, 'can_t_travel_from_here'), (8, 'force_hide_land_exterior_oblivion_interior_interior'), (32, 'public_area'), (64, 'hand_changed'), (128, 'behave_like_exterior')),
        labels=((1, 'Is Interior Cell'), (2, 'Has Water'), (4, 'Can\'t Travel From Here'), (8, 'Force Hide Land (Exterior) / Oblivion Interior (Interior)'), (32, 'Public Area'), (64, 'Hand Changed'), (128, 'Behave Like Exterior')),
        storage_kind='flags',
        byte_width=1,
    )
    enums['CLAS.DATA.flags'] = EnumDef(
        name='CLAS.DATA.flags',
        values=((1, 'playable'), (2, 'guard')),
        labels=((1, 'Playable'), (2, 'Guard')),
        storage_kind='flags',
    )
    enums['CLOT.BMDT.general_flags'] = EnumDef(
        name='CLOT.BMDT.general_flags',
        values=((1, 'hide_rings'), (2, 'hide_amulets'), (64, 'non_playable')),
        labels=((1, 'Hide Rings'), (2, 'Hide Amulets'), (64, 'Non-Playable')),
        storage_kind='flags',
        byte_width=1,
    )
    enums['CONT.DATA.flags'] = EnumDef(
        name='CONT.DATA.flags',
        values=((2, 'respawns'),),
        labels=((2, 'Respawns'),),
        storage_kind='flags',
        byte_width=1,
    )
    enums['CREA.ACBS.flags'] = EnumDef(
        name='CREA.ACBS.flags',
        values=((1, 'biped'), (2, 'essential'), (4, 'weapon_shield'), (8, 'respawn'), (16, 'swims'), (32, 'flies'), (64, 'walks'), (128, 'pc_level_offset'), (512, 'no_low_level_processing'), (2048, 'no_blood_spray'), (4096, 'no_blood_decal'), (32768, 'no_head'), (65536, 'no_right_arm'), (131072, 'no_left_arm'), (262144, 'no_combat_in_water'), (524288, 'no_shadow'), (1048576, 'no_corpse_check')),
        labels=((1, 'Biped'), (2, 'Essential'), (4, 'Weapon & Shield'), (8, 'Respawn'), (16, 'Swims'), (32, 'Flies'), (64, 'Walks'), (128, 'PC Level Offset'), (512, 'No Low Level Processing'), (2048, 'No Blood Spray'), (4096, 'No Blood Decal'), (32768, 'No Head'), (65536, 'No Right Arm'), (131072, 'No Left Arm'), (262144, 'No Combat in Water'), (524288, 'No Shadow'), (1048576, 'No Corpse Check')),
        storage_kind='flags',
    )
    enums['CREA.CSDT.type'] = EnumDef(
        name='CREA.CSDT.type',
        values=((0, 'left_foot'), (1, 'right_foot'), (2, 'left_back_foot'), (3, 'right_back_foot'), (4, 'idle'), (5, 'aware'), (6, 'attack'), (7, 'hit'), (8, 'death'), (9, 'weapon')),
        labels=((0, 'Left Foot'), (1, 'Right Foot'), (2, 'Left Back Foot'), (3, 'Right Back Foot'), (4, 'Idle'), (5, 'Aware'), (6, 'Attack'), (7, 'Hit'), (8, 'Death'), (9, 'Weapon')),
        default_value=0,
    )
    enums['CREA.DATA.type'] = EnumDef(
        name='CREA.DATA.type',
        values=((0, 'creature'), (1, 'daedra'), (2, 'undead'), (3, 'humanoid'), (4, 'horse'), (5, 'giant')),
        labels=((0, 'Creature'), (1, 'Daedra'), (2, 'Undead'), (3, 'Humanoid'), (4, 'Horse'), (5, 'Giant')),
        byte_width=1,
        default_value=0,
    )
    enums['CSTY.CSTD.flags'] = EnumDef(
        name='CSTY.CSTD.flags',
        values=((1, 'advanced'), (2, 'choose_attack_using_chance'), (4, 'ignore_allies_in_area'), (8, 'will_yield'), (16, 'rejects_yields'), (32, 'fleeing_disabled'), (64, 'prefers_ranged'), (128, 'melee_alert_ok')),
        labels=((1, 'Advanced'), (2, 'Choose Attack using % Chance'), (4, 'Ignore Allies in Area'), (8, 'Will Yield'), (16, 'Rejects Yields'), (32, 'Fleeing Disabled'), (64, 'Prefers Ranged'), (128, 'Melee Alert OK')),
        storage_kind='flags',
        byte_width=1,
    )
    enums['DOOR.FNAM.flags'] = EnumDef(
        name='DOOR.FNAM.flags',
        values=((1, 'oblivion_gate'), (2, 'automatic_door'), (4, 'hidden'), (8, 'minimal_use')),
        labels=((1, 'Oblivion Gate'), (2, 'Automatic Door'), (4, 'Hidden'), (8, 'Minimal Use')),
        storage_kind='flags',
        byte_width=1,
    )
    enums['EFSH.DATA.flags'] = EnumDef(
        name='EFSH.DATA.flags',
        values=((1, 'no_membrane_shader'), (8, 'no_particle_shader'), (16, 'edge_effect_inverse'), (32, 'membrane_shader_affect_skin_only')),
        labels=((1, 'No Membrane Shader'), (8, 'No Particle Shader'), (16, 'Edge Effect - Inverse'), (32, 'Membrane Shader - Affect Skin Only')),
        storage_kind='flags',
        byte_width=1,
    )
    enums['ENCH.ENIT.type'] = EnumDef(
        name='ENCH.ENIT.type',
        values=((0, 'scroll'), (1, 'staff'), (2, 'weapon'), (3, 'apparel')),
        labels=((0, 'Scroll'), (1, 'Staff'), (2, 'Weapon'), (3, 'Apparel')),
        default_value=0,
    )
    enums['FACT.DATA.flags'] = EnumDef(
        name='FACT.DATA.flags',
        values=((1, 'hidden_from_player'), (2, 'evil'), (4, 'special_combat')),
        labels=((1, 'Hidden from Player'), (2, 'Evil'), (4, 'Special Combat')),
        storage_kind='flags',
        byte_width=1,
    )
    enums['GRAS.DATA.flags'] = EnumDef(
        name='GRAS.DATA.flags',
        values=((1, 'vertex_lighting'), (2, 'uniform_scaling'), (4, 'fit_to_slope')),
        labels=((1, 'Vertex Lighting'), (2, 'Uniform Scaling'), (4, 'Fit to Slope')),
        storage_kind='flags',
        byte_width=1,
    )
    enums['GRAS.DATA.unit_from_water_type'] = EnumDef(
        name='GRAS.DATA.unit_from_water_type',
        values=((0, 'above_at_least'), (1, 'above_at_most'), (2, 'below_at_least'), (3, 'below_at_most'), (4, 'either_at_least'), (5, 'either_at_most'), (6, 'either_at_most_above'), (7, 'either_at_most_below')),
        labels=((0, 'Above - At Least'), (1, 'Above - At Most'), (2, 'Below - At Least'), (3, 'Below - At Most'), (4, 'Either - At Least'), (5, 'Either - At Most'), (6, 'Either - At Most Above'), (7, 'Either - At Most Below')),
        default_value=0,
    )
    enums['HAIR.DATA.flags'] = EnumDef(
        name='HAIR.DATA.flags',
        values=((1, 'playable'), (2, 'not_male'), (4, 'not_female'), (8, 'fixed')),
        labels=((1, 'Playable'), (2, 'Not Male'), (4, 'Not Female'), (8, 'Fixed')),
        storage_kind='flags',
        byte_width=1,
    )
    enums['INFO.DATA.flags'] = EnumDef(
        name='INFO.DATA.flags',
        values=((1, 'goodbye'), (2, 'random'), (4, 'say_once'), (8, 'run_immediately'), (16, 'info_refusal'), (32, 'random_end'), (64, 'run_for_rumors')),
        labels=((1, 'Goodbye'), (2, 'Random'), (4, 'Say Once'), (8, 'Run Immediately'), (16, 'Info Refusal'), (32, 'Random End'), (64, 'Run for Rumors')),
        storage_kind='flags',
        byte_width=1,
    )
    enums['INFO.DATA.next_speaker'] = EnumDef(
        name='INFO.DATA.next_speaker',
        values=((0, 'target'), (1, 'self'), (2, 'either')),
        labels=((0, 'Target'), (1, 'Self'), (2, 'Either')),
        byte_width=1,
        default_value=0,
    )
    enums['INFO.SCHD.type'] = EnumDef(
        name='INFO.SCHD.type',
        values=((256, 'magic_effect'),),
        labels=((256, 'Magic Effect'),),
    )
    enums['INFO.SCHR.type'] = EnumDef(
        name='INFO.SCHR.type',
        values=((256, 'magic_effect'),),
        labels=((256, 'Magic Effect'),),
    )
    enums['INFO.TRDT.emotion_type'] = EnumDef(
        name='INFO.TRDT.emotion_type',
        values=((0, 'neutral'), (1, 'anger'), (2, 'disgust'), (3, 'fear'), (4, 'sad'), (5, 'happy'), (6, 'surprise')),
        labels=((0, 'Neutral'), (1, 'Anger'), (2, 'Disgust'), (3, 'Fear'), (4, 'Sad'), (5, 'Happy'), (6, 'Surprise')),
        default_value=0,
    )
    enums['INGR.ENIT.flags'] = EnumDef(
        name='INGR.ENIT.flags',
        values=((1, 'no_auto_calculate'), (2, 'food_item')),
        labels=((1, 'No Auto-Calculate'), (2, 'Food Item')),
        storage_kind='flags',
        byte_width=1,
    )
    enums['LAND.DATA.flags'] = EnumDef(
        name='LAND.DATA.flags',
        values=((1, 'has_vertex_normals_height_map'), (2, 'has_vertex_colours'), (4, 'has_layers'), (8, 'unknown_4'), (16, 'auto_calc_normals'), (1024, 'ignored')),
        labels=((1, 'Has Vertex Normals/Height Map'), (2, 'Has Vertex Colours'), (4, 'Has Layers'), (8, 'Unknown 4'), (16, 'Auto-Calc Normals'), (1024, 'Ignored')),
        storage_kind='flags',
    )
    enums['LIGH.DATA.flags'] = EnumDef(
        name='LIGH.DATA.flags',
        values=((1, 'dynamic'), (2, 'can_be_carried'), (4, 'negative'), (8, 'flicker'), (16, 'unused'), (32, 'off_by_default'), (64, 'flicker_slow'), (128, 'pulse'), (256, 'pulse_slow'), (512, 'spot_light'), (1024, 'spot_shadow')),
        labels=((1, 'Dynamic'), (2, 'Can be Carried'), (4, 'Negative'), (8, 'Flicker'), (16, 'Unused'), (32, 'Off By Default'), (64, 'Flicker Slow'), (128, 'Pulse'), (256, 'Pulse Slow'), (512, 'Spot Light'), (1024, 'Spot Shadow')),
        storage_kind='flags',
    )
    enums['LTEX.HNAM.material_type'] = EnumDef(
        name='LTEX.HNAM.material_type',
        values=((0, 'stone'), (1, 'cloth'), (2, 'dirt'), (3, 'glass'), (4, 'grass'), (5, 'metal'), (6, 'organic'), (7, 'skin'), (8, 'water'), (9, 'wood'), (10, 'heavy_stone'), (11, 'heavy_metal'), (12, 'heavy_wood'), (13, 'chain'), (14, 'snow'), (15, 'stone_stairs'), (16, 'cloth_stairs'), (17, 'dirt_stairs'), (18, 'glass_stairs'), (19, 'grass_stairs'), (20, 'metal_stairs'), (21, 'organic_stairs'), (22, 'skin_stairs'), (23, 'water_stairs'), (24, 'wood_stairs'), (25, 'heavy_stone_stairs'), (26, 'heavy_metal_stairs'), (27, 'heavy_wood_stairs'), (28, 'chain_stairs'), (29, 'snow_stairs'), (30, 'elevator')),
        labels=((0, 'Stone'), (1, 'Cloth'), (2, 'Dirt'), (3, 'Glass'), (4, 'Grass'), (5, 'Metal'), (6, 'Organic'), (7, 'Skin'), (8, 'Water'), (9, 'Wood'), (10, 'Heavy Stone'), (11, 'Heavy Metal'), (12, 'Heavy Wood'), (13, 'Chain'), (14, 'Snow'), (15, 'Stone Stairs'), (16, 'Cloth Stairs'), (17, 'Dirt Stairs'), (18, 'Glass Stairs'), (19, 'Grass Stairs'), (20, 'Metal Stairs'), (21, 'Organic Stairs'), (22, 'Skin Stairs'), (23, 'Water Stairs'), (24, 'Wood Stairs'), (25, 'Heavy Stone Stairs'), (26, 'Heavy Metal Stairs'), (27, 'Heavy Wood Stairs'), (28, 'Chain Stairs'), (29, 'Snow Stairs'), (30, 'Elevator')),
        byte_width=1,
        default_value=0,
    )
    enums['LVLC.LVLF.flags'] = EnumDef(
        name='LVLC.LVLF.flags',
        values=((1, 'calculate_from_all_levels_player_s_level'), (2, 'calculate_for_each_item_in_count')),
        labels=((1, 'Calculate from all levels <= player\'s level'), (2, 'Calculate for each item in count')),
        storage_kind='flags',
        byte_width=1,
    )
    enums['LVLI.LVLF.flags'] = EnumDef(
        name='LVLI.LVLF.flags',
        values=((1, 'calculate_from_all_levels_player_s_level'), (2, 'calculate_for_each_item_in_count')),
        labels=((1, 'Calculate from all levels <= player\'s level'), (2, 'Calculate for each item in count')),
        storage_kind='flags',
        byte_width=1,
    )
    enums['LVSP.LVLF.flags'] = EnumDef(
        name='LVSP.LVLF.flags',
        values=((1, 'calculate_from_all_levels_player_s_level'), (2, 'calculate_for_each_item_in_count'), (4, 'use_all_spells')),
        labels=((1, 'Calculate from all levels <= player\'s level'), (2, 'Calculate for each item in count'), (4, 'Use all spells')),
        storage_kind='flags',
        byte_width=1,
    )
    enums['MGEF.DATA.flags'] = EnumDef(
        name='MGEF.DATA.flags',
        values=((1, 'hostile'), (2, 'recover'), (4, 'detrimental'), (8, 'magnitude'), (16, 'self'), (32, 'touch'), (64, 'target'), (128, 'no_duration'), (256, 'no_magnitude'), (512, 'no_area'), (1024, 'fx_persist'), (2048, 'spellmaking'), (4096, 'enchanting'), (8192, 'no_ingredient'), (65536, 'use_weapon'), (131072, 'use_armor'), (262144, 'use_creature'), (524288, 'use_skill'), (1048576, 'use_attribute'), (16777216, 'use_actor_value'), (33554432, 'spray_projectile_type_or_fog_if_bolt_is_specified_as_well'), (67108864, 'bolt_projectile_type'), (134217728, 'no_hit_effect')),
        labels=((1, 'Hostile'), (2, 'Recover'), (4, 'Detrimental'), (8, 'Magnitude %'), (16, 'Self'), (32, 'Touch'), (64, 'Target'), (128, 'No duration'), (256, 'No magnitude'), (512, 'No area'), (1024, 'FX persist'), (2048, 'Spellmaking'), (4096, 'Enchanting'), (8192, 'No Ingredient'), (65536, 'Use weapon'), (131072, 'Use armor'), (262144, 'Use creature'), (524288, 'Use skill'), (1048576, 'Use attribute'), (16777216, 'Use actor value'), (33554432, 'Spray projectile type (or Fog if Bolt is specified as well)'), (67108864, 'Bolt projectile type'), (134217728, 'No hit effect')),
        storage_kind='flags',
    )
    enums['MGEF.DATA.resist_value'] = EnumDef(
        name='MGEF.DATA.resist_value',
        values=((61, 'resist_fire'), (62, 'resist_frost'), (63, 'resist_disease'), (64, 'resist_magic'), (65, 'resist_normal_weapons'), (66, 'resist_paralysis'), (67, 'resist_poison'), (68, 'resist_shock')),
        labels=((61, 'Resist Fire'), (62, 'Resist Frost'), (63, 'Resist Disease'), (64, 'Resist Magic'), (65, 'Resist Normal Weapons'), (66, 'Resist Paralysis'), (67, 'Resist Poison'), (68, 'Resist Shock')),
    )
    enums['MISC.DATA.group_group'] = EnumDef(
        name='MISC.DATA.group_group',
        values=((0, 'attribute'), (1065353216, 'stat'), (1073741824, 'skill'), (1077936128, 'ai'), (1082130432, 'social'), (1084227584, 'misc'), (1086324736, 'combat'), (1088421888, 'none')),
        labels=((0, 'Attribute'), (1065353216, 'Stat'), (1073741824, 'Skill'), (1077936128, 'AI'), (1082130432, 'Social'), (1084227584, 'Misc'), (1086324736, 'Combat'), (1088421888, ' [NONE]')),
        default_value=0,
    )
    enums['NPC_.ACBS.flags'] = EnumDef(
        name='NPC_.ACBS.flags',
        values=((1, 'female'), (2, 'essential'), (8, 'respawn'), (16, 'auto_calc_stats'), (128, 'pc_level_offset'), (512, 'no_low_level_processing'), (8192, 'no_rumors'), (16384, 'summonable'), (32768, 'no_persuasion'), (1048576, 'can_corpse_check')),
        labels=((1, 'Female'), (2, 'Essential'), (8, 'Respawn'), (16, 'Auto-calc stats'), (128, 'PC Level Offset'), (512, 'No Low Level Processing'), (8192, 'No Rumors'), (16384, 'Summonable'), (32768, 'No Persuasion'), (1048576, 'Can Corpse Check')),
        storage_kind='flags',
    )
    enums['PACK.PLDT.type'] = EnumDef(
        name='PACK.PLDT.type',
        values=((0, 'near_reference'), (1, 'in_cell'), (2, 'near_current_location'), (3, 'near_editor_location'), (4, 'object_id'), (5, 'object_type')),
        labels=((0, 'Near Reference'), (1, 'In Cell'), (2, 'Near Current Location'), (3, 'Near Editor Location'), (4, 'Object ID'), (5, 'Object Type')),
        default_value=0,
    )
    enums['PACK.PTDT.object_type_object_type'] = EnumDef(
        name='PACK.PTDT.object_type_object_type',
        values=((0, 'none'), (1, 'activators'), (2, 'apparatus'), (3, 'armor'), (4, 'books'), (5, 'clothing'), (6, 'containers'), (7, 'doors'), (8, 'ingredients'), (9, 'lights'), (10, 'miscellaneous'), (11, 'flora'), (12, 'furniture'), (13, 'weapons_all'), (14, 'ammo'), (15, 'npcs'), (16, 'creatures'), (17, 'soul_gems'), (18, 'keys'), (19, 'alchemy'), (20, 'food'), (21, 'all_combat_wearable'), (22, 'all_wearable'), (23, 'weapons_none'), (24, 'weapons_melee'), (25, 'weapons_ranged'), (26, 'spells_any'), (27, 'spells_range_target'), (28, 'spells_range_touch'), (29, 'spells_range_self'), (30, 'spells_school_alteration'), (31, 'spells_school_conjuration'), (32, 'spells_school_destruction'), (33, 'spells_school_illusion'), (34, 'spells_school_mysticism'), (35, 'spells_school_restoration')),
        labels=((0, 'None'), (1, 'Activators'), (2, 'Apparatus'), (3, 'Armor'), (4, 'Books'), (5, 'Clothing'), (6, 'Containers'), (7, 'Doors'), (8, 'Ingredients'), (9, 'Lights'), (10, 'Miscellaneous'), (11, 'Flora'), (12, 'Furniture'), (13, 'Weapons: All'), (14, 'Ammo'), (15, 'NPCs'), (16, 'Creatures'), (17, 'Soul Gems'), (18, 'Keys'), (19, 'Alchemy'), (20, 'Food'), (21, 'All: Combat Wearable'), (22, 'All: Wearable'), (23, 'Weapons: None'), (24, 'Weapons: Melee'), (25, 'Weapons: Ranged'), (26, 'Spells: Any'), (27, 'Spells: Range Target'), (28, 'Spells: Range Touch'), (29, 'Spells: Range Self'), (30, 'Spells: School Alteration'), (31, 'Spells: School Conjuration'), (32, 'Spells: School Destruction'), (33, 'Spells: School Illusion'), (34, 'Spells: School Mysticism'), (35, 'Spells: School Restoration')),
        default_value=0,
    )
    enums['PACK.PTDT.type'] = EnumDef(
        name='PACK.PTDT.type',
        values=((0, 'specific_reference'), (1, 'object_id'), (2, 'object_type')),
        labels=((0, 'Specific Reference'), (1, 'Object ID'), (2, 'Object Type')),
        default_value=0,
    )
    enums['QUST.DATA.flags'] = EnumDef(
        name='QUST.DATA.flags',
        values=((1, 'start_game_enabled'), (4, 'allow_repeated_conversation_topics'), (8, 'allow_repeated_stages')),
        labels=((1, 'Start game enabled'), (4, 'Allow repeated conversation topics'), (8, 'Allow repeated stages')),
        storage_kind='flags',
        byte_width=1,
    )
    enums['QUST.SCHD.type'] = EnumDef(
        name='QUST.SCHD.type',
        values=((256, 'magic_effect'),),
        labels=((256, 'Magic Effect'),),
    )
    enums['QUST.SCHR.type'] = EnumDef(
        name='QUST.SCHR.type',
        values=((256, 'magic_effect'),),
        labels=((256, 'Magic Effect'),),
    )
    enums['REFR.FNAM.map_flags'] = EnumDef(
        name='REFR.FNAM.map_flags',
        values=((1, 'visible'), (2, 'can_travel_to')),
        labels=((1, 'Visible'), (2, 'Can Travel To')),
        storage_kind='flags',
        byte_width=1,
    )
    enums['REFR.TNAM.type'] = EnumDef(
        name='REFR.TNAM.type',
        values=((0, 'none'), (1, 'camp'), (2, 'cave'), (3, 'city'), (4, 'elven_ruin'), (5, 'fort_ruin'), (6, 'mine'), (7, 'landmark'), (8, 'tavern'), (9, 'settlement'), (10, 'daedric_shrine'), (11, 'oblivion_gate'), (12, 'unknown_door_icon')),
        labels=((0, 'None'), (1, 'Camp'), (2, 'Cave'), (3, 'City'), (4, 'Elven Ruin'), (5, 'Fort Ruin'), (6, 'Mine'), (7, 'Landmark'), (8, 'Tavern'), (9, 'Settlement'), (10, 'Daedric Shrine'), (11, 'Oblivion Gate'), (12, 'Unknown? (door icon)')),
        byte_width=1,
        default_value=0,
    )
    enums['REFR.XACT.action_flag'] = EnumDef(
        name='REFR.XACT.action_flag',
        values=((1, 'use_default'), (2, 'activate'), (4, 'open'), (8, 'open_by_default')),
        labels=((1, 'Use Default'), (2, 'Activate'), (4, 'Open'), (8, 'Open by Default')),
        storage_kind='flags',
    )
    enums['REGN.RDAT.type'] = EnumDef(
        name='REGN.RDAT.type',
        values=((2, 'objects'), (3, 'weather'), (4, 'map'), (5, 'land'), (6, 'grass'), (7, 'sound')),
        labels=((2, 'Objects'), (3, 'Weather'), (4, 'Map'), (5, 'Land'), (6, 'Grass'), (7, 'Sound')),
    )
    enums['REGN.RDOT.objects_flags'] = EnumDef(
        name='REGN.RDOT.objects_flags',
        values=((1, 'conform_to_slope'), (2, 'paint_vertices'), (4, 'size_variance'), (8, 'x'), (16, 'y'), (32, 'z'), (64, 'tree'), (128, 'huge_rock')),
        labels=((1, 'Conform to slope'), (2, 'Paint Vertices'), (4, 'Size Variance +/-'), (8, 'X +/-'), (16, 'Y +/-'), (32, 'Z +/-'), (64, 'Tree'), (128, 'Huge Rock')),
        storage_kind='flags',
        byte_width=1,
    )
    enums['SCPT.SCHD.type'] = EnumDef(
        name='SCPT.SCHD.type',
        values=((256, 'magic_effect'),),
        labels=((256, 'Magic Effect'),),
    )
    enums['SCPT.SCHR.type'] = EnumDef(
        name='SCPT.SCHR.type',
        values=((256, 'magic_effect'),),
        labels=((256, 'Magic Effect'),),
    )
    enums['SPEL.SPIT.flags'] = EnumDef(
        name='SPEL.SPIT.flags',
        values=((1, 'manual_spell_cost'), (2, 'immune_to_silence_1'), (4, 'player_start_spell'), (8, 'immune_to_silence_2'), (16, 'area_effect_ignores_los'), (32, 'script_effect_always_applies'), (64, 'disallow_spell_absorb_reflect'), (128, 'touch_spell_explodes_w_no_target')),
        labels=((1, 'Manual Spell Cost'), (2, 'Immune to Silence 1'), (4, 'Player Start Spell'), (8, 'Immune to Silence 2'), (16, 'Area Effect Ignores LOS'), (32, 'Script Effect Always Applies'), (64, 'Disallow Spell Absorb/Reflect'), (128, 'Touch Spell Explodes w/ no Target')),
        storage_kind='flags',
        byte_width=1,
    )
    enums['SPEL.SPIT.level'] = EnumDef(
        name='SPEL.SPIT.level',
        values=((0, 'novice'), (1, 'apprentice'), (2, 'journeyman'), (3, 'expert'), (4, 'master')),
        labels=((0, 'Novice'), (1, 'Apprentice'), (2, 'Journeyman'), (3, 'Expert'), (4, 'Master')),
        byte_width=1,
        default_value=0,
    )
    enums['SPEL.SPIT.type'] = EnumDef(
        name='SPEL.SPIT.type',
        values=((0, 'spell'), (1, 'disease'), (2, 'power'), (3, 'lesser_power'), (4, 'ability'), (5, 'poison')),
        labels=((0, 'Spell'), (1, 'Disease'), (2, 'Power'), (3, 'Lesser Power'), (4, 'Ability'), (5, 'Poison')),
        byte_width=1,
        default_value=0,
    )
    enums['WATR.FNAM.flags'] = EnumDef(
        name='WATR.FNAM.flags',
        values=((1, 'causes_damage'), (2, 'reflective')),
        labels=((1, 'Causes Damage'), (2, 'Reflective')),
        storage_kind='flags',
        byte_width=1,
    )
    enums['WEAP.DATA.type'] = EnumDef(
        name='WEAP.DATA.type',
        values=((0, 'blade_one_hand'), (1, 'blade_two_hand'), (2, 'blunt_one_hand'), (3, 'blunt_two_hand'), (4, 'staff'), (5, 'bow')),
        labels=((0, 'Blade One Hand'), (1, 'Blade Two Hand'), (2, 'Blunt One Hand'), (3, 'Blunt Two Hand'), (4, 'Staff'), (5, 'Bow')),
        byte_width=1,
        default_value=0,
    )
    enums['WRLD.DATA.flags'] = EnumDef(
        name='WRLD.DATA.flags',
        values=((1, 'small_world'), (2, 'can_t_fast_travel'), (4, 'oblivion_worldspace'), (16, 'no_lod_water')),
        labels=((1, 'Small world'), (2, 'Can\'t fast travel'), (4, 'Oblivion worldspace'), (16, 'No LOD water')),
        storage_kind='flags',
        byte_width=1,
    )
    enums['WTHR.DATA.flags'] = EnumDef(
        name='WTHR.DATA.flags',
        values=((1, 'weather_pleasant'), (2, 'weather_cloudy'), (4, 'weather_rainy'), (8, 'weather_snow')),
        labels=((1, 'Weather - Pleasant'), (2, 'Weather - Cloudy'), (4, 'Weather - Rainy'), (8, 'Weather - Snow')),
        storage_kind='flags',
        byte_width=1,
    )
    enums['WTHR.SNAM.type'] = EnumDef(
        name='WTHR.SNAM.type',
        values=((0, 'default'), (1, 'precipitation'), (2, 'wind'), (3, 'thunder')),
        labels=((0, 'Default'), (1, 'Precipitation'), (2, 'Wind'), (3, 'Thunder')),
        default_value=0,
    )
    enums['actor_value_enum'] = EnumDef(
        name='actor_value_enum',
        values=((0, 'strength'), (1, 'intelligence'), (2, 'willpower'), (3, 'agility'), (4, 'speed'), (5, 'endurance'), (6, 'personality'), (7, 'luck'), (8, 'health'), (9, 'magicka'), (10, 'fatigue'), (11, 'encumbrance'), (12, 'armorer'), (13, 'athletics'), (14, 'blade'), (15, 'block'), (16, 'blunt'), (17, 'hand_to_hand'), (18, 'heavy_armor'), (19, 'alchemy'), (20, 'alteration'), (21, 'conjuration'), (22, 'destruction'), (23, 'illusion'), (24, 'mysticism'), (25, 'restoration'), (26, 'acrobatics'), (27, 'light_armor'), (28, 'marksman'), (29, 'mercantile'), (30, 'security'), (31, 'sneak'), (32, 'speechcraft'), (33, 'aggression'), (34, 'confidence'), (35, 'energy'), (36, 'responsibility'), (37, 'bounty'), (38, 'fame'), (39, 'infamy'), (40, 'magicka_multiplier'), (41, 'night_eye_bonus'), (42, 'attack_bonus'), (43, 'defend_bonus'), (44, 'casting_penalty'), (45, 'blindness'), (46, 'chameleon'), (47, 'invisibility'), (48, 'paralysis'), (49, 'silence'), (50, 'confusion'), (51, 'detect_item_range'), (52, 'spell_absorb_chance'), (53, 'spell_reflect_chance'), (54, 'swim_speed_multiplier'), (55, 'water_breathing'), (56, 'water_walking'), (57, 'stunted_magicka'), (58, 'detect_life_range'), (59, 'reflect_damage'), (60, 'telekinesis'), (61, 'resist_fire'), (62, 'resist_frost'), (63, 'resist_disease'), (64, 'resist_magic'), (65, 'resist_normal_weapons'), (66, 'resist_paralysis'), (67, 'resist_poison'), (68, 'resist_shock'), (69, 'vampirism'), (70, 'darkness'), (71, 'resist_water_damage')),
        labels=((0, 'Strength'), (1, 'Intelligence'), (2, 'Willpower'), (3, 'Agility'), (4, 'Speed'), (5, 'Endurance'), (6, 'Personality'), (7, 'Luck'), (8, 'Health'), (9, 'Magicka'), (10, 'Fatigue'), (11, 'Encumbrance'), (12, 'Armorer'), (13, 'Athletics'), (14, 'Blade'), (15, 'Block'), (16, 'Blunt'), (17, 'Hand To Hand'), (18, 'Heavy Armor'), (19, 'Alchemy'), (20, 'Alteration'), (21, 'Conjuration'), (22, 'Destruction'), (23, 'Illusion'), (24, 'Mysticism'), (25, 'Restoration'), (26, 'Acrobatics'), (27, 'Light Armor'), (28, 'Marksman'), (29, 'Mercantile'), (30, 'Security'), (31, 'Sneak'), (32, 'Speechcraft'), (33, 'Aggression'), (34, 'Confidence'), (35, 'Energy'), (36, 'Responsibility'), (37, 'Bounty'), (38, 'Fame'), (39, 'Infamy'), (40, 'Magicka Multiplier'), (41, 'Night Eye Bonus'), (42, 'Attack Bonus'), (43, 'Defend Bonus'), (44, 'Casting Penalty'), (45, 'Blindness'), (46, 'Chameleon'), (47, 'Invisibility'), (48, 'Paralysis'), (49, 'Silence'), (50, 'Confusion'), (51, 'Detect Item Range'), (52, 'Spell Absorb Chance'), (53, 'Spell Reflect Chance'), (54, 'Swim Speed Multiplier'), (55, 'Water Breathing'), (56, 'Water Walking'), (57, 'Stunted Magicka'), (58, 'Detect Life Range'), (59, 'Reflect Damage'), (60, 'Telekinesis'), (61, 'Resist Fire'), (62, 'Resist Frost'), (63, 'Resist Disease'), (64, 'Resist Magic'), (65, 'Resist Normal Weapons'), (66, 'Resist Paralysis'), (67, 'Resist Poison'), (68, 'Resist Shock'), (69, 'Vampirism'), (70, 'Darkness'), (71, 'Resist Water Damage')),
        default_value=0,
    )
    enums['attribute_enum'] = EnumDef(
        name='attribute_enum',
        values=((0, 'strength'), (1, 'intelligence'), (2, 'willpower'), (3, 'agility'), (4, 'speed'), (5, 'endurance'), (6, 'personality'), (7, 'luck')),
        labels=((0, 'Strength'), (1, 'Intelligence'), (2, 'Willpower'), (3, 'Agility'), (4, 'Speed'), (5, 'Endurance'), (6, 'Personality'), (7, 'Luck')),
        default_value=0,
    )
    enums['biped_flags'] = EnumDef(
        name='biped_flags',
        values=((1, 'head'), (2, 'hair'), (4, 'upper_body'), (8, 'lower_body'), (16, 'hand'), (32, 'foot'), (64, 'right_ring'), (128, 'left_ring'), (256, 'amulet'), (512, 'weapon'), (1024, 'back_weapon'), (2048, 'side_weapon'), (4096, 'quiver'), (8192, 'shield'), (16384, 'torch'), (32768, 'tail')),
        labels=((1, 'Head'), (2, 'Hair'), (4, 'Upper Body'), (8, 'Lower Body'), (16, 'Hand'), (32, 'Foot'), (64, 'Right Ring'), (128, 'Left Ring'), (256, 'Amulet'), (512, 'Weapon'), (1024, 'Back Weapon'), (2048, 'Side Weapon'), (4096, 'Quiver'), (8192, 'Shield'), (16384, 'Torch'), (32768, 'Tail')),
        storage_kind='flags',
        byte_width=2,
    )
    enums['blend_mode_enum'] = EnumDef(
        name='blend_mode_enum',
        values=((0, 'value'), (1, 'zero'), (2, 'one'), (3, 'source_color'), (4, 'source_inverse_color'), (5, 'source_alpha'), (6, 'source_inverted_alpha'), (7, 'dest_alpha'), (8, 'dest_inverted_alpha'), (9, 'dest_color'), (10, 'dest_inverse_color'), (11, 'source_alpha_sat')),
        labels=((0, ''), (1, 'Zero'), (2, 'One'), (3, 'Source Color'), (4, 'Source Inverse Color'), (5, 'Source Alpha'), (6, 'Source Inverted Alpha'), (7, 'Dest Alpha'), (8, 'Dest Inverted Alpha'), (9, 'Dest Color'), (10, 'Dest Inverse Color'), (11, 'Source Alpha SAT')),
        default_value=0,
    )
    enums['blend_op_enum'] = EnumDef(
        name='blend_op_enum',
        values=((0, 'value'), (1, 'add'), (2, 'subtract'), (3, 'reverse_subtract'), (4, 'minimum'), (5, 'maximum')),
        labels=((0, ''), (1, 'Add'), (2, 'Subtract'), (3, 'Reverse Subtract'), (4, 'Minimum'), (5, 'Maximum')),
        default_value=0,
    )
    enums['body_part_index_enum'] = EnumDef(
        name='body_part_index_enum',
        values=((0, 'upper_body'),),
        labels=((0, 'Upper Body'),),
        default_value=0,
    )
    enums['bool_enum'] = EnumDef(
        name='bool_enum',
        values=((0, 'false'), (1, 'true')),
        labels=((0, 'False'), (1, 'True')),
        byte_width=1,
        default_value=0,
    )
    enums['dialogue_type_enum'] = EnumDef(
        name='dialogue_type_enum',
        values=((0, 'topic'), (1, 'conversation'), (2, 'combat'), (3, 'persuasion'), (4, 'detection'), (5, 'service'), (6, 'miscellaneous')),
        labels=((0, 'Topic'), (1, 'Conversation'), (2, 'Combat'), (3, 'Persuasion'), (4, 'Detection'), (5, 'Service'), (6, 'Miscellaneous')),
        byte_width=1,
        default_value=0,
    )
    enums['effect_type_enum'] = EnumDef(
        name='effect_type_enum',
        values=((0, 'self'), (1, 'touch'), (2, 'target')),
        labels=((0, 'Self'), (1, 'Touch'), (2, 'Target')),
        default_value=0,
    )
    enums['magic_school_enum'] = EnumDef(
        name='magic_school_enum',
        values=((0, 'alteration'), (1, 'conjuration'), (2, 'destruction'), (3, 'illusion'), (4, 'mysticism'), (5, 'restoration'), (6, 'none')),
        labels=((0, 'Alteration'), (1, 'Conjuration'), (2, 'Destruction'), (3, 'Illusion'), (4, 'Mysticism'), (5, 'Restoration'), (6, 'None')),
        default_value=0,
    )
    enums['major_skill_enum'] = EnumDef(
        name='major_skill_enum',
        values=((12, 'armorer'), (13, 'athletics'), (14, 'blade'), (15, 'block'), (16, 'blunt'), (17, 'hand_to_hand'), (18, 'heavy_armor'), (19, 'alchemy'), (20, 'alteration'), (21, 'conjuration'), (22, 'destruction'), (23, 'illusion'), (24, 'mysticism'), (25, 'restoration'), (26, 'acrobatics'), (27, 'light_armor'), (28, 'marksman'), (29, 'mercantile'), (30, 'security'), (31, 'sneak'), (32, 'speechcraft')),
        labels=((12, 'Armorer'), (13, 'Athletics'), (14, 'Blade'), (15, 'Block'), (16, 'Blunt'), (17, 'Hand To Hand'), (18, 'Heavy Armor'), (19, 'Alchemy'), (20, 'Alteration'), (21, 'Conjuration'), (22, 'Destruction'), (23, 'Illusion'), (24, 'Mysticism'), (25, 'Restoration'), (26, 'Acrobatics'), (27, 'Light Armor'), (28, 'Marksman'), (29, 'Mercantile'), (30, 'Security'), (31, 'Sneak'), (32, 'Speechcraft')),
    )
    enums['music_enum'] = EnumDef(
        name='music_enum',
        values=((0, 'default'), (1, 'public'), (2, 'dungeon')),
        labels=((0, 'Default'), (1, 'Public'), (2, 'Dungeon')),
        byte_width=1,
        default_value=0,
    )
    enums['package_flags'] = EnumDef(
        name='package_flags',
        values=((1, 'offers_services'), (4, 'must_complete'), (64, 'unlock_doors_at_package_start'), (128, 'unlock_doors_at_package_end'), (512, 'continue_if_pc_near'), (1024, 'once_per_day'), (131072, 'always_sneak'), (262144, 'allow_swimming'), (2097152, 'weapons_unequipped')),
        labels=((1, 'Offers Services'), (4, 'Must Complete'), (64, 'Unlock Doors At Package Start'), (128, 'Unlock Doors At Package End'), (512, 'Continue If PC Near'), (1024, 'Once Per Day'), (131072, 'Always Sneak'), (262144, 'Allow Swimming'), (2097152, 'Weapons Unequipped')),
        storage_kind='flags',
        byte_width=2,
    )
    enums['package_schedule_day_of_month_enum'] = EnumDef(
        name='package_schedule_day_of_month_enum',
        values=((0, 'any'), (1, '1'), (2, '2'), (3, '3'), (4, '4'), (5, '5'), (6, '6'), (7, '7'), (8, '8'), (9, '9'), (10, '10'), (11, '11'), (12, '12'), (13, '13'), (14, '14'), (15, '15'), (16, '16'), (17, '17'), (18, '18'), (19, '19'), (20, '20'), (21, '21'), (22, '22'), (23, '23'), (24, '24'), (25, '25'), (26, '26'), (27, '27'), (28, '28'), (29, '29'), (30, '30'), (31, '31')),
        labels=((0, 'Any'), (1, '1'), (2, '2'), (3, '3'), (4, '4'), (5, '5'), (6, '6'), (7, '7'), (8, '8'), (9, '9'), (10, '10'), (11, '11'), (12, '12'), (13, '13'), (14, '14'), (15, '15'), (16, '16'), (17, '17'), (18, '18'), (19, '19'), (20, '20'), (21, '21'), (22, '22'), (23, '23'), (24, '24'), (25, '25'), (26, '26'), (27, '27'), (28, '28'), (29, '29'), (30, '30'), (31, '31')),
        byte_width=1,
        default_value=0,
    )
    enums['package_schedule_day_of_week_enum'] = EnumDef(
        name='package_schedule_day_of_week_enum',
        values=((0, 'sunday'), (1, 'monday'), (2, 'tuesday'), (3, 'wednesday'), (4, 'thursday'), (5, 'friday'), (6, 'saturday'), (7, 'weekdays_mtwtf'), (8, 'weekends_ss'), (9, 'monday_wednesday_friday'), (10, 'tuesday_thursday')),
        labels=((0, 'Sunday'), (1, 'Monday'), (2, 'Tuesday'), (3, 'Wednesday'), (4, 'Thursday'), (5, 'Friday'), (6, 'Saturday'), (7, 'Weekdays (MTWTF)'), (8, 'Weekends (SS)'), (9, 'Monday, Wednesday, Friday'), (10, 'Tuesday, Thursday')),
        byte_width=1,
        default_value=0,
    )
    enums['package_schedule_hours_enum'] = EnumDef(
        name='package_schedule_hours_enum',
        values=((0, '0'), (1, '1'), (2, '2'), (3, '3'), (4, '4'), (5, '5'), (6, '6'), (7, '7'), (8, '8'), (9, '9'), (10, '10'), (11, '11'), (12, '12'), (13, '13'), (14, '14'), (15, '15'), (16, '16'), (17, '17'), (18, '18'), (19, '19'), (20, '20'), (21, '21'), (22, '22'), (23, '23')),
        labels=((0, '0'), (1, '1'), (2, '2'), (3, '3'), (4, '4'), (5, '5'), (6, '6'), (7, '7'), (8, '8'), (9, '9'), (10, '10'), (11, '11'), (12, '12'), (13, '13'), (14, '14'), (15, '15'), (16, '16'), (17, '17'), (18, '18'), (19, '19'), (20, '20'), (21, '21'), (22, '22'), (23, '23')),
        byte_width=1,
        default_value=0,
    )
    enums['package_type_enum'] = EnumDef(
        name='package_type_enum',
        values=((0, 'find'), (1, 'follow'), (2, 'escort'), (3, 'eat'), (4, 'sleep'), (5, 'wander'), (6, 'travel'), (7, 'accompany'), (8, 'use_item_at'), (9, 'ambush'), (10, 'flee_not_combat')),
        labels=((0, 'Find'), (1, 'Follow'), (2, 'Escort'), (3, 'Eat'), (4, 'Sleep'), (5, 'Wander'), (6, 'Travel'), (7, 'Accompany'), (8, 'Use Item At'), (9, 'Ambush'), (10, 'Flee Not Combat')),
        byte_width=1,
        default_value=0,
    )
    enums['pgag_flags'] = EnumDef(
        name='pgag_flags',
        values=((1, 'point_1'), (2, 'point_2'), (4, 'point_3'), (8, 'point_4'), (16, 'point_5'), (32, 'point_6'), (64, 'point_7'), (128, 'point_8')),
        labels=((1, 'Point 1'), (2, 'Point 2'), (4, 'Point 3'), (8, 'Point 4'), (16, 'Point 5'), (32, 'Point 6'), (64, 'Point 7'), (128, 'Point 8')),
        storage_kind='flags',
        byte_width=1,
    )
    enums['quadrant_enum'] = EnumDef(
        name='quadrant_enum',
        values=((0, 'bottom_left'), (1, 'bottom_right'), (2, 'top_left'), (3, 'top_right')),
        labels=((0, 'Bottom Left'), (1, 'Bottom Right'), (2, 'Top Left'), (3, 'Top Right')),
        byte_width=1,
        default_value=0,
    )
    enums['service_flags'] = EnumDef(
        name='service_flags',
        values=((1, 'weapons'), (2, 'armor'), (4, 'clothing'), (8, 'books'), (16, 'ingredients'), (32, 'value'), (64, 'value'), (128, 'lights'), (256, 'apparatus'), (512, 'value'), (1024, 'miscellaneous'), (2048, 'spells'), (4096, 'magic_items'), (8192, 'potions'), (16384, 'training'), (32768, 'value'), (65536, 'recharge'), (131072, 'repair')),
        labels=((1, 'Weapons'), (2, 'Armor'), (4, 'Clothing'), (8, 'Books'), (16, 'Ingredients'), (32, ''), (64, ''), (128, 'Lights'), (256, 'Apparatus'), (512, ''), (1024, 'Miscellaneous'), (2048, 'Spells'), (4096, 'Magic Items'), (8192, 'Potions'), (16384, 'Training'), (32768, ''), (65536, 'Recharge'), (131072, 'Repair')),
        storage_kind='flags',
    )
    enums['skill_enum'] = EnumDef(
        name='skill_enum',
        values=((0, 'armorer'), (1, 'athletics'), (2, 'blade'), (3, 'block'), (4, 'blunt'), (5, 'hand_to_hand'), (6, 'heavy_armor'), (7, 'alchemy'), (8, 'alteration'), (9, 'conjuration'), (10, 'destruction'), (11, 'illusion'), (12, 'mysticism'), (13, 'restoration'), (14, 'acrobatics'), (15, 'light_armor'), (16, 'marksman'), (17, 'mercantile'), (18, 'security'), (19, 'sneak'), (20, 'speechcraft')),
        labels=((0, 'Armorer'), (1, 'Athletics'), (2, 'Blade'), (3, 'Block'), (4, 'Blunt'), (5, 'Hand To Hand'), (6, 'Heavy Armor'), (7, 'Alchemy'), (8, 'Alteration'), (9, 'Conjuration'), (10, 'Destruction'), (11, 'Illusion'), (12, 'Mysticism'), (13, 'Restoration'), (14, 'Acrobatics'), (15, 'Light Armor'), (16, 'Marksman'), (17, 'Mercantile'), (18, 'Security'), (19, 'Sneak'), (20, 'Speechcraft')),
        byte_width=1,
        default_value=0,
    )
    enums['soul_gem_enum'] = EnumDef(
        name='soul_gem_enum',
        values=((0, 'none'), (1, 'petty'), (2, 'lesser'), (3, 'common'), (4, 'greater'), (5, 'grand')),
        labels=((0, 'None'), (1, 'Petty'), (2, 'Lesser'), (3, 'Common'), (4, 'Greater'), (5, 'Grand')),
        byte_width=1,
        default_value=0,
    )
    enums['specialization_enum'] = EnumDef(
        name='specialization_enum',
        values=((0, 'combat'), (1, 'magic'), (2, 'stealth')),
        labels=((0, 'Combat'), (1, 'Magic'), (2, 'Stealth')),
        default_value=0,
    )
    enums['z_test_func_enum'] = EnumDef(
        name='z_test_func_enum',
        values=((3, 'equal_to'), (5, 'greater_than'), (7, 'greater_than_or_equal_to'), (8, 'always_show')),
        labels=((3, 'Equal To'), (5, 'Greater Than'), (7, 'Greater Than or Equal To'), (8, 'Always Show')),
    )

    records['ACHR'] = RecordSpec(
        sig='ACHR',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='NAME',
                kind=FieldKind.PARSED,
                display_label='Base',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='base',
                        kind='formid',
                        formlink_target='NPC_',
                        formlink_targets=('NPC_',),
                        authoring_label='Base',
                    ),
                ),
                required=True,
                formlink_target='NPC_',
                formlink_targets=('NPC_',),
            ),
            SubrecordSpec(
                sig='XPCI',
                kind=FieldKind.PARSED,
                display_label='Unused',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='unused',
                        kind='formid',
                        formlink_target='CELL',
                        formlink_targets=('CELL',),
                        authoring_label='Unused',
                    ),
                ),
                repeatable=True,
                formlink_target='CELL',
                formlink_targets=('CELL',),
                authoring_layout='row_group',
                authoring_key='group_unused',
                scope_id='unused',
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Unused',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='unused',
                        kind='zstring',
                        authoring_label='Unused',
                    ),
                ),
                repeatable=True,
                authoring_layout='row_group',
                authoring_key='group_unused',
                scope_id='unused',
            ),
            SubrecordSpec(
                sig='XLOD',
                kind=FieldKind.PARSED,
                display_label='Distant LOD Data',
                codec='array_struct:f',
                fields=(
                    FieldSpec(
                        name='distant_lod_data_unknown',
                        kind='float32',
                        authoring_label='Distant LOD Data Unknown',
                    ),
                ),
                array=ArraySpec(layout='row_array', element_codec='f'),
                row_label='Distant LOD Data',
            ),
            SubrecordSpec(
                sig='XESP',
                kind=FieldKind.PARSED,
                display_label='Enable Parent',
                codec='struct:I,B,B,B,B',
                fields=(
                    FieldSpec(
                        name='reference',
                        kind='formid',
                        formlink_targets=('ACHR', 'ACRE', 'PLYR', 'REFR'),
                        authoring_label='Reference',
                    ),
                    FieldSpec(
                        name='set_enable_state_to_opposite_of_parent',
                        kind='uint8',
                        enum_ref='bool_enum',
                        authoring_label='Set Enable State To Opposite Of Parent',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Unknown Byte 3',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Unknown Byte 4',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_4',
                        kind='uint8',
                        authoring_label='Unknown Byte 5',
                        notes='wbUnused(3)',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='XMRC',
                kind=FieldKind.PARSED,
                display_label='Merchant container',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='merchant_container',
                        kind='formid',
                        formlink_target='REFR',
                        formlink_targets=('REFR',),
                        authoring_label='Merchant container',
                    ),
                ),
                formlink_target='REFR',
                formlink_targets=('REFR',),
            ),
            SubrecordSpec(
                sig='XHRS',
                kind=FieldKind.PARSED,
                display_label='Horse',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='horse',
                        kind='formid',
                        formlink_target='ACRE',
                        formlink_targets=('ACRE',),
                        authoring_label='Horse',
                    ),
                ),
                formlink_target='ACRE',
                formlink_targets=('ACRE',),
            ),
            SubrecordSpec(
                sig='XRGD',
                kind=FieldKind.PARSED,
                display_label='Bones',
                codec='array_struct:B,B,B,B,f,f,f,f,f,f',
                fields=(
                    FieldSpec(
                        name='bones_bone_id',
                        kind='uint8',
                        authoring_label='Bones Bone Id',
                    ),
                    FieldSpec(
                        name='unknown_u8_1',
                        kind='uint8',
                        authoring_label='Bones Unknown Byte 2',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Bones Unknown Byte 3',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Bones Unknown Byte 4',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='bones_position_x',
                        kind='float32',
                        authoring_label='Bones Position X',
                    ),
                    FieldSpec(
                        name='bones_position_y',
                        kind='float32',
                        authoring_label='Bones Position Y',
                    ),
                    FieldSpec(
                        name='bones_position_z',
                        kind='float32',
                        authoring_label='Bones Position Z',
                    ),
                    FieldSpec(
                        name='bones_rotation_x',
                        kind='float32',
                        authoring_label='Bones Rotation X',
                    ),
                    FieldSpec(
                        name='bones_rotation_y',
                        kind='float32',
                        authoring_label='Bones Rotation Y',
                    ),
                    FieldSpec(
                        name='bones_rotation_z',
                        kind='float32',
                        authoring_label='Bones Rotation Z',
                    ),
                ),
                repeatable=True,
                array=ArraySpec(layout='row_array', element_codec='B,B,B,B,f,f,f,f,f,f'),
                row_label='Bones',
                scope_id='ragdoll_data',
            ),
            SubrecordSpec(
                sig='XSCL',
                kind=FieldKind.PARSED,
                display_label='Scale',
                codec='float32',
                fields=(
                    FieldSpec(name='float32_0', kind='float32'),
                ),
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                codec='struct:f,f,f,f,f,f',
                fields=(
                    FieldSpec(
                        name='position_rotation_position_x',
                        kind='float32',
                        authoring_label='Position Rotation Position X',
                    ),
                    FieldSpec(
                        name='position_rotation_position_y',
                        kind='float32',
                        authoring_label='Position Rotation Position Y',
                    ),
                    FieldSpec(
                        name='position_rotation_position_z',
                        kind='float32',
                        authoring_label='Position Rotation Position Z',
                    ),
                    FieldSpec(
                        name='position_rotation_rotation_x',
                        kind='float32',
                        authoring_label='Position Rotation Rotation X',
                    ),
                    FieldSpec(
                        name='position_rotation_rotation_y',
                        kind='float32',
                        authoring_label='Position Rotation Rotation Y',
                    ),
                    FieldSpec(
                        name='position_rotation_rotation_z',
                        kind='float32',
                        authoring_label='Position Rotation Rotation Z',
                    ),
                ),
                required=True,
            ),
        ),
        display_label='Placed NPC',
        record_flags=RecordFlagsSpec(valid_mask=39968, bits=(RecordFlagBit(bit=10, name='Persistent'), RecordFlagBit(bit=11, name='Initially Disabled'), RecordFlagBit(bit=15, name='Visible When Distant'))),
    )

    records['ACRE'] = RecordSpec(
        sig='ACRE',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='NAME',
                kind=FieldKind.PARSED,
                display_label='Base',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='base',
                        kind='formid',
                        formlink_target='CREA',
                        formlink_targets=('CREA',),
                        authoring_label='Base',
                    ),
                ),
                required=True,
                formlink_target='CREA',
                formlink_targets=('CREA',),
            ),
            SubrecordSpec(
                sig='XOWN',
                kind=FieldKind.PARSED,
                display_label='Owner',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='owner',
                        kind='formid',
                        formlink_targets=('FACT', 'NPC_'),
                        authoring_label='Owner',
                    ),
                ),
                repeatable=True,
                formlink_targets=('FACT', 'NPC_'),
                scope_id='ownership',
            ),
            SubrecordSpec(
                sig='XRNK',
                kind=FieldKind.PARSED,
                display_label='Faction rank',
                codec='int32',
                fields=(
                    FieldSpec(
                        name='faction_rank',
                        kind='int32',
                        authoring_label='Faction rank',
                    ),
                ),
                repeatable=True,
                scope_id='ownership',
            ),
            SubrecordSpec(
                sig='XGLB',
                kind=FieldKind.PARSED,
                display_label='Global',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='global',
                        kind='formid',
                        formlink_target='GLOB',
                        formlink_targets=('GLOB',),
                        authoring_label='Global',
                    ),
                ),
                repeatable=True,
                formlink_target='GLOB',
                formlink_targets=('GLOB',),
                scope_id='ownership',
            ),
            SubrecordSpec(
                sig='XRGD',
                kind=FieldKind.PARSED,
                display_label='Bones',
                codec='array_struct:B,B,B,B,f,f,f,f,f,f',
                fields=(
                    FieldSpec(
                        name='bones_bone_id',
                        kind='uint8',
                        authoring_label='Bones Bone Id',
                    ),
                    FieldSpec(
                        name='unknown_u8_1',
                        kind='uint8',
                        authoring_label='Bones Unknown Byte 2',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Bones Unknown Byte 3',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Bones Unknown Byte 4',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='bones_position_x',
                        kind='float32',
                        authoring_label='Bones Position X',
                    ),
                    FieldSpec(
                        name='bones_position_y',
                        kind='float32',
                        authoring_label='Bones Position Y',
                    ),
                    FieldSpec(
                        name='bones_position_z',
                        kind='float32',
                        authoring_label='Bones Position Z',
                    ),
                    FieldSpec(
                        name='bones_rotation_x',
                        kind='float32',
                        authoring_label='Bones Rotation X',
                    ),
                    FieldSpec(
                        name='bones_rotation_y',
                        kind='float32',
                        authoring_label='Bones Rotation Y',
                    ),
                    FieldSpec(
                        name='bones_rotation_z',
                        kind='float32',
                        authoring_label='Bones Rotation Z',
                    ),
                ),
                repeatable=True,
                array=ArraySpec(layout='row_array', element_codec='B,B,B,B,f,f,f,f,f,f'),
                row_label='Bones',
                scope_id='ragdoll_data',
            ),
            SubrecordSpec(
                sig='XLOD',
                kind=FieldKind.PARSED,
                display_label='Distant LOD Data',
                codec='array_struct:f',
                fields=(
                    FieldSpec(
                        name='distant_lod_data_unknown',
                        kind='float32',
                        authoring_label='Distant LOD Data Unknown',
                    ),
                ),
                array=ArraySpec(layout='row_array', element_codec='f'),
                row_label='Distant LOD Data',
            ),
            SubrecordSpec(
                sig='XESP',
                kind=FieldKind.PARSED,
                display_label='Enable Parent',
                codec='struct:I,B,B,B,B',
                fields=(
                    FieldSpec(
                        name='reference',
                        kind='formid',
                        formlink_targets=('ACHR', 'ACRE', 'PLYR', 'REFR'),
                        authoring_label='Reference',
                    ),
                    FieldSpec(
                        name='set_enable_state_to_opposite_of_parent',
                        kind='uint8',
                        enum_ref='bool_enum',
                        authoring_label='Set Enable State To Opposite Of Parent',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Unknown Byte 3',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Unknown Byte 4',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_4',
                        kind='uint8',
                        authoring_label='Unknown Byte 5',
                        notes='wbUnused(3)',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='XSCL',
                kind=FieldKind.PARSED,
                display_label='Scale',
                codec='float32',
                fields=(
                    FieldSpec(name='float32_0', kind='float32'),
                ),
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                codec='struct:f,f,f,f,f,f',
                fields=(
                    FieldSpec(
                        name='position_rotation_position_x',
                        kind='float32',
                        authoring_label='Position Rotation Position X',
                    ),
                    FieldSpec(
                        name='position_rotation_position_y',
                        kind='float32',
                        authoring_label='Position Rotation Position Y',
                    ),
                    FieldSpec(
                        name='position_rotation_position_z',
                        kind='float32',
                        authoring_label='Position Rotation Position Z',
                    ),
                    FieldSpec(
                        name='position_rotation_rotation_x',
                        kind='float32',
                        authoring_label='Position Rotation Rotation X',
                    ),
                    FieldSpec(
                        name='position_rotation_rotation_y',
                        kind='float32',
                        authoring_label='Position Rotation Rotation Y',
                    ),
                    FieldSpec(
                        name='position_rotation_rotation_z',
                        kind='float32',
                        authoring_label='Position Rotation Rotation Z',
                    ),
                ),
                required=True,
            ),
        ),
        display_label='Placed Creature',
        record_flags=RecordFlagsSpec(valid_mask=39968, bits=(RecordFlagBit(bit=10, name='Persistent'), RecordFlagBit(bit=11, name='Initially Disabled'), RecordFlagBit(bit=15, name='Visible When Distant'))),
    )

    records['ACTI'] = RecordSpec(
        sig='ACTI',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='model_filename',
                        kind='zstring',
                        authoring_label='Model FileName',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='SCRI',
                kind=FieldKind.PARSED,
                display_label='Script',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='script',
                        kind='formid',
                        formlink_target='SCPT',
                        formlink_targets=('SCPT',),
                        authoring_label='Script',
                    ),
                ),
                formlink_target='SCPT',
                formlink_targets=('SCPT',),
            ),
            SubrecordSpec(
                sig='SNAM',
                kind=FieldKind.PARSED,
                display_label='Sound',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='sound',
                        kind='formid',
                        formlink_target='SOUN',
                        formlink_targets=('SOUN',),
                        authoring_label='Sound',
                    ),
                ),
                formlink_target='SOUN',
                formlink_targets=('SOUN',),
            ),
        ),
        display_label='Activator',
        record_flags=RecordFlagsSpec(valid_mask=136224, bits=(RecordFlagBit(bit=10, name='Quest Item'), RecordFlagBit(bit=17, name='Dangerous'))),
    )

    records['ALCH'] = RecordSpec(
        sig='ALCH',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='model_filename',
                        kind='zstring',
                        authoring_label='Model FileName',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='ICON',
                kind=FieldKind.PARSED,
                display_label='Icon FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='icon_filename',
                        kind='zstring',
                        authoring_label='Icon FileName',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='SCRI',
                kind=FieldKind.PARSED,
                display_label='Script',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='script',
                        kind='formid',
                        formlink_target='SCPT',
                        formlink_targets=('SCPT',),
                        authoring_label='Script',
                    ),
                ),
                formlink_target='SCPT',
                formlink_targets=('SCPT',),
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                display_label='Weight',
                codec='float32',
                fields=(
                    FieldSpec(name='float32_0', kind='float32'),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='ENIT',
                kind=FieldKind.PARSED,
                display_label='Data',
                codec='struct:i,B,B,B,B',
                fields=(
                    FieldSpec(
                        name='value',
                        kind='int32',
                        authoring_label='Value',
                    ),
                    FieldSpec(
                        name='flags',
                        kind='uint8',
                        enum_ref='ALCH.ENIT.flags',
                        authoring_label='Flags',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Unknown Byte 3',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Unknown Byte 4',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_4',
                        kind='uint8',
                        authoring_label='Unknown Byte 5',
                        notes='wbUnused(3)',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='EFID',
                kind=FieldKind.PARSED,
                display_label='Magic Effect Name',
                codec='uint32',
                fields=(
                    FieldSpec(
                        name='magic_effect_name',
                        kind='uint32',
                        authoring_label='Magic Effect Name',
                    ),
                ),
                repeatable=True,
                required=True,
                scope_id='effects',
            ),
            SubrecordSpec(
                sig='EFIT',
                kind=FieldKind.PARSED,
                codec='struct:I,I,I,I,I,i',
                fields=(
                    FieldSpec(
                        name='magic_effect_name',
                        kind='uint32',
                        authoring_label='Magic Effect Name',
                    ),
                    FieldSpec(
                        name='magnitude',
                        kind='uint32',
                        authoring_label='Magnitude',
                    ),
                    FieldSpec(
                        name='area',
                        kind='uint32',
                        authoring_label='Area',
                    ),
                    FieldSpec(
                        name='duration',
                        kind='uint32',
                        authoring_label='Duration',
                    ),
                    FieldSpec(
                        name='type',
                        kind='uint32',
                        enum_ref='effect_type_enum',
                        authoring_label='Type',
                    ),
                    FieldSpec(
                        name='actor_value',
                        kind='int32',
                        enum_ref='actor_value_enum',
                        authoring_label='Actor Value',
                    ),
                ),
                repeatable=True,
                required=True,
                scope_id='effects',
            ),
            SubrecordSpec(
                sig='SCIT',
                kind=FieldKind.PARSED_WITH_RAW_FALLBACK,
                codec='struct:I,I,I,B,B,B,B',
                fields=(
                    FieldSpec(
                        name='script_effect',
                        kind='formid',
                        formlink_target='SCPT',
                        formlink_targets=('SCPT',),
                        null_allowed=True,
                        authoring_label='Script effect',
                    ),
                    FieldSpec(
                        name='magic_school',
                        kind='uint32',
                        enum_ref='magic_school_enum',
                        authoring_label='Magic school',
                    ),
                    FieldSpec(
                        name='visual_effect_name',
                        kind='uint32',
                        authoring_label='Visual effect name',
                    ),
                    FieldSpec(
                        name='hostile',
                        kind='uint8',
                        enum_ref='bool_enum',
                        authoring_label='Hostile',
                    ),
                    FieldSpec(
                        name='unknown_u8_4',
                        kind='uint8',
                        authoring_label='Unknown Byte 5',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_5',
                        kind='uint8',
                        authoring_label='Unknown Byte 6',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_6',
                        kind='uint8',
                        authoring_label='Unknown Byte 7',
                        notes='wbUnused(3)',
                    ),
                ),
                repeatable=True,
                required=True,
                scope_id='effects',
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
                repeatable=True,
                required=True,
                scope_id='effects',
            ),
        ),
        display_label='Potion',
        record_flags=RecordFlagsSpec(valid_mask=5152, bits=(RecordFlagBit(bit=10, name='Quest Item'),)),
    )

    records['AMMO'] = RecordSpec(
        sig='AMMO',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='model_filename',
                        kind='zstring',
                        authoring_label='Model FileName',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='ICON',
                kind=FieldKind.PARSED,
                display_label='Icon FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='icon_filename',
                        kind='zstring',
                        authoring_label='Icon FileName',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='EITM',
                kind=FieldKind.PARSED,
                display_label='Effect',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='effect',
                        kind='formid',
                        formlink_target='ENCH',
                        formlink_targets=('ENCH',),
                        authoring_label='Effect',
                    ),
                ),
                repeatable=True,
                formlink_target='ENCH',
                formlink_targets=('ENCH',),
                scope_id='enchantment',
            ),
            SubrecordSpec(
                sig='EAMT',
                kind=FieldKind.PARSED,
                display_label='Capacity',
                codec='uint16',
                fields=(
                    FieldSpec(
                        name='capacity',
                        kind='uint16',
                        authoring_label='Capacity',
                    ),
                ),
                repeatable=True,
                scope_id='enchantment',
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                display_label='Data',
                codec='struct:f,B,B,B,B,I,f,H',
                fields=(
                    FieldSpec(
                        name='speed',
                        kind='float32',
                        authoring_label='Speed',
                    ),
                    FieldSpec(
                        name='ignores_normal_weapon_resistance',
                        kind='uint8',
                        enum_ref='bool_enum',
                        authoring_label='Ignores Normal Weapon Resistance',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Unknown Byte 3',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Unknown Byte 4',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_4',
                        kind='uint8',
                        authoring_label='Unknown Byte 5',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='value',
                        kind='uint32',
                        authoring_label='Value',
                    ),
                    FieldSpec(
                        name='weight',
                        kind='float32',
                        authoring_label='Weight',
                    ),
                    FieldSpec(
                        name='damage',
                        kind='uint16',
                        authoring_label='Damage',
                    ),
                ),
                required=True,
            ),
        ),
        display_label='Ammunition',
        record_flags=RecordFlagsSpec(valid_mask=5152, bits=(RecordFlagBit(bit=10, name='Quest Item'),)),
    )

    records['ANIO'] = RecordSpec(
        sig='ANIO',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='model_filename',
                        kind='zstring',
                        authoring_label='Model FileName',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                display_label='Idle Animation',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='idle_animation',
                        kind='formid',
                        formlink_target='IDLE',
                        formlink_targets=('IDLE',),
                        authoring_label='Idle Animation',
                    ),
                ),
                required=True,
                formlink_target='IDLE',
                formlink_targets=('IDLE',),
            ),
        ),
        display_label='Animated Object',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['APPA'] = RecordSpec(
        sig='APPA',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='model_filename',
                        kind='zstring',
                        authoring_label='Model FileName',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='ICON',
                kind=FieldKind.PARSED,
                display_label='Icon FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='icon_filename',
                        kind='zstring',
                        authoring_label='Icon FileName',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='SCRI',
                kind=FieldKind.PARSED,
                display_label='Script',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='script',
                        kind='formid',
                        formlink_target='SCPT',
                        formlink_targets=('SCPT',),
                        authoring_label='Script',
                    ),
                ),
                formlink_target='SCPT',
                formlink_targets=('SCPT',),
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                display_label='Data',
                codec='struct:B,I,f,f',
                fields=(
                    FieldSpec(
                        name='type',
                        kind='uint8',
                        enum_ref='APPA.DATA.type',
                        authoring_label='Type',
                    ),
                    FieldSpec(
                        name='value',
                        kind='uint32',
                        authoring_label='Value',
                    ),
                    FieldSpec(
                        name='weight',
                        kind='float32',
                        authoring_label='Weight',
                    ),
                    FieldSpec(
                        name='quality',
                        kind='float32',
                        authoring_label='Quality',
                    ),
                ),
                required=True,
            ),
        ),
        display_label='Apparatus',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['ARMO'] = RecordSpec(
        sig='ARMO',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='SCRI',
                kind=FieldKind.PARSED,
                display_label='Script',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='script',
                        kind='formid',
                        formlink_target='SCPT',
                        formlink_targets=('SCPT',),
                        authoring_label='Script',
                    ),
                ),
                formlink_target='SCPT',
                formlink_targets=('SCPT',),
            ),
            SubrecordSpec(
                sig='EITM',
                kind=FieldKind.PARSED,
                display_label='Effect',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='effect',
                        kind='formid',
                        formlink_target='ENCH',
                        formlink_targets=('ENCH',),
                        authoring_label='Effect',
                    ),
                ),
                repeatable=True,
                formlink_target='ENCH',
                formlink_targets=('ENCH',),
                scope_id='enchantment',
            ),
            SubrecordSpec(
                sig='EAMT',
                kind=FieldKind.PARSED,
                display_label='Capacity',
                codec='uint16',
                fields=(
                    FieldSpec(
                        name='capacity',
                        kind='uint16',
                        authoring_label='Capacity',
                    ),
                ),
                repeatable=True,
                scope_id='enchantment',
            ),
            SubrecordSpec(
                sig='BMDT',
                kind=FieldKind.PARSED,
                display_label='Flags',
                codec='struct:H,B,B',
                fields=(
                    FieldSpec(
                        name='biped_flags',
                        kind='uint16',
                        enum_ref='biped_flags',
                        authoring_label='Biped Flags',
                    ),
                    FieldSpec(
                        name='general_flags',
                        kind='uint8',
                        enum_ref='ARMO.BMDT.general_flags',
                        authoring_label='General Flags',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Unknown Byte 3',
                        notes='wbUnused(1)',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='Biped Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='biped_model_filename',
                        kind='zstring',
                        authoring_label='Biped Model FileName',
                    ),
                ),
                repeatable=True,
                scope_id='male',
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
                repeatable=True,
                scope_id='male',
            ),
            SubrecordSpec(
                sig='MOD2',
                kind=FieldKind.PARSED,
                display_label='World Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='world_model_filename',
                        kind='zstring',
                        authoring_label='World Model FileName',
                    ),
                ),
                repeatable=True,
                scope_id='male',
            ),
            SubrecordSpec(
                sig='MO2B',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
                repeatable=True,
                scope_id='male',
            ),
            SubrecordSpec(
                sig='ICON',
                kind=FieldKind.PARSED,
                display_label='Icon Image',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='icon_image',
                        kind='zstring',
                        authoring_label='Icon Image',
                    ),
                ),
                repeatable=True,
                scope_id='male',
            ),
            SubrecordSpec(
                sig='MOD3',
                kind=FieldKind.PARSED,
                display_label='Biped Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='biped_model_filename',
                        kind='zstring',
                        authoring_label='Biped Model FileName',
                    ),
                ),
                repeatable=True,
                scope_id='female',
            ),
            SubrecordSpec(
                sig='MO3B',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
                repeatable=True,
                scope_id='female',
            ),
            SubrecordSpec(
                sig='MOD4',
                kind=FieldKind.PARSED,
                display_label='World Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='world_model_filename',
                        kind='zstring',
                        authoring_label='World Model FileName',
                    ),
                ),
                repeatable=True,
                scope_id='female',
            ),
            SubrecordSpec(
                sig='MO4B',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
                repeatable=True,
                scope_id='female',
            ),
            SubrecordSpec(
                sig='ICO2',
                kind=FieldKind.PARSED,
                display_label='Icon Image',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='icon_image',
                        kind='zstring',
                        authoring_label='Icon Image',
                    ),
                ),
                repeatable=True,
                scope_id='female',
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                display_label='Data',
                codec='struct:H,I,I,f',
                fields=(
                    FieldSpec(
                        name='armor',
                        kind='uint16',
                        authoring_label='Armor',
                    ),
                    FieldSpec(
                        name='value',
                        kind='uint32',
                        authoring_label='Value',
                    ),
                    FieldSpec(
                        name='health',
                        kind='uint32',
                        authoring_label='Health',
                    ),
                    FieldSpec(
                        name='weight',
                        kind='float32',
                        authoring_label='Weight',
                    ),
                ),
                required=True,
            ),
        ),
        display_label='Armor',
        record_flags=RecordFlagsSpec(valid_mask=5152, bits=(RecordFlagBit(bit=10, name='Quest Item'),)),
    )

    records['BOOK'] = RecordSpec(
        sig='BOOK',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='model_filename',
                        kind='zstring',
                        authoring_label='Model FileName',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='ICON',
                kind=FieldKind.PARSED,
                display_label='Icon FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='icon_filename',
                        kind='zstring',
                        authoring_label='Icon FileName',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='SCRI',
                kind=FieldKind.PARSED,
                display_label='Script',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='script',
                        kind='formid',
                        formlink_target='SCPT',
                        formlink_targets=('SCPT',),
                        authoring_label='Script',
                    ),
                ),
                formlink_target='SCPT',
                formlink_targets=('SCPT',),
            ),
            SubrecordSpec(
                sig='EITM',
                kind=FieldKind.PARSED,
                display_label='Effect',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='effect',
                        kind='formid',
                        formlink_target='ENCH',
                        formlink_targets=('ENCH',),
                        authoring_label='Effect',
                    ),
                ),
                repeatable=True,
                formlink_target='ENCH',
                formlink_targets=('ENCH',),
                scope_id='enchantment',
            ),
            SubrecordSpec(
                sig='EAMT',
                kind=FieldKind.PARSED,
                display_label='Capacity',
                codec='uint16',
                fields=(
                    FieldSpec(
                        name='capacity',
                        kind='uint16',
                        authoring_label='Capacity',
                    ),
                ),
                repeatable=True,
                scope_id='enchantment',
            ),
            SubrecordSpec(
                sig='DESC',
                kind=FieldKind.PARSED,
                display_label='Description',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='description',
                        kind='zstring',
                        authoring_label='Description',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                display_label='Data',
                codec='struct:B,b,I,f',
                fields=(
                    FieldSpec(
                        name='flags',
                        kind='uint8',
                        enum_ref='BOOK.DATA.flags',
                        authoring_label='Flags',
                    ),
                    FieldSpec(
                        name='teaches',
                        kind='int8',
                        enum_ref='skill_enum',
                        authoring_label='Teaches',
                    ),
                    FieldSpec(
                        name='value',
                        kind='uint32',
                        authoring_label='Value',
                    ),
                    FieldSpec(
                        name='weight',
                        kind='float32',
                        authoring_label='Weight',
                    ),
                ),
                required=True,
            ),
        ),
        display_label='Book',
        record_flags=RecordFlagsSpec(valid_mask=5152, bits=(RecordFlagBit(bit=10, name='Quest Item'),)),
    )

    records['BSGN'] = RecordSpec(
        sig='BSGN',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='ICON',
                kind=FieldKind.PARSED,
                display_label='Constellation Filename',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='constellation_filename',
                        kind='zstring',
                        authoring_label='Constellation Filename',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='DESC',
                kind=FieldKind.PARSED,
                display_label='Description',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='description',
                        kind='zstring',
                        authoring_label='Description',
                    ),
                ),
            ),
            SubrecordSpec(sig='SPLO', kind=FieldKind.RAW, display_label='Spell', repeatable=True),
        ),
        display_label='Birthsign',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['CELL'] = RecordSpec(
        sig='CELL',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                display_label='Flags',
                codec='uint8',
                fields=(
                    FieldSpec(
                        name='flags',
                        kind='uint8',
                        enum_ref='CELL.DATA.flags',
                        authoring_label='Flags',
                    ),
                ),
                required=True,
                enum_ref='CELL.DATA.flags',
            ),
            SubrecordSpec(
                sig='XCLL',
                kind=FieldKind.PARSED,
                display_label='Lighting',
                codec='struct:B,B,B,B,B,B,B,B,B,B,B,B,f,f,i,i,f,f',
                fields=(
                    FieldSpec(
                        name='ambient_color_red',
                        kind='uint8',
                        authoring_label='Ambient Color Red',
                    ),
                    FieldSpec(
                        name='ambient_color_green',
                        kind='uint8',
                        authoring_label='Ambient Color Green',
                    ),
                    FieldSpec(
                        name='ambient_color_blue',
                        kind='uint8',
                        authoring_label='Ambient Color Blue',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Ambient Color Unknown Byte 4',
                        notes='wbUnused(1)',
                    ),
                    FieldSpec(
                        name='directional_color_red',
                        kind='uint8',
                        authoring_label='Directional Color Red',
                    ),
                    FieldSpec(
                        name='directional_color_green',
                        kind='uint8',
                        authoring_label='Directional Color Green',
                    ),
                    FieldSpec(
                        name='directional_color_blue',
                        kind='uint8',
                        authoring_label='Directional Color Blue',
                    ),
                    FieldSpec(
                        name='unknown_u8_7',
                        kind='uint8',
                        authoring_label='Directional Color Unknown Byte 8',
                        notes='wbUnused(1)',
                    ),
                    FieldSpec(
                        name='fog_color_red',
                        kind='uint8',
                        authoring_label='Fog Color Red',
                    ),
                    FieldSpec(
                        name='fog_color_green',
                        kind='uint8',
                        authoring_label='Fog Color Green',
                    ),
                    FieldSpec(
                        name='fog_color_blue',
                        kind='uint8',
                        authoring_label='Fog Color Blue',
                    ),
                    FieldSpec(
                        name='unknown_u8_11',
                        kind='uint8',
                        authoring_label='Fog Color Unknown Byte 12',
                        notes='wbUnused(1)',
                    ),
                    FieldSpec(
                        name='fog_near',
                        kind='float32',
                        authoring_label='Fog Near',
                    ),
                    FieldSpec(
                        name='fog_far',
                        kind='float32',
                        authoring_label='Fog Far',
                    ),
                    FieldSpec(
                        name='directional_rotation_xy',
                        kind='int32',
                        authoring_label='Directional Rotation XY',
                    ),
                    FieldSpec(
                        name='directional_rotation_z',
                        kind='int32',
                        authoring_label='Directional Rotation Z',
                    ),
                    FieldSpec(
                        name='directional_fade',
                        kind='float32',
                        authoring_label='Directional Fade',
                    ),
                    FieldSpec(
                        name='fog_clip_dist',
                        kind='float32',
                        authoring_label='Fog Clip Dist',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='XCLR',
                kind=FieldKind.PARSED,
                display_label='Regions',
                codec='formid_array',
                fields=(
                    FieldSpec(
                        name='regions_region',
                        kind='formid',
                        formlink_target='REGN',
                        formlink_targets=('REGN',),
                        authoring_label='Regions Region',
                    ),
                ),
                formlink_target='REGN',
                formlink_targets=('REGN',),
            ),
            SubrecordSpec(
                sig='XCMT',
                kind=FieldKind.PARSED,
                display_label='Music',
                codec='uint8',
                fields=(
                    FieldSpec(
                        name='music',
                        kind='uint8',
                        enum_ref='music_enum',
                        authoring_label='Music',
                    ),
                ),
                enum_ref='music_enum',
            ),
            SubrecordSpec(
                sig='XCLW',
                kind=FieldKind.PARSED,
                display_label='Water Height',
                codec='float32',
                fields=(
                    FieldSpec(name='float32_0', kind='float32'),
                ),
            ),
            SubrecordSpec(
                sig='XCCM',
                kind=FieldKind.PARSED,
                display_label='Climate',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='climate',
                        kind='formid',
                        formlink_target='CLMT',
                        formlink_targets=('CLMT',),
                        authoring_label='Climate',
                    ),
                ),
                formlink_target='CLMT',
                formlink_targets=('CLMT',),
            ),
            SubrecordSpec(
                sig='XCWT',
                kind=FieldKind.PARSED,
                display_label='Water',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='water',
                        kind='formid',
                        formlink_target='WATR',
                        formlink_targets=('WATR',),
                        authoring_label='Water',
                    ),
                ),
                formlink_target='WATR',
                formlink_targets=('WATR',),
            ),
            SubrecordSpec(
                sig='XCLC',
                kind=FieldKind.PARSED,
                display_label='Grid',
                codec='struct:i,i',
                fields=(
                    FieldSpec(
                        name='x',
                        kind='int32',
                        authoring_label='X',
                    ),
                    FieldSpec(
                        name='y',
                        kind='int32',
                        authoring_label='Y',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='XOWN',
                kind=FieldKind.PARSED,
                display_label='Owner',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='owner',
                        kind='formid',
                        authoring_label='Owner',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='XRNK',
                kind=FieldKind.PARSED,
                display_label='Faction rank',
                codec='int32',
                fields=(
                    FieldSpec(
                        name='faction_rank',
                        kind='int32',
                        authoring_label='Faction rank',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='XGLB',
                kind=FieldKind.PARSED,
                display_label='Global',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='global',
                        kind='formid',
                        authoring_label='Global',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='XTLI',
                kind=FieldKind.PARSED,
                display_label='Threat Level',
                codec='uint32',
                fields=(
                    FieldSpec(
                        name='threat_level',
                        kind='uint32',
                        authoring_label='Threat Level',
                    ),
                ),
            ),
        ),
        display_label='Cell',
        record_flags=RecordFlagsSpec(valid_mask=660512, bits=(RecordFlagBit(bit=10, name='Persistent'), RecordFlagBit(bit=17, name='Off Limits'), RecordFlagBit(bit=19, name='Can\'t Wait'))),
    )

    records['CLAS'] = RecordSpec(
        sig='CLAS',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='DESC',
                kind=FieldKind.PARSED,
                display_label='Description',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='description',
                        kind='zstring',
                        authoring_label='Description',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='ICON',
                kind=FieldKind.PARSED,
                display_label='Image Filename',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='image_filename',
                        kind='zstring',
                        authoring_label='Image Filename',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED_WITH_RAW_FALLBACK,
                display_label='Data',
                codec='struct:I,I,I,i,i,i,i,i,i,i,I,I,b,B,H',
                fields=(
                    FieldSpec(
                        name='primary_attributes_attribute_1',
                        kind='uint32',
                        enum_ref='attribute_enum',
                        authoring_label='Primary Attributes Attribute #1',
                    ),
                    FieldSpec(
                        name='primary_attributes_attribute_2',
                        kind='uint32',
                        enum_ref='attribute_enum',
                        authoring_label='Primary Attributes Attribute #2',
                    ),
                    FieldSpec(
                        name='specialization',
                        kind='uint32',
                        enum_ref='specialization_enum',
                        authoring_label='Specialization',
                    ),
                    FieldSpec(
                        name='major_skills_skill_1',
                        kind='int32',
                        enum_ref='major_skill_enum',
                        authoring_label='Major Skills Skill #1',
                    ),
                    FieldSpec(
                        name='major_skills_skill_2',
                        kind='int32',
                        enum_ref='major_skill_enum',
                        authoring_label='Major Skills Skill #2',
                    ),
                    FieldSpec(
                        name='major_skills_skill_3',
                        kind='int32',
                        enum_ref='major_skill_enum',
                        authoring_label='Major Skills Skill #3',
                    ),
                    FieldSpec(
                        name='major_skills_skill_4',
                        kind='int32',
                        enum_ref='major_skill_enum',
                        authoring_label='Major Skills Skill #4',
                    ),
                    FieldSpec(
                        name='major_skills_skill_5',
                        kind='int32',
                        enum_ref='major_skill_enum',
                        authoring_label='Major Skills Skill #5',
                    ),
                    FieldSpec(
                        name='major_skills_skill_6',
                        kind='int32',
                        enum_ref='major_skill_enum',
                        authoring_label='Major Skills Skill #6',
                    ),
                    FieldSpec(
                        name='major_skills_skill_7',
                        kind='int32',
                        enum_ref='major_skill_enum',
                        authoring_label='Major Skills Skill #7',
                    ),
                    FieldSpec(
                        name='flags',
                        kind='uint32',
                        enum_ref='CLAS.DATA.flags',
                        authoring_label='Flags',
                    ),
                    FieldSpec(
                        name='buys_sells_and_services',
                        kind='uint32',
                        enum_ref='service_flags',
                        authoring_label='Buys/Sells and Services',
                    ),
                    FieldSpec(
                        name='teaches',
                        kind='int8',
                        enum_ref='skill_enum',
                        authoring_label='Teaches',
                    ),
                    FieldSpec(
                        name='maximum_training_level',
                        kind='uint8',
                        authoring_label='Maximum training level',
                    ),
                    FieldSpec(
                        name='unused',
                        kind='uint16',
                        authoring_label='Unused',
                    ),
                ),
            ),
        ),
        display_label='Class',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['CLMT'] = RecordSpec(
        sig='CLMT',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='WLST',
                kind=FieldKind.PARSED,
                display_label='Weather Types',
                codec='array_struct:I,i',
                fields=(
                    FieldSpec(
                        name='weather_types_weather',
                        kind='formid',
                        formlink_target='WTHR',
                        formlink_targets=('WTHR',),
                        authoring_label='Weather Types Weather',
                    ),
                    FieldSpec(
                        name='weather_types_chance',
                        kind='int32',
                        authoring_label='Weather Types Chance',
                    ),
                ),
                array=ArraySpec(layout='row_array', element_codec='I,i'),
                row_label='Weather Types',
            ),
            SubrecordSpec(
                sig='FNAM',
                kind=FieldKind.PARSED,
                display_label='Sun Texture',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='sun_texture',
                        kind='zstring',
                        authoring_label='Sun Texture',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='GNAM',
                kind=FieldKind.PARSED,
                display_label='Sun Glare Texture',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='sun_glare_texture',
                        kind='zstring',
                        authoring_label='Sun Glare Texture',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='model_filename',
                        kind='zstring',
                        authoring_label='Model FileName',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='TNAM',
                kind=FieldKind.PARSED,
                display_label='Timing',
                codec='struct:B,B,B,B,B,B',
                fields=(
                    FieldSpec(
                        name='sunrise_begin',
                        kind='uint8',
                        authoring_label='Sunrise Begin',
                    ),
                    FieldSpec(
                        name='sunrise_end',
                        kind='uint8',
                        authoring_label='Sunrise End',
                    ),
                    FieldSpec(
                        name='sunset_begin',
                        kind='uint8',
                        authoring_label='Sunset Begin',
                    ),
                    FieldSpec(
                        name='sunset_end',
                        kind='uint8',
                        authoring_label='Sunset End',
                    ),
                    FieldSpec(
                        name='volatility',
                        kind='uint8',
                        authoring_label='Volatility',
                    ),
                    FieldSpec(
                        name='moons_phase_length',
                        kind='uint8',
                        authoring_label='Moons / Phase Length',
                    ),
                ),
                required=True,
            ),
        ),
        display_label='Climate',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['CLOT'] = RecordSpec(
        sig='CLOT',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='SCRI',
                kind=FieldKind.PARSED,
                display_label='Script',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='script',
                        kind='formid',
                        formlink_target='SCPT',
                        formlink_targets=('SCPT',),
                        authoring_label='Script',
                    ),
                ),
                formlink_target='SCPT',
                formlink_targets=('SCPT',),
            ),
            SubrecordSpec(
                sig='EITM',
                kind=FieldKind.PARSED,
                display_label='Effect',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='effect',
                        kind='formid',
                        formlink_target='ENCH',
                        formlink_targets=('ENCH',),
                        authoring_label='Effect',
                    ),
                ),
                repeatable=True,
                formlink_target='ENCH',
                formlink_targets=('ENCH',),
                scope_id='enchantment',
            ),
            SubrecordSpec(
                sig='EAMT',
                kind=FieldKind.PARSED,
                display_label='Capacity',
                codec='uint16',
                fields=(
                    FieldSpec(
                        name='capacity',
                        kind='uint16',
                        authoring_label='Capacity',
                    ),
                ),
                repeatable=True,
                scope_id='enchantment',
            ),
            SubrecordSpec(
                sig='BMDT',
                kind=FieldKind.PARSED,
                display_label='Flags',
                codec='struct:H,B,B',
                fields=(
                    FieldSpec(
                        name='biped_flags',
                        kind='uint16',
                        enum_ref='biped_flags',
                        authoring_label='Biped Flags',
                    ),
                    FieldSpec(
                        name='general_flags',
                        kind='uint8',
                        enum_ref='CLOT.BMDT.general_flags',
                        authoring_label='General Flags',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Unknown Byte 3',
                        notes='wbUnused(1)',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='Biped Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='biped_model_filename',
                        kind='zstring',
                        authoring_label='Biped Model FileName',
                    ),
                ),
                repeatable=True,
                scope_id='male',
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
                repeatable=True,
                scope_id='male',
            ),
            SubrecordSpec(
                sig='MOD2',
                kind=FieldKind.PARSED,
                display_label='World Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='world_model_filename',
                        kind='zstring',
                        authoring_label='World Model FileName',
                    ),
                ),
                repeatable=True,
                scope_id='male',
            ),
            SubrecordSpec(
                sig='MO2B',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
                repeatable=True,
                scope_id='male',
            ),
            SubrecordSpec(
                sig='ICON',
                kind=FieldKind.PARSED,
                display_label='Icon Image',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='icon_image',
                        kind='zstring',
                        authoring_label='Icon Image',
                    ),
                ),
                repeatable=True,
                scope_id='male',
            ),
            SubrecordSpec(
                sig='MOD3',
                kind=FieldKind.PARSED,
                display_label='Biped Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='biped_model_filename',
                        kind='zstring',
                        authoring_label='Biped Model FileName',
                    ),
                ),
                repeatable=True,
                scope_id='female',
            ),
            SubrecordSpec(
                sig='MO3B',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
                repeatable=True,
                scope_id='female',
            ),
            SubrecordSpec(
                sig='MOD4',
                kind=FieldKind.PARSED,
                display_label='World Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='world_model_filename',
                        kind='zstring',
                        authoring_label='World Model FileName',
                    ),
                ),
                repeatable=True,
                scope_id='female',
            ),
            SubrecordSpec(
                sig='MO4B',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
                repeatable=True,
                scope_id='female',
            ),
            SubrecordSpec(
                sig='ICO2',
                kind=FieldKind.PARSED,
                display_label='Icon Image',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='icon_image',
                        kind='zstring',
                        authoring_label='Icon Image',
                    ),
                ),
                repeatable=True,
                scope_id='female',
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                display_label='Data',
                codec='struct:I,f',
                fields=(
                    FieldSpec(
                        name='value',
                        kind='uint32',
                        authoring_label='Value',
                    ),
                    FieldSpec(
                        name='weight',
                        kind='float32',
                        authoring_label='Weight',
                    ),
                ),
                required=True,
            ),
        ),
        display_label='Clothing',
        record_flags=RecordFlagsSpec(valid_mask=5152, bits=(RecordFlagBit(bit=10, name='Quest Item'),)),
    )

    records['CONT'] = RecordSpec(
        sig='CONT',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='model_filename',
                        kind='zstring',
                        authoring_label='Model FileName',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='SCRI',
                kind=FieldKind.PARSED,
                display_label='Script',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='script',
                        kind='formid',
                        formlink_target='SCPT',
                        formlink_targets=('SCPT',),
                        authoring_label='Script',
                    ),
                ),
                formlink_target='SCPT',
                formlink_targets=('SCPT',),
            ),
            SubrecordSpec(
                sig='CNTO',
                kind=FieldKind.PARSED,
                display_label='Items',
                codec='struct:I,i',
                fields=(
                    FieldSpec(
                        name='item',
                        kind='formid',
                        formlink_targets=('ALCH', 'AMMO', 'APPA', 'ARMO', 'BOOK', 'CLOT', 'INGR', 'KEYM', 'LIGH', 'LVLI', 'MISC', 'SGST', 'SLGM', 'WEAP'),
                        authoring_label='Item',
                    ),
                    FieldSpec(
                        name='count',
                        kind='int32',
                        authoring_label='Count',
                    ),
                ),
                repeatable=True,
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                display_label='Data',
                codec='struct:B,f',
                fields=(
                    FieldSpec(
                        name='flags',
                        kind='uint8',
                        enum_ref='CONT.DATA.flags',
                        authoring_label='Flags',
                    ),
                    FieldSpec(
                        name='weight',
                        kind='float32',
                        authoring_label='Weight',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='SNAM',
                kind=FieldKind.PARSED,
                display_label='Open Sound',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='open_sound',
                        kind='formid',
                        formlink_target='SOUN',
                        formlink_targets=('SOUN',),
                        authoring_label='Open Sound',
                    ),
                ),
                formlink_target='SOUN',
                formlink_targets=('SOUN',),
            ),
            SubrecordSpec(
                sig='QNAM',
                kind=FieldKind.PARSED,
                display_label='Close Sound',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='close_sound',
                        kind='formid',
                        formlink_target='SOUN',
                        formlink_targets=('SOUN',),
                        authoring_label='Close Sound',
                    ),
                ),
                formlink_target='SOUN',
                formlink_targets=('SOUN',),
            ),
        ),
        display_label='Container',
        record_flags=RecordFlagsSpec(valid_mask=5152, bits=(RecordFlagBit(bit=10, name='Quest Item'),)),
    )

    records['CREA'] = RecordSpec(
        sig='CREA',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='model_filename',
                        kind='zstring',
                        authoring_label='Model FileName',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='CNTO',
                kind=FieldKind.PARSED,
                display_label='Items',
                codec='struct:I,i',
                fields=(
                    FieldSpec(
                        name='item',
                        kind='formid',
                        formlink_targets=('ALCH', 'AMMO', 'APPA', 'ARMO', 'BOOK', 'CLOT', 'INGR', 'KEYM', 'LIGH', 'LVLI', 'MISC', 'SGST', 'SLGM', 'WEAP'),
                        authoring_label='Item',
                    ),
                    FieldSpec(
                        name='count',
                        kind='int32',
                        authoring_label='Count',
                    ),
                ),
                repeatable=True,
            ),
            SubrecordSpec(sig='SPLO', kind=FieldKind.RAW, display_label='Spell', repeatable=True),
            SubrecordSpec(sig='NIFZ', kind=FieldKind.RAW, display_label='Model List'),
            SubrecordSpec(sig='NIFT', kind=FieldKind.RAW, display_label='Model List Textures', required=True),
            SubrecordSpec(
                sig='ACBS',
                kind=FieldKind.PARSED,
                display_label='Configuration',
                codec='struct:I,H,H,H,h,H,H',
                fields=(
                    FieldSpec(
                        name='flags',
                        kind='uint32',
                        enum_ref='CREA.ACBS.flags',
                        authoring_label='Flags',
                    ),
                    FieldSpec(
                        name='base_spell_points',
                        kind='uint16',
                        authoring_label='Base spell points',
                    ),
                    FieldSpec(
                        name='fatigue',
                        kind='uint16',
                        authoring_label='Fatigue',
                    ),
                    FieldSpec(
                        name='barter_gold',
                        kind='uint16',
                        authoring_label='Barter gold',
                    ),
                    FieldSpec(
                        name='level_offset',
                        kind='int16',
                        authoring_label='Level (offset)',
                    ),
                    FieldSpec(
                        name='calc_min',
                        kind='uint16',
                        authoring_label='Calc min',
                    ),
                    FieldSpec(
                        name='calc_max',
                        kind='uint16',
                        authoring_label='Calc max',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='SNAM',
                kind=FieldKind.PARSED,
                display_label='Factions',
                codec='struct:I,b,B,B,B',
                fields=(
                    FieldSpec(
                        name='faction',
                        kind='formid',
                        formlink_target='FACT',
                        formlink_targets=('FACT',),
                        authoring_label='Faction',
                    ),
                    FieldSpec(
                        name='rank',
                        kind='int8',
                        authoring_label='Rank',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Unknown Byte 3',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Unknown Byte 4',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_4',
                        kind='uint8',
                        authoring_label='Unknown Byte 5',
                        notes='wbUnused(3)',
                    ),
                ),
                repeatable=True,
            ),
            SubrecordSpec(
                sig='INAM',
                kind=FieldKind.PARSED,
                display_label='Death item',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='death_item',
                        kind='formid',
                        formlink_target='LVLI',
                        formlink_targets=('LVLI',),
                        authoring_label='Death item',
                    ),
                ),
                formlink_target='LVLI',
                formlink_targets=('LVLI',),
            ),
            SubrecordSpec(
                sig='SCRI',
                kind=FieldKind.PARSED,
                display_label='Script',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='script',
                        kind='formid',
                        formlink_target='SCPT',
                        formlink_targets=('SCPT',),
                        authoring_label='Script',
                    ),
                ),
                formlink_target='SCPT',
                formlink_targets=('SCPT',),
            ),
            SubrecordSpec(
                sig='AIDT',
                kind=FieldKind.PARSED,
                display_label='AI Data',
                codec='struct:B,B,B,B,I,b,B,B,B',
                fields=(
                    FieldSpec(
                        name='aggression',
                        kind='uint8',
                        authoring_label='Aggression',
                    ),
                    FieldSpec(
                        name='confidence',
                        kind='uint8',
                        authoring_label='Confidence',
                    ),
                    FieldSpec(
                        name='energy_level',
                        kind='uint8',
                        authoring_label='Energy Level',
                    ),
                    FieldSpec(
                        name='responsibility',
                        kind='uint8',
                        authoring_label='Responsibility',
                    ),
                    FieldSpec(
                        name='buys_sells_and_services',
                        kind='uint32',
                        enum_ref='service_flags',
                        authoring_label='Buys/Sells and Services',
                    ),
                    FieldSpec(
                        name='teaches',
                        kind='int8',
                        enum_ref='skill_enum',
                        authoring_label='Teaches',
                    ),
                    FieldSpec(
                        name='maximum_training_level',
                        kind='uint8',
                        authoring_label='Maximum training level',
                    ),
                    FieldSpec(
                        name='unknown_u8_7',
                        kind='uint8',
                        authoring_label='Unknown Byte 8',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='unknown_u8_8',
                        kind='uint8',
                        authoring_label='Unknown Byte 9',
                        notes='wbUnused(2)',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='PKID',
                kind=FieldKind.PARSED,
                display_label='AI Package',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='ai_package',
                        kind='formid',
                        formlink_target='PACK',
                        formlink_targets=('PACK',),
                        authoring_label='AI Package',
                    ),
                ),
                repeatable=True,
                formlink_target='PACK',
                formlink_targets=('PACK',),
            ),
            SubrecordSpec(sig='KFFZ', kind=FieldKind.RAW, display_label='Animations'),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                display_label='Creature Data',
                codec='struct:B,B,B,B,B,B,H,B,B,H,B,B,B,B,B,B,B,B',
                fields=(
                    FieldSpec(
                        name='type',
                        kind='uint8',
                        enum_ref='CREA.DATA.type',
                        authoring_label='Type',
                    ),
                    FieldSpec(
                        name='combat_skill',
                        kind='uint8',
                        authoring_label='Combat Skill',
                    ),
                    FieldSpec(
                        name='magic_skill',
                        kind='uint8',
                        authoring_label='Magic Skill',
                    ),
                    FieldSpec(
                        name='stealth_skill',
                        kind='uint8',
                        authoring_label='Stealth Skill',
                    ),
                    FieldSpec(
                        name='soul',
                        kind='uint8',
                        enum_ref='soul_gem_enum',
                        authoring_label='Soul',
                    ),
                    FieldSpec(
                        name='unknown_u8_5',
                        kind='uint8',
                        authoring_label='Unknown Byte 6',
                        notes='wbUnused(1)',
                    ),
                    FieldSpec(
                        name='health',
                        kind='uint16',
                        authoring_label='Health',
                    ),
                    FieldSpec(
                        name='unknown_u8_7',
                        kind='uint8',
                        authoring_label='Unknown Byte 8',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='unknown_u8_8',
                        kind='uint8',
                        authoring_label='Unknown Byte 9',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='attack_damage',
                        kind='uint16',
                        authoring_label='Attack Damage',
                    ),
                    FieldSpec(
                        name='strength',
                        kind='uint8',
                        authoring_label='Strength',
                    ),
                    FieldSpec(
                        name='intelligence',
                        kind='uint8',
                        authoring_label='Intelligence',
                    ),
                    FieldSpec(
                        name='willpower',
                        kind='uint8',
                        authoring_label='Willpower',
                    ),
                    FieldSpec(
                        name='agility',
                        kind='uint8',
                        authoring_label='Agility',
                    ),
                    FieldSpec(
                        name='speed',
                        kind='uint8',
                        authoring_label='Speed',
                    ),
                    FieldSpec(
                        name='endurance',
                        kind='uint8',
                        authoring_label='Endurance',
                    ),
                    FieldSpec(
                        name='personality',
                        kind='uint8',
                        authoring_label='Personality',
                    ),
                    FieldSpec(
                        name='luck',
                        kind='uint8',
                        authoring_label='Luck',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='RNAM',
                kind=FieldKind.PARSED,
                display_label='Attack reach',
                codec='uint8',
                fields=(
                    FieldSpec(
                        name='attack_reach',
                        kind='uint8',
                        authoring_label='Attack reach',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='ZNAM',
                kind=FieldKind.PARSED,
                display_label='Combat Style',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='combat_style',
                        kind='formid',
                        formlink_target='CSTY',
                        formlink_targets=('CSTY',),
                        authoring_label='Combat Style',
                    ),
                ),
                formlink_target='CSTY',
                formlink_targets=('CSTY',),
            ),
            SubrecordSpec(
                sig='TNAM',
                kind=FieldKind.PARSED,
                display_label='Turning Speed',
                codec='float32',
                fields=(
                    FieldSpec(name='float32_0', kind='float32'),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='BNAM',
                kind=FieldKind.PARSED,
                display_label='Base Scale',
                codec='float32',
                fields=(
                    FieldSpec(name='float32_0', kind='float32'),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='WNAM',
                kind=FieldKind.PARSED,
                display_label='Foot Weight',
                codec='float32',
                fields=(
                    FieldSpec(name='float32_0', kind='float32'),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='NAM0',
                kind=FieldKind.PARSED,
                display_label='Blood Spray',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='blood_spray',
                        kind='zstring',
                        authoring_label='Blood Spray',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='NAM1',
                kind=FieldKind.PARSED,
                display_label='Blood Decal',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='blood_decal',
                        kind='zstring',
                        authoring_label='Blood Decal',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='CSCR',
                kind=FieldKind.PARSED,
                display_label='Inherits Sounds from',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='inherits_sounds_from',
                        kind='formid',
                        formlink_target='CREA',
                        formlink_targets=('CREA',),
                        authoring_label='Inherits Sounds from',
                    ),
                ),
                formlink_target='CREA',
                formlink_targets=('CREA',),
            ),
            SubrecordSpec(
                sig='CSDT',
                kind=FieldKind.PARSED,
                display_label='Type',
                codec='uint32',
                fields=(
                    FieldSpec(
                        name='type',
                        kind='uint32',
                        enum_ref='CREA.CSDT.type',
                        authoring_label='Type',
                    ),
                ),
                repeatable=True,
                enum_ref='CREA.CSDT.type',
                scope_id='sound_types',
            ),
            SubrecordSpec(
                sig='CSDI',
                kind=FieldKind.PARSED,
                display_label='Sound',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='sound',
                        kind='formid',
                        formlink_target='SOUN',
                        formlink_targets=('SOUN',),
                        null_allowed=True,
                        authoring_label='Sound',
                    ),
                ),
                repeatable=True,
                required=True,
                formlink_target='SOUN',
                formlink_targets=('SOUN',),
                null_allowed=True,
                scope_id='sound_types',
            ),
            SubrecordSpec(
                sig='CSDC',
                kind=FieldKind.PARSED,
                display_label='Sound Chance',
                codec='uint8',
                fields=(
                    FieldSpec(
                        name='sound_chance',
                        kind='uint8',
                        authoring_label='Sound Chance',
                    ),
                ),
                repeatable=True,
                required=True,
                scope_id='sound_types',
            ),
        ),
        display_label='Creature',
        record_flags=RecordFlagsSpec(valid_mask=529440, bits=(RecordFlagBit(bit=10, name='Quest Item'), RecordFlagBit(bit=19, name='Starts Dead'))),
    )

    records['CSTY'] = RecordSpec(
        sig='CSTY',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='CSTD',
                kind=FieldKind.PARSED_WITH_RAW_FALLBACK,
                display_label='Standard',
                codec='struct:B,B,B,B,f,f,f,f,f,f,f,f,B,B,B,B,f,f,f,B,B,B,B,f,f,B,B,B,B,B,B,B,B,f,f,B,B,B,B,f,f,f,f,f,f,f,B,B,B,B,f,I',
                fields=(
                    FieldSpec(
                        name='dodge_chance',
                        kind='uint8',
                        authoring_label='Dodge % Chance',
                    ),
                    FieldSpec(
                        name='left_right_chance',
                        kind='uint8',
                        authoring_label='Left/Right % Chance',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Unknown Byte 3',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Unknown Byte 4',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='dodge_l_r_timer_min',
                        kind='float32',
                        authoring_label='Dodge L/R Timer Min',
                    ),
                    FieldSpec(
                        name='dodge_l_r_timer_max',
                        kind='float32',
                        authoring_label='Dodge L/R Timer Max',
                    ),
                    FieldSpec(
                        name='dodge_forward_timer_min',
                        kind='float32',
                        authoring_label='Dodge Forward Timer Min',
                    ),
                    FieldSpec(
                        name='dodge_forward_timer_max',
                        kind='float32',
                        authoring_label='Dodge Forward Timer Max',
                    ),
                    FieldSpec(
                        name='dodge_back_timer_min',
                        kind='float32',
                        authoring_label='Dodge Back Timer Min',
                    ),
                    FieldSpec(
                        name='dodge_back_timer_max',
                        kind='float32',
                        authoring_label='Dodge Back Timer Max',
                    ),
                    FieldSpec(
                        name='idle_timer_min',
                        kind='float32',
                        authoring_label='Idle Timer Min',
                    ),
                    FieldSpec(
                        name='idle_timer_max',
                        kind='float32',
                        authoring_label='Idle Timer Max',
                    ),
                    FieldSpec(
                        name='block_chance',
                        kind='uint8',
                        authoring_label='Block % Chance',
                    ),
                    FieldSpec(
                        name='attack_chance',
                        kind='uint8',
                        authoring_label='Attack % Chance',
                    ),
                    FieldSpec(
                        name='unknown_u8_14',
                        kind='uint8',
                        authoring_label='Unknown Byte 15',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='unknown_u8_15',
                        kind='uint8',
                        authoring_label='Unknown Byte 16',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='recoil_stagger_bonus_to_attack',
                        kind='float32',
                        authoring_label='Recoil/Stagger Bonus to Attack',
                    ),
                    FieldSpec(
                        name='unconscious_bonus_to_attack',
                        kind='float32',
                        authoring_label='Unconscious Bonus to Attack',
                    ),
                    FieldSpec(
                        name='hand_to_hand_bonus_to_attack',
                        kind='float32',
                        authoring_label='Hand-To-Hand Bonus to Attack',
                    ),
                    FieldSpec(
                        name='power_attack_chance',
                        kind='uint8',
                        authoring_label='Power Attack % Chance',
                    ),
                    FieldSpec(
                        name='unknown_u8_20',
                        kind='uint8',
                        authoring_label='Unknown Byte 21',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_21',
                        kind='uint8',
                        authoring_label='Unknown Byte 22',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_22',
                        kind='uint8',
                        authoring_label='Unknown Byte 23',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='recoil_stagger_bonus_to_power_attack',
                        kind='float32',
                        authoring_label='Recoil/Stagger Bonus to Power Attack',
                    ),
                    FieldSpec(
                        name='unconscious_bonus_to_power_attack',
                        kind='float32',
                        authoring_label='Unconscious Bonus to Power Attack',
                    ),
                    FieldSpec(
                        name='power_attack_normal',
                        kind='uint8',
                        authoring_label='Power Attack Normal',
                    ),
                    FieldSpec(
                        name='power_attack_forward',
                        kind='uint8',
                        authoring_label='Power Attack Forward',
                    ),
                    FieldSpec(
                        name='power_attack_back',
                        kind='uint8',
                        authoring_label='Power Attack Back',
                    ),
                    FieldSpec(
                        name='power_attack_left',
                        kind='uint8',
                        authoring_label='Power Attack Left',
                    ),
                    FieldSpec(
                        name='power_attack_right',
                        kind='uint8',
                        authoring_label='Power Attack Right',
                    ),
                    FieldSpec(
                        name='unknown_u8_30',
                        kind='uint8',
                        authoring_label='Unknown Byte 31',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_31',
                        kind='uint8',
                        authoring_label='Unknown Byte 32',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_32',
                        kind='uint8',
                        authoring_label='Unknown Byte 33',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='hold_timer_min',
                        kind='float32',
                        authoring_label='Hold Timer Min',
                    ),
                    FieldSpec(
                        name='hold_timer_max',
                        kind='float32',
                        authoring_label='Hold Timer Max',
                    ),
                    FieldSpec(
                        name='flags',
                        kind='uint8',
                        enum_ref='CSTY.CSTD.flags',
                        authoring_label='Flags',
                    ),
                    FieldSpec(
                        name='acrobatic_dodge_chance',
                        kind='uint8',
                        authoring_label='Acrobatic Dodge % Chance',
                    ),
                    FieldSpec(
                        name='unknown_u8_37',
                        kind='uint8',
                        authoring_label='Unknown Byte 38',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='unknown_u8_38',
                        kind='uint8',
                        authoring_label='Unknown Byte 39',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='range_mult_optimal',
                        kind='float32',
                        authoring_label='Range Mult (Optimal)',
                    ),
                    FieldSpec(
                        name='range_mult_max',
                        kind='float32',
                        authoring_label='Range Mult (Max)',
                    ),
                    FieldSpec(
                        name='switch_distance_melee',
                        kind='float32',
                        authoring_label='Switch Distance (Melee)',
                    ),
                    FieldSpec(
                        name='switch_distance_ranged',
                        kind='float32',
                        authoring_label='Switch Distance (Ranged)',
                    ),
                    FieldSpec(
                        name='buff_standoff_distance',
                        kind='float32',
                        authoring_label='Buff standoff Distance',
                    ),
                    FieldSpec(
                        name='ranged_standoff_distance',
                        kind='float32',
                        authoring_label='Ranged standoff Distance',
                    ),
                    FieldSpec(
                        name='group_standoff_distance',
                        kind='float32',
                        authoring_label='Group standoff Distance',
                    ),
                    FieldSpec(
                        name='rushing_attack_chance',
                        kind='uint8',
                        authoring_label='Rushing Attack % Chance',
                    ),
                    FieldSpec(
                        name='unknown_u8_47',
                        kind='uint8',
                        authoring_label='Unknown Byte 48',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_48',
                        kind='uint8',
                        authoring_label='Unknown Byte 49',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_49',
                        kind='uint8',
                        authoring_label='Unknown Byte 50',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='rushing_attack_distance_mult',
                        kind='float32',
                        authoring_label='Rushing Attack Distance Mult',
                    ),
                    FieldSpec(
                        name='do_not_acquire',
                        kind='uint32',
                        enum_ref='bool_enum',
                        authoring_label='Do Not Acquire',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='CSAD',
                kind=FieldKind.PARSED,
                display_label='Advanced',
                codec='struct:f,f,f,f,f,f,f,f,f,f,f,f,f,f,f,f,f,f,f,f,f',
                fields=(
                    FieldSpec(
                        name='dodge_fatigue_mod_mult',
                        kind='float32',
                        authoring_label='Dodge Fatigue Mod Mult',
                    ),
                    FieldSpec(
                        name='dodge_fatigue_mod_base',
                        kind='float32',
                        authoring_label='Dodge Fatigue Mod Base',
                    ),
                    FieldSpec(
                        name='encumbered_speed_mod_base',
                        kind='float32',
                        authoring_label='Encumbered Speed Mod Base',
                    ),
                    FieldSpec(
                        name='encumbered_speed_mod_mult',
                        kind='float32',
                        authoring_label='Encumbered Speed Mod Mult',
                    ),
                    FieldSpec(
                        name='dodge_while_under_attack_mult',
                        kind='float32',
                        authoring_label='Dodge While Under Attack Mult',
                    ),
                    FieldSpec(
                        name='dodge_not_under_attack_mult',
                        kind='float32',
                        authoring_label='Dodge Not Under Attack Mult',
                    ),
                    FieldSpec(
                        name='dodge_back_while_under_attack_mult',
                        kind='float32',
                        authoring_label='Dodge Back While Under Attack Mult',
                    ),
                    FieldSpec(
                        name='dodge_back_not_under_attack_mult',
                        kind='float32',
                        authoring_label='Dodge Back Not Under Attack Mult',
                    ),
                    FieldSpec(
                        name='dodge_forward_while_attacking_mult',
                        kind='float32',
                        authoring_label='Dodge Forward While Attacking Mult',
                    ),
                    FieldSpec(
                        name='dodge_forward_not_attacking_mult',
                        kind='float32',
                        authoring_label='Dodge Forward Not Attacking Mult',
                    ),
                    FieldSpec(
                        name='block_skill_modifier_mult',
                        kind='float32',
                        authoring_label='Block Skill Modifier Mult',
                    ),
                    FieldSpec(
                        name='block_skill_modifier_base',
                        kind='float32',
                        authoring_label='Block Skill Modifier Base',
                    ),
                    FieldSpec(
                        name='block_while_under_attack_mult',
                        kind='float32',
                        authoring_label='Block While Under Attack Mult',
                    ),
                    FieldSpec(
                        name='block_not_under_attack_mult',
                        kind='float32',
                        authoring_label='Block Not Under Attack Mult',
                    ),
                    FieldSpec(
                        name='attack_skill_modifier_mult',
                        kind='float32',
                        authoring_label='Attack Skill Modifier Mult',
                    ),
                    FieldSpec(
                        name='attack_skill_modifier_base',
                        kind='float32',
                        authoring_label='Attack Skill Modifier Base',
                    ),
                    FieldSpec(
                        name='attack_while_under_attack_mult',
                        kind='float32',
                        authoring_label='Attack While Under Attack Mult',
                    ),
                    FieldSpec(
                        name='attack_not_under_attack_mult',
                        kind='float32',
                        authoring_label='Attack Not Under Attack Mult',
                    ),
                    FieldSpec(
                        name='attack_during_block_mult',
                        kind='float32',
                        authoring_label='Attack During Block Mult',
                    ),
                    FieldSpec(
                        name='power_attack_fatigue_mod_base',
                        kind='float32',
                        authoring_label='Power Attack Fatigue Mod Base',
                    ),
                    FieldSpec(
                        name='power_attack_fatigue_mod_mult',
                        kind='float32',
                        authoring_label='Power Attack Fatigue Mod Mult',
                    ),
                ),
            ),
        ),
        display_label='Combat Style',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['DIAL'] = RecordSpec(
        sig='DIAL',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='QSTI',
                kind=FieldKind.PARSED,
                display_label='Associated Quest',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='associated_quest',
                        kind='formid',
                        formlink_target='QUST',
                        formlink_targets=('QUST',),
                        authoring_label='Associated Quest',
                    ),
                ),
                repeatable=True,
                formlink_target='QUST',
                formlink_targets=('QUST',),
            ),
            SubrecordSpec(
                sig='QSTR',
                kind=FieldKind.PARSED,
                display_label='Removed Quest',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='removed_quest',
                        kind='formid',
                        formlink_target='QUST',
                        formlink_targets=('QUST',),
                        authoring_label='Removed Quest',
                    ),
                ),
                repeatable=True,
                formlink_target='QUST',
                formlink_targets=('QUST',),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                display_label='Type',
                codec='uint8',
                fields=(
                    FieldSpec(
                        name='type',
                        kind='uint8',
                        enum_ref='dialogue_type_enum',
                        authoring_label='Type',
                    ),
                ),
                required=True,
                enum_ref='dialogue_type_enum',
            ),
            SubrecordSpec(
                sig='INOM',
                kind=FieldKind.PARSED,
                display_label='INFO Order (Masters only)',
                codec='formid_array',
                fields=(
                    FieldSpec(
                        name='info_order_masters_only_info',
                        kind='formid',
                        formlink_target='INFO',
                        formlink_targets=('INFO',),
                        authoring_label='INFO Order (Masters only) INFO',
                    ),
                ),
                formlink_target='INFO',
                formlink_targets=('INFO',),
            ),
            SubrecordSpec(
                sig='INOA',
                kind=FieldKind.PARSED,
                display_label='INFO Order (All previous modules)',
                codec='formid_array',
                fields=(
                    FieldSpec(
                        name='info_order_all_previous_modules_info',
                        kind='formid',
                        formlink_target='INFO',
                        formlink_targets=('INFO',),
                        authoring_label='INFO Order (All previous modules) INFO',
                    ),
                ),
                formlink_target='INFO',
                formlink_targets=('INFO',),
            ),
        ),
        display_label='Dialog Topic',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['DOOR'] = RecordSpec(
        sig='DOOR',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='model_filename',
                        kind='zstring',
                        authoring_label='Model FileName',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='SCRI',
                kind=FieldKind.PARSED,
                display_label='Script',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='script',
                        kind='formid',
                        formlink_target='SCPT',
                        formlink_targets=('SCPT',),
                        authoring_label='Script',
                    ),
                ),
                formlink_target='SCPT',
                formlink_targets=('SCPT',),
            ),
            SubrecordSpec(
                sig='SNAM',
                kind=FieldKind.PARSED,
                display_label='Open Sound',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='open_sound',
                        kind='formid',
                        formlink_target='SOUN',
                        formlink_targets=('SOUN',),
                        authoring_label='Open Sound',
                    ),
                ),
                formlink_target='SOUN',
                formlink_targets=('SOUN',),
            ),
            SubrecordSpec(
                sig='ANAM',
                kind=FieldKind.PARSED,
                display_label='Close Sound',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='close_sound',
                        kind='formid',
                        formlink_target='SOUN',
                        formlink_targets=('SOUN',),
                        authoring_label='Close Sound',
                    ),
                ),
                formlink_target='SOUN',
                formlink_targets=('SOUN',),
            ),
            SubrecordSpec(
                sig='BNAM',
                kind=FieldKind.PARSED,
                display_label='Loop Sound',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='loop_sound',
                        kind='formid',
                        formlink_target='SOUN',
                        formlink_targets=('SOUN',),
                        authoring_label='Loop Sound',
                    ),
                ),
                formlink_target='SOUN',
                formlink_targets=('SOUN',),
            ),
            SubrecordSpec(
                sig='FNAM',
                kind=FieldKind.PARSED,
                display_label='Flags',
                codec='uint8',
                fields=(
                    FieldSpec(
                        name='flags',
                        kind='uint8',
                        enum_ref='DOOR.FNAM.flags',
                        authoring_label='Flags',
                    ),
                ),
                required=True,
                enum_ref='DOOR.FNAM.flags',
            ),
            SubrecordSpec(
                sig='TNAM',
                kind=FieldKind.PARSED,
                display_label='Destination',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='destination',
                        kind='formid',
                        formlink_targets=('CELL', 'WRLD'),
                        authoring_label='Destination',
                    ),
                ),
                repeatable=True,
                formlink_targets=('CELL', 'WRLD'),
            ),
        ),
        display_label='Door',
        record_flags=RecordFlagsSpec(valid_mask=5152, bits=(RecordFlagBit(bit=10, name='Quest Item'),)),
    )

    records['EFSH'] = RecordSpec(
        sig='EFSH',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='ICON',
                kind=FieldKind.PARSED,
                display_label='Fill Texture',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='fill_texture',
                        kind='zstring',
                        authoring_label='Fill Texture',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='ICO2',
                kind=FieldKind.PARSED,
                display_label='Particle Shader Texture',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='particle_shader_texture',
                        kind='zstring',
                        authoring_label='Particle Shader Texture',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED_WITH_RAW_FALLBACK,
                display_label='Data',
                codec='struct:B,B,B,B,I,I,I,B,B,B,B,f,f,f,f,f,f,f,f,f,B,B,B,B,f,f,f,f,f,f,f,f,I,I,I,I,I,f,f,f,f,f,f,f,f,f,f,f,f,f,f,f,f,f,f,f,B,B,B,B,B,B,B,B,B,B,B,B,f,f,f,f,f,f',
                fields=(
                    FieldSpec(
                        name='flags',
                        kind='uint8',
                        enum_ref='EFSH.DATA.flags',
                        authoring_label='Flags',
                    ),
                    FieldSpec(
                        name='unknown_u8_1',
                        kind='uint8',
                        authoring_label='Unknown Byte 2',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Unknown Byte 3',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Unknown Byte 4',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='membrane_shader_source_blend_mode',
                        kind='uint32',
                        enum_ref='blend_mode_enum',
                        authoring_label='Membrane Shader Source Blend Mode',
                    ),
                    FieldSpec(
                        name='membrane_shader_blend_operation',
                        kind='uint32',
                        enum_ref='blend_op_enum',
                        authoring_label='Membrane Shader Blend Operation',
                    ),
                    FieldSpec(
                        name='membrane_shader_z_test_function',
                        kind='uint32',
                        enum_ref='z_test_func_enum',
                        authoring_label='Membrane Shader Z Test Function',
                    ),
                    FieldSpec(
                        name='fill_texture_effect_color_red',
                        kind='uint8',
                        authoring_label='Fill/Texture Effect Color Red',
                    ),
                    FieldSpec(
                        name='fill_texture_effect_color_green',
                        kind='uint8',
                        authoring_label='Fill/Texture Effect Color Green',
                    ),
                    FieldSpec(
                        name='fill_texture_effect_color_blue',
                        kind='uint8',
                        authoring_label='Fill/Texture Effect Color Blue',
                    ),
                    FieldSpec(
                        name='unknown_u8_10',
                        kind='uint8',
                        authoring_label='Fill/Texture Effect Color Unknown Byte 11',
                        notes='wbUnused(1)',
                    ),
                    FieldSpec(
                        name='fill_texture_effect_alpha_fade_in_time',
                        kind='float32',
                        authoring_label='Fill/Texture Effect Alpha Fade In Time',
                    ),
                    FieldSpec(
                        name='fill_texture_effect_full_alpha_time',
                        kind='float32',
                        authoring_label='Fill/Texture Effect Full Alpha Time',
                    ),
                    FieldSpec(
                        name='fill_texture_effect_alpha_fade_out_time',
                        kind='float32',
                        authoring_label='Fill/Texture Effect Alpha Fade Out Time',
                    ),
                    FieldSpec(
                        name='fill_texture_effect_persistent_alpha_ratio',
                        kind='float32',
                        authoring_label='Fill/Texture Effect Persistent Alpha Ratio',
                    ),
                    FieldSpec(
                        name='fill_texture_effect_alpha_pulse_amplitude',
                        kind='float32',
                        authoring_label='Fill/Texture Effect Alpha Pulse Amplitude',
                    ),
                    FieldSpec(
                        name='fill_texture_effect_alpha_pulse_frequency',
                        kind='float32',
                        authoring_label='Fill/Texture Effect Alpha Pulse Frequency',
                    ),
                    FieldSpec(
                        name='fill_texture_effect_texture_animation_speed_u',
                        kind='float32',
                        authoring_label='Fill/Texture Effect Texture Animation Speed (U)',
                    ),
                    FieldSpec(
                        name='fill_texture_effect_texture_animation_speed_v',
                        kind='float32',
                        authoring_label='Fill/Texture Effect Texture Animation Speed (V)',
                    ),
                    FieldSpec(
                        name='edge_effect_fall_off',
                        kind='float32',
                        authoring_label='Edge Effect Fall Off',
                    ),
                    FieldSpec(
                        name='edge_effect_color_red',
                        kind='uint8',
                        authoring_label='Edge Effect Color Red',
                    ),
                    FieldSpec(
                        name='edge_effect_color_green',
                        kind='uint8',
                        authoring_label='Edge Effect Color Green',
                    ),
                    FieldSpec(
                        name='edge_effect_color_blue',
                        kind='uint8',
                        authoring_label='Edge Effect Color Blue',
                    ),
                    FieldSpec(
                        name='unknown_u8_23',
                        kind='uint8',
                        authoring_label='Edge Effect Color Unknown Byte 24',
                        notes='wbUnused(1)',
                    ),
                    FieldSpec(
                        name='edge_effect_alpha_fade_in_time',
                        kind='float32',
                        authoring_label='Edge Effect Alpha Fade In Time',
                    ),
                    FieldSpec(
                        name='edge_effect_full_alpha_time',
                        kind='float32',
                        authoring_label='Edge Effect Full Alpha Time',
                    ),
                    FieldSpec(
                        name='edge_effect_alpha_fade_out_time',
                        kind='float32',
                        authoring_label='Edge Effect Alpha Fade Out Time',
                    ),
                    FieldSpec(
                        name='edge_effect_persistent_alpha_ratio',
                        kind='float32',
                        authoring_label='Edge Effect Persistent Alpha Ratio',
                    ),
                    FieldSpec(
                        name='edge_effect_alpha_pulse_amplitude',
                        kind='float32',
                        authoring_label='Edge Effect Alpha Pulse Amplitude',
                    ),
                    FieldSpec(
                        name='edge_effect_alpha_pusle_frequence',
                        kind='float32',
                        authoring_label='Edge Effect Alpha Pusle Frequence',
                    ),
                    FieldSpec(
                        name='fill_texture_effect_full_alpha_ratio',
                        kind='float32',
                        authoring_label='Fill/Texture Effect - Full Alpha Ratio',
                    ),
                    FieldSpec(
                        name='edge_effect_full_alpha_ratio',
                        kind='float32',
                        authoring_label='Edge Effect - Full Alpha Ratio',
                    ),
                    FieldSpec(
                        name='membrane_shader_dest_blend_mode',
                        kind='uint32',
                        enum_ref='blend_mode_enum',
                        authoring_label='Membrane Shader - Dest Blend Mode',
                    ),
                    FieldSpec(
                        name='particle_shader_source_blend_mode',
                        kind='uint32',
                        enum_ref='blend_mode_enum',
                        authoring_label='Particle Shader Source Blend Mode',
                    ),
                    FieldSpec(
                        name='particle_shader_blend_operation',
                        kind='uint32',
                        enum_ref='blend_op_enum',
                        authoring_label='Particle Shader Blend Operation',
                    ),
                    FieldSpec(
                        name='particle_shader_z_test_function',
                        kind='uint32',
                        enum_ref='z_test_func_enum',
                        authoring_label='Particle Shader Z Test Function',
                    ),
                    FieldSpec(
                        name='particle_shader_dest_blend_mode',
                        kind='uint32',
                        enum_ref='blend_mode_enum',
                        authoring_label='Particle Shader Dest Blend Mode',
                    ),
                    FieldSpec(
                        name='particle_shader_particle_birth_ramp_up_time',
                        kind='float32',
                        authoring_label='Particle Shader Particle Birth Ramp Up Time',
                    ),
                    FieldSpec(
                        name='particle_shader_full_particle_birth_time',
                        kind='float32',
                        authoring_label='Particle Shader Full Particle Birth Time',
                    ),
                    FieldSpec(
                        name='particle_shader_particle_birth_ramp_down_time',
                        kind='float32',
                        authoring_label='Particle Shader Particle Birth Ramp Down Time',
                    ),
                    FieldSpec(
                        name='particle_shader_full_particle_birth_ratio',
                        kind='float32',
                        authoring_label='Particle Shader Full Particle Birth Ratio',
                    ),
                    FieldSpec(
                        name='particle_shader_persistant_particle_birth_ratio',
                        kind='float32',
                        authoring_label='Particle Shader Persistant Particle Birth Ratio',
                    ),
                    FieldSpec(
                        name='particle_shader_particle_lifetime',
                        kind='float32',
                        authoring_label='Particle Shader Particle Lifetime',
                    ),
                    FieldSpec(
                        name='particle_shader_particle_lifetime_1',
                        kind='float32',
                        authoring_label='Particle Shader Particle Lifetime +/-',
                    ),
                    FieldSpec(
                        name='particle_shader_initial_speed_along_normal',
                        kind='float32',
                        authoring_label='Particle Shader Initial Speed Along Normal',
                    ),
                    FieldSpec(
                        name='particle_shader_acceleration_along_normal',
                        kind='float32',
                        authoring_label='Particle Shader Acceleration Along Normal',
                    ),
                    FieldSpec(
                        name='particle_shader_initial_velocity_1',
                        kind='float32',
                        authoring_label='Particle Shader Initial Velocity #1',
                    ),
                    FieldSpec(
                        name='particle_shader_initial_velocity_2',
                        kind='float32',
                        authoring_label='Particle Shader Initial Velocity #2',
                    ),
                    FieldSpec(
                        name='particle_shader_initial_velocity_3',
                        kind='float32',
                        authoring_label='Particle Shader Initial Velocity #3',
                    ),
                    FieldSpec(
                        name='particle_shader_acceleration_1',
                        kind='float32',
                        authoring_label='Particle Shader Acceleration #1',
                    ),
                    FieldSpec(
                        name='particle_shader_acceleration_2',
                        kind='float32',
                        authoring_label='Particle Shader Acceleration #2',
                    ),
                    FieldSpec(
                        name='particle_shader_acceleration_3',
                        kind='float32',
                        authoring_label='Particle Shader Acceleration #3',
                    ),
                    FieldSpec(
                        name='particle_shader_scale_key_1',
                        kind='float32',
                        authoring_label='Particle Shader Scale Key 1',
                    ),
                    FieldSpec(
                        name='particle_shader_scale_key_2',
                        kind='float32',
                        authoring_label='Particle Shader Scale Key 2',
                    ),
                    FieldSpec(
                        name='particle_shader_scale_key_1_time',
                        kind='float32',
                        authoring_label='Particle Shader Scale Key 1 Time',
                    ),
                    FieldSpec(
                        name='particle_shader_scale_key_2_time',
                        kind='float32',
                        authoring_label='Particle Shader Scale Key 2 Time',
                    ),
                    FieldSpec(
                        name='color_key_1_color_red',
                        kind='uint8',
                        authoring_label='Color Key 1 - Color Red',
                    ),
                    FieldSpec(
                        name='color_key_1_color_green',
                        kind='uint8',
                        authoring_label='Color Key 1 - Color Green',
                    ),
                    FieldSpec(
                        name='color_key_1_color_blue',
                        kind='uint8',
                        authoring_label='Color Key 1 - Color Blue',
                    ),
                    FieldSpec(
                        name='unknown_u8_59',
                        kind='uint8',
                        authoring_label='Color Key 1 - Color Unknown Byte 60',
                        notes='wbUnused(1)',
                    ),
                    FieldSpec(
                        name='color_key_2_color_red',
                        kind='uint8',
                        authoring_label='Color Key 2 - Color Red',
                    ),
                    FieldSpec(
                        name='color_key_2_color_green',
                        kind='uint8',
                        authoring_label='Color Key 2 - Color Green',
                    ),
                    FieldSpec(
                        name='color_key_2_color_blue',
                        kind='uint8',
                        authoring_label='Color Key 2 - Color Blue',
                    ),
                    FieldSpec(
                        name='unknown_u8_63',
                        kind='uint8',
                        authoring_label='Color Key 2 - Color Unknown Byte 64',
                        notes='wbUnused(1)',
                    ),
                    FieldSpec(
                        name='color_key_3_color_red',
                        kind='uint8',
                        authoring_label='Color Key 3 - Color Red',
                    ),
                    FieldSpec(
                        name='color_key_3_color_green',
                        kind='uint8',
                        authoring_label='Color Key 3 - Color Green',
                    ),
                    FieldSpec(
                        name='color_key_3_color_blue',
                        kind='uint8',
                        authoring_label='Color Key 3 - Color Blue',
                    ),
                    FieldSpec(
                        name='unknown_u8_67',
                        kind='uint8',
                        authoring_label='Color Key 3 - Color Unknown Byte 68',
                        notes='wbUnused(1)',
                    ),
                    FieldSpec(
                        name='color_key_1_color_alpha',
                        kind='float32',
                        authoring_label='Color Key 1 - Color Alpha',
                    ),
                    FieldSpec(
                        name='color_key_2_color_alpha',
                        kind='float32',
                        authoring_label='Color Key 2 - Color Alpha',
                    ),
                    FieldSpec(
                        name='color_key_3_color_alpha',
                        kind='float32',
                        authoring_label='Color Key 3 - Color Alpha',
                    ),
                    FieldSpec(
                        name='color_key_1_color_key_time',
                        kind='float32',
                        authoring_label='Color Key 1 - Color Key Time',
                    ),
                    FieldSpec(
                        name='color_key_2_color_key_time',
                        kind='float32',
                        authoring_label='Color Key 2 - Color Key Time',
                    ),
                    FieldSpec(
                        name='color_key_3_color_key_time',
                        kind='float32',
                        authoring_label='Color Key 3 - Color Key Time',
                    ),
                ),
            ),
        ),
        display_label='Effect Shader',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['ENCH'] = RecordSpec(
        sig='ENCH',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='ENIT',
                kind=FieldKind.PARSED,
                display_label='Data',
                codec='struct:I,I,I,B,B,B,B',
                fields=(
                    FieldSpec(
                        name='type',
                        kind='uint32',
                        enum_ref='ENCH.ENIT.type',
                        authoring_label='Type',
                    ),
                    FieldSpec(
                        name='charge_amount',
                        kind='uint32',
                        authoring_label='Charge Amount',
                    ),
                    FieldSpec(
                        name='enchant_cost',
                        kind='uint32',
                        authoring_label='Enchant Cost',
                    ),
                    FieldSpec(
                        name='no_autocalc_cost',
                        kind='uint8',
                        enum_ref='bool_enum',
                        authoring_label='No Autocalc Cost',
                    ),
                    FieldSpec(
                        name='unknown_u8_4',
                        kind='uint8',
                        authoring_label='Unknown Byte 5',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_5',
                        kind='uint8',
                        authoring_label='Unknown Byte 6',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_6',
                        kind='uint8',
                        authoring_label='Unknown Byte 7',
                        notes='wbUnused(3)',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='EFID',
                kind=FieldKind.PARSED,
                display_label='Magic Effect Name',
                codec='uint32',
                fields=(
                    FieldSpec(
                        name='magic_effect_name',
                        kind='uint32',
                        authoring_label='Magic Effect Name',
                    ),
                ),
                repeatable=True,
                required=True,
                scope_id='effects',
            ),
            SubrecordSpec(
                sig='EFIT',
                kind=FieldKind.PARSED,
                codec='struct:I,I,I,I,I,i',
                fields=(
                    FieldSpec(
                        name='magic_effect_name',
                        kind='uint32',
                        authoring_label='Magic Effect Name',
                    ),
                    FieldSpec(
                        name='magnitude',
                        kind='uint32',
                        authoring_label='Magnitude',
                    ),
                    FieldSpec(
                        name='area',
                        kind='uint32',
                        authoring_label='Area',
                    ),
                    FieldSpec(
                        name='duration',
                        kind='uint32',
                        authoring_label='Duration',
                    ),
                    FieldSpec(
                        name='type',
                        kind='uint32',
                        enum_ref='effect_type_enum',
                        authoring_label='Type',
                    ),
                    FieldSpec(
                        name='actor_value',
                        kind='int32',
                        enum_ref='actor_value_enum',
                        authoring_label='Actor Value',
                    ),
                ),
                repeatable=True,
                required=True,
                scope_id='effects',
            ),
            SubrecordSpec(
                sig='SCIT',
                kind=FieldKind.PARSED_WITH_RAW_FALLBACK,
                codec='struct:I,I,I,B,B,B,B',
                fields=(
                    FieldSpec(
                        name='script_effect',
                        kind='formid',
                        formlink_target='SCPT',
                        formlink_targets=('SCPT',),
                        null_allowed=True,
                        authoring_label='Script effect',
                    ),
                    FieldSpec(
                        name='magic_school',
                        kind='uint32',
                        enum_ref='magic_school_enum',
                        authoring_label='Magic school',
                    ),
                    FieldSpec(
                        name='visual_effect_name',
                        kind='uint32',
                        authoring_label='Visual effect name',
                    ),
                    FieldSpec(
                        name='hostile',
                        kind='uint8',
                        enum_ref='bool_enum',
                        authoring_label='Hostile',
                    ),
                    FieldSpec(
                        name='unknown_u8_4',
                        kind='uint8',
                        authoring_label='Unknown Byte 5',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_5',
                        kind='uint8',
                        authoring_label='Unknown Byte 6',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_6',
                        kind='uint8',
                        authoring_label='Unknown Byte 7',
                        notes='wbUnused(3)',
                    ),
                ),
                repeatable=True,
                required=True,
                scope_id='effects',
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
                repeatable=True,
                required=True,
                scope_id='effects',
            ),
        ),
        display_label='Enchantment',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['EYES'] = RecordSpec(
        sig='EYES',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='ICON',
                kind=FieldKind.PARSED,
                display_label='Texture',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='texture',
                        kind='zstring',
                        authoring_label='Texture',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                display_label='Playable',
                codec='uint8',
                fields=(
                    FieldSpec(
                        name='playable',
                        kind='uint8',
                        enum_ref='bool_enum',
                        authoring_label='Playable',
                    ),
                ),
                required=True,
                enum_ref='bool_enum',
            ),
        ),
        display_label='Eyes',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['FACT'] = RecordSpec(
        sig='FACT',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='XNAM',
                kind=FieldKind.PARSED,
                display_label='Relations',
                codec='struct:I,i',
                fields=(
                    FieldSpec(
                        name='faction',
                        kind='formid',
                        formlink_targets=('FACT', 'RACE'),
                        authoring_label='Faction',
                    ),
                    FieldSpec(
                        name='modifier',
                        kind='int32',
                        authoring_label='Modifier',
                    ),
                ),
                repeatable=True,
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                display_label='Flags',
                codec='uint8',
                fields=(
                    FieldSpec(
                        name='flags',
                        kind='uint8',
                        enum_ref='FACT.DATA.flags',
                        authoring_label='Flags',
                    ),
                ),
                required=True,
                enum_ref='FACT.DATA.flags',
            ),
            SubrecordSpec(
                sig='CNAM',
                kind=FieldKind.PARSED,
                display_label='Crime Gold Multiplier',
                codec='float32',
                fields=(
                    FieldSpec(name='float32_0', kind='float32'),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='RNAM',
                kind=FieldKind.PARSED,
                display_label='Rank#',
                codec='int32',
                fields=(
                    FieldSpec(
                        name='rank',
                        kind='int32',
                        authoring_label='Rank#',
                    ),
                ),
                repeatable=True,
                scope_id='ranks',
            ),
            SubrecordSpec(
                sig='MNAM',
                kind=FieldKind.PARSED,
                display_label='Male',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='male',
                        kind='zstring',
                        authoring_label='Male',
                    ),
                ),
                repeatable=True,
                scope_id='ranks',
            ),
            SubrecordSpec(
                sig='FNAM',
                kind=FieldKind.PARSED,
                display_label='Female',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='female',
                        kind='zstring',
                        authoring_label='Female',
                    ),
                ),
                repeatable=True,
                scope_id='ranks',
            ),
            SubrecordSpec(
                sig='INAM',
                kind=FieldKind.PARSED,
                display_label='Insignia',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='insignia',
                        kind='zstring',
                        authoring_label='Insignia',
                    ),
                ),
                repeatable=True,
                scope_id='ranks',
            ),
        ),
        display_label='Faction',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['FLOR'] = RecordSpec(
        sig='FLOR',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='model_filename',
                        kind='zstring',
                        authoring_label='Model FileName',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='SCRI',
                kind=FieldKind.PARSED,
                display_label='Script',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='script',
                        kind='formid',
                        formlink_target='SCPT',
                        formlink_targets=('SCPT',),
                        authoring_label='Script',
                    ),
                ),
                formlink_target='SCPT',
                formlink_targets=('SCPT',),
            ),
            SubrecordSpec(
                sig='PFIG',
                kind=FieldKind.PARSED,
                display_label='Ingredient',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='ingredient',
                        kind='formid',
                        formlink_target='INGR',
                        formlink_targets=('INGR',),
                        authoring_label='Ingredient',
                    ),
                ),
                formlink_target='INGR',
                formlink_targets=('INGR',),
            ),
            SubrecordSpec(
                sig='PFPC',
                kind=FieldKind.PARSED,
                display_label='Seasonal ingredient production',
                codec='struct:B,B,B,B',
                fields=(
                    FieldSpec(
                        name='spring',
                        kind='uint8',
                        authoring_label='Spring',
                    ),
                    FieldSpec(
                        name='summer',
                        kind='uint8',
                        authoring_label='Summer ',
                    ),
                    FieldSpec(
                        name='fall',
                        kind='uint8',
                        authoring_label='Fall',
                    ),
                    FieldSpec(
                        name='winter',
                        kind='uint8',
                        authoring_label='Winter',
                    ),
                ),
                required=True,
            ),
        ),
        display_label='Flora',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['FURN'] = RecordSpec(
        sig='FURN',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='model_filename',
                        kind='zstring',
                        authoring_label='Model FileName',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='SCRI',
                kind=FieldKind.PARSED,
                display_label='Script',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='script',
                        kind='formid',
                        formlink_target='SCPT',
                        formlink_targets=('SCPT',),
                        authoring_label='Script',
                    ),
                ),
                formlink_target='SCPT',
                formlink_targets=('SCPT',),
            ),
            SubrecordSpec(
                sig='MNAM',
                kind=FieldKind.PARSED,
                display_label='Marker Flags',
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='marker_flags',
                        kind='bytes',
                        authoring_label='Marker Flags',
                    ),
                ),
                required=True,
            ),
        ),
        display_label='Furniture',
        record_flags=RecordFlagsSpec(valid_mask=5152, bits=(RecordFlagBit(bit=10, name='Quest Item'),)),
    )

    records['GLOB'] = RecordSpec(
        sig='GLOB',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FNAM',
                kind=FieldKind.PARSED,
                display_label='Type',
                codec='uint8',
                fields=(
                    FieldSpec(
                        name='type',
                        kind='uint8',
                        authoring_label='Type',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='FLTV',
                kind=FieldKind.PARSED,
                display_label='Value',
                codec='float32',
                fields=(
                    FieldSpec(name='float32_0', kind='float32'),
                ),
                required=True,
            ),
        ),
        display_label='Global',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['GMST'] = RecordSpec(
        sig='GMST',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                display_label='Value',
                required=True,
                union_selector='editor_id_prefix',
                union_variants=(
                    UnionVariantSpec(
                        name='name',
                        codec='zstring',
                        fields=(
                            FieldSpec(
                                name='name_name',
                                kind='zstring',
                                authoring_label='Name Name',
                            ),
                        ),
                        conditions=(
                            ConditionSpec(field='editor_id_prefix', operator='in', values=('s',)),
                        ),
                    ),
                    UnionVariantSpec(
                        name='int',
                        codec='int32',
                        fields=(
                            FieldSpec(
                                name='int_int',
                                kind='int32',
                                authoring_label='Int Int',
                            ),
                        ),
                        conditions=(
                            ConditionSpec(field='editor_id_prefix', operator='not_in', values=('f', 's')),
                        ),
                    ),
                    UnionVariantSpec(
                        name='float',
                        codec='float32',
                        fields=(
                            FieldSpec(
                                name='float_float',
                                kind='float32',
                                authoring_label='Float Float',
                            ),
                        ),
                        conditions=(
                            ConditionSpec(field='editor_id_prefix', operator='in', values=('f',)),
                        ),
                    ),
                ),
            ),
        ),
        display_label='Game Setting',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['GRAS'] = RecordSpec(
        sig='GRAS',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='model_filename',
                        kind='zstring',
                        authoring_label='Model FileName',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                display_label='Data',
                codec='struct:B,B,B,B,H,B,B,I,f,f,f,f,B,B,B,B',
                fields=(
                    FieldSpec(
                        name='density',
                        kind='uint8',
                        authoring_label='Density',
                    ),
                    FieldSpec(
                        name='min_slope',
                        kind='uint8',
                        authoring_label='Min Slope',
                    ),
                    FieldSpec(
                        name='max_slope',
                        kind='uint8',
                        authoring_label='Max Slope',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Unknown Byte 4',
                        notes='wbUnused(1)',
                    ),
                    FieldSpec(
                        name='unit_from_water_amount',
                        kind='uint16',
                        authoring_label='Unit from water amount',
                    ),
                    FieldSpec(
                        name='unknown_u8_5',
                        kind='uint8',
                        authoring_label='Unknown Byte 6',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='unknown_u8_6',
                        kind='uint8',
                        authoring_label='Unknown Byte 7',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='unit_from_water_type',
                        kind='uint32',
                        enum_ref='GRAS.DATA.unit_from_water_type',
                        authoring_label='Unit from water type',
                    ),
                    FieldSpec(
                        name='position_range',
                        kind='float32',
                        authoring_label='Position Range',
                    ),
                    FieldSpec(
                        name='height_range',
                        kind='float32',
                        authoring_label='Height Range',
                    ),
                    FieldSpec(
                        name='color_range',
                        kind='float32',
                        authoring_label='Color Range',
                    ),
                    FieldSpec(
                        name='wave_period',
                        kind='float32',
                        authoring_label='Wave Period',
                    ),
                    FieldSpec(
                        name='flags',
                        kind='uint8',
                        enum_ref='GRAS.DATA.flags',
                        authoring_label='Flags',
                    ),
                    FieldSpec(
                        name='unknown_u8_13',
                        kind='uint8',
                        authoring_label='Unknown Byte 14',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_14',
                        kind='uint8',
                        authoring_label='Unknown Byte 15',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_15',
                        kind='uint8',
                        authoring_label='Unknown Byte 16',
                        notes='wbUnused(3)',
                    ),
                ),
                required=True,
            ),
        ),
        display_label='Grass',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['HAIR'] = RecordSpec(
        sig='HAIR',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='model_filename',
                        kind='zstring',
                        authoring_label='Model FileName',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='ICON',
                kind=FieldKind.PARSED,
                display_label='Texture',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='texture',
                        kind='zstring',
                        authoring_label='Texture',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                display_label='Flags',
                codec='uint8',
                fields=(
                    FieldSpec(
                        name='flags',
                        kind='uint8',
                        enum_ref='HAIR.DATA.flags',
                        authoring_label='Flags',
                    ),
                ),
                required=True,
                enum_ref='HAIR.DATA.flags',
            ),
        ),
        display_label='Hair',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['IDLE'] = RecordSpec(
        sig='IDLE',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='model_filename',
                        kind='zstring',
                        authoring_label='Model FileName',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='CTDA',
                kind=FieldKind.RAW,
                repeatable=True,
                scope_id='conditions',
            ),
            SubrecordSpec(
                sig='CTDT',
                kind=FieldKind.RAW,
                repeatable=True,
                scope_id='conditions',
            ),
            SubrecordSpec(
                sig='ANAM',
                kind=FieldKind.PARSED,
                display_label='Animation Group Section',
                codec='uint8',
                fields=(
                    FieldSpec(
                        name='animation_group_section',
                        kind='uint8',
                        authoring_label='Animation Group Section',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                display_label='Animations',
                codec='struct:I,I',
                fields=(
                    FieldSpec(
                        name='parent',
                        kind='formid',
                        formlink_target='IDLE',
                        formlink_targets=('IDLE',),
                        null_allowed=True,
                        authoring_label='Parent',
                    ),
                    FieldSpec(
                        name='previous',
                        kind='formid',
                        formlink_target='IDLE',
                        formlink_targets=('IDLE',),
                        null_allowed=True,
                        authoring_label='Previous',
                    ),
                ),
                required=True,
            ),
        ),
        display_label='Idle Animation',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['INFO'] = RecordSpec(
        sig='INFO',
        subrecords=(
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED_WITH_RAW_FALLBACK,
                display_label='Data',
                codec='struct:B,B,B',
                fields=(
                    FieldSpec(
                        name='type',
                        kind='uint8',
                        enum_ref='dialogue_type_enum',
                        authoring_label='Type',
                    ),
                    FieldSpec(
                        name='next_speaker',
                        kind='uint8',
                        enum_ref='INFO.DATA.next_speaker',
                        authoring_label='Next Speaker',
                    ),
                    FieldSpec(
                        name='flags',
                        kind='uint8',
                        enum_ref='INFO.DATA.flags',
                        authoring_label='Flags',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='QSTI',
                kind=FieldKind.PARSED,
                display_label='Quest',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='quest',
                        kind='formid',
                        formlink_target='QUST',
                        formlink_targets=('QUST',),
                        authoring_label='Quest',
                    ),
                ),
                required=True,
                formlink_target='QUST',
                formlink_targets=('QUST',),
            ),
            SubrecordSpec(
                sig='TPIC',
                kind=FieldKind.PARSED,
                display_label='Previous Topic',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='previous_topic',
                        kind='formid',
                        formlink_target='DIAL',
                        formlink_targets=('DIAL',),
                        authoring_label='Previous Topic',
                    ),
                ),
                formlink_target='DIAL',
                formlink_targets=('DIAL',),
            ),
            SubrecordSpec(
                sig='PNAM',
                kind=FieldKind.PARSED,
                display_label='Previous Info',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='previous_info',
                        kind='formid',
                        formlink_target='INFO',
                        formlink_targets=('INFO',),
                        null_allowed=True,
                        authoring_label='Previous Info',
                    ),
                ),
                formlink_target='INFO',
                formlink_targets=('INFO',),
                null_allowed=True,
            ),
            SubrecordSpec(
                sig='NAME',
                kind=FieldKind.PARSED,
                display_label='Topic',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='topic',
                        kind='formid',
                        formlink_target='DIAL',
                        formlink_targets=('DIAL',),
                        authoring_label='Topic',
                    ),
                ),
                repeatable=True,
                formlink_target='DIAL',
                formlink_targets=('DIAL',),
            ),
            SubrecordSpec(
                sig='TRDT',
                kind=FieldKind.PARSED,
                display_label='Response Data',
                codec='struct:I,i,B,B,B,B,B,B,B,B',
                fields=(
                    FieldSpec(
                        name='emotion_type',
                        kind='uint32',
                        enum_ref='INFO.TRDT.emotion_type',
                        authoring_label='Emotion Type',
                    ),
                    FieldSpec(
                        name='emotion_value',
                        kind='int32',
                        authoring_label='Emotion Value',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Unknown Byte 3',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Unknown Byte 4',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='unknown_u8_4',
                        kind='uint8',
                        authoring_label='Unknown Byte 5',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='unknown_u8_5',
                        kind='uint8',
                        authoring_label='Unknown Byte 6',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='response_number',
                        kind='uint8',
                        authoring_label='Response Number',
                    ),
                    FieldSpec(
                        name='unknown_u8_7',
                        kind='uint8',
                        authoring_label='Unknown Byte 8',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_8',
                        kind='uint8',
                        authoring_label='Unknown Byte 9',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_9',
                        kind='uint8',
                        authoring_label='Unknown Byte 10',
                        notes='wbUnused(3)',
                    ),
                ),
                repeatable=True,
                required=True,
                scope_id='responses',
            ),
            SubrecordSpec(
                sig='NAM1',
                kind=FieldKind.PARSED,
                display_label='Response Text',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='response_text',
                        kind='zstring',
                        authoring_label='Response Text',
                    ),
                ),
                repeatable=True,
                required=True,
                scope_id='responses',
            ),
            SubrecordSpec(
                sig='NAM2',
                kind=FieldKind.PARSED,
                display_label='Actor Notes',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='actor_notes',
                        kind='zstring',
                        authoring_label='Actor Notes',
                    ),
                ),
                repeatable=True,
                required=True,
                scope_id='responses',
            ),
            SubrecordSpec(
                sig='CTDA',
                kind=FieldKind.RAW,
                repeatable=True,
                scope_id='conditions',
            ),
            SubrecordSpec(
                sig='CTDT',
                kind=FieldKind.RAW,
                repeatable=True,
                scope_id='conditions',
            ),
            SubrecordSpec(
                sig='TCLT',
                kind=FieldKind.PARSED,
                display_label='Choice',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='choice',
                        kind='formid',
                        formlink_target='DIAL',
                        formlink_targets=('DIAL',),
                        authoring_label='Choice',
                    ),
                ),
                repeatable=True,
                formlink_target='DIAL',
                formlink_targets=('DIAL',),
            ),
            SubrecordSpec(
                sig='TCLF',
                kind=FieldKind.PARSED,
                display_label='Topic',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='topic',
                        kind='formid',
                        formlink_target='DIAL',
                        formlink_targets=('DIAL',),
                        authoring_label='Topic',
                    ),
                ),
                repeatable=True,
                formlink_target='DIAL',
                formlink_targets=('DIAL',),
            ),
            SubrecordSpec(
                sig='SCHR',
                kind=FieldKind.PARSED,
                display_label='Basic Script Data',
                codec='struct:B,B,B,B,I,I,I,I',
                fields=(
                    FieldSpec(
                        name='unknown_u8_0',
                        kind='uint8',
                        authoring_label='Unknown Byte 1',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='unknown_u8_1',
                        kind='uint8',
                        authoring_label='Unknown Byte 2',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Unknown Byte 3',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Unknown Byte 4',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='refcount',
                        kind='uint32',
                        authoring_label='RefCount',
                    ),
                    FieldSpec(
                        name='compiledsize',
                        kind='uint32',
                        authoring_label='CompiledSize',
                    ),
                    FieldSpec(
                        name='variablecount',
                        kind='uint32',
                        authoring_label='VariableCount',
                    ),
                    FieldSpec(
                        name='type',
                        kind='uint32',
                        enum_ref='INFO.SCHR.type',
                        authoring_label='Type',
                    ),
                ),
                repeatable=True,
                authoring_layout='row_group',
                authoring_key='group_result_script',
                scope_id='result_script',
            ),
            SubrecordSpec(
                sig='SCHD',
                kind=FieldKind.PARSED,
                display_label='Basic Script Data',
                codec='struct:B,B,B,B,I,I,I,I',
                fields=(
                    FieldSpec(
                        name='unknown_u8_0',
                        kind='uint8',
                        authoring_label='Unknown Byte 1',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='unknown_u8_1',
                        kind='uint8',
                        authoring_label='Unknown Byte 2',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Unknown Byte 3',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Unknown Byte 4',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='refcount',
                        kind='uint32',
                        authoring_label='RefCount',
                    ),
                    FieldSpec(
                        name='compiledsize',
                        kind='uint32',
                        authoring_label='CompiledSize',
                    ),
                    FieldSpec(
                        name='variablecount',
                        kind='uint32',
                        authoring_label='VariableCount',
                    ),
                    FieldSpec(
                        name='type',
                        kind='uint32',
                        enum_ref='INFO.SCHD.type',
                        authoring_label='Type',
                    ),
                    FieldSpec(
                        name='unknown',
                        kind='bytes',
                        authoring_label='Unknown',
                    ),
                ),
                repeatable=True,
                authoring_layout='row_group',
                authoring_key='group_result_script',
                scope_id='result_script',
            ),
            SubrecordSpec(
                sig='SCDA',
                kind=FieldKind.PARSED,
                display_label='Compiled result script',
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='compiled_result_script',
                        kind='bytes',
                        authoring_label='Compiled result script',
                    ),
                ),
                repeatable=True,
                authoring_layout='row_group',
                authoring_key='group_result_script',
                scope_id='result_script',
            ),
            SubrecordSpec(
                sig='SCTX',
                kind=FieldKind.RAW,
                display_label='Result script source',
                repeatable=True,
                authoring_layout='row_group',
                authoring_key='group_result_script',
                scope_id='result_script',
            ),
            SubrecordSpec(
                sig='SCRO',
                kind=FieldKind.PARSED,
                display_label='Global Reference',
                codec='formid',
                fields=(
                    FieldSpec(name='formid_0', kind='formid'),
                ),
                repeatable=True,
                authoring_layout='row_group',
                authoring_key='group_result_script',
                scope_id='result_script',
            ),
            SubrecordSpec(
                sig='SCRV',
                kind=FieldKind.PARSED,
                display_label='Local Variable',
                codec='uint32',
                fields=(
                    FieldSpec(
                        name='local_variable',
                        kind='uint32',
                        authoring_label='Local Variable',
                    ),
                ),
                repeatable=True,
                authoring_layout='row_group',
                authoring_key='group_result_script',
                scope_id='result_script',
            ),
        ),
        display_label='Dialog response',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['INGR'] = RecordSpec(
        sig='INGR',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='model_filename',
                        kind='zstring',
                        authoring_label='Model FileName',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='ICON',
                kind=FieldKind.PARSED,
                display_label='Icon FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='icon_filename',
                        kind='zstring',
                        authoring_label='Icon FileName',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='SCRI',
                kind=FieldKind.PARSED,
                display_label='Script',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='script',
                        kind='formid',
                        formlink_target='SCPT',
                        formlink_targets=('SCPT',),
                        authoring_label='Script',
                    ),
                ),
                formlink_target='SCPT',
                formlink_targets=('SCPT',),
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                display_label='Weight',
                codec='float32',
                fields=(
                    FieldSpec(name='float32_0', kind='float32'),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='ENIT',
                kind=FieldKind.PARSED,
                display_label='Data',
                codec='struct:i,B,B,B,B',
                fields=(
                    FieldSpec(
                        name='value',
                        kind='int32',
                        authoring_label='Value',
                    ),
                    FieldSpec(
                        name='flags',
                        kind='uint8',
                        enum_ref='INGR.ENIT.flags',
                        authoring_label='Flags',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Unknown Byte 3',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Unknown Byte 4',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_4',
                        kind='uint8',
                        authoring_label='Unknown Byte 5',
                        notes='wbUnused(3)',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='EFID',
                kind=FieldKind.PARSED,
                display_label='Magic Effect Name',
                codec='uint32',
                fields=(
                    FieldSpec(
                        name='magic_effect_name',
                        kind='uint32',
                        authoring_label='Magic Effect Name',
                    ),
                ),
                repeatable=True,
                required=True,
                scope_id='effects',
            ),
            SubrecordSpec(
                sig='EFIT',
                kind=FieldKind.PARSED,
                codec='struct:I,I,I,I,I,i',
                fields=(
                    FieldSpec(
                        name='magic_effect_name',
                        kind='uint32',
                        authoring_label='Magic Effect Name',
                    ),
                    FieldSpec(
                        name='magnitude',
                        kind='uint32',
                        authoring_label='Magnitude',
                    ),
                    FieldSpec(
                        name='area',
                        kind='uint32',
                        authoring_label='Area',
                    ),
                    FieldSpec(
                        name='duration',
                        kind='uint32',
                        authoring_label='Duration',
                    ),
                    FieldSpec(
                        name='type',
                        kind='uint32',
                        enum_ref='effect_type_enum',
                        authoring_label='Type',
                    ),
                    FieldSpec(
                        name='actor_value',
                        kind='int32',
                        enum_ref='actor_value_enum',
                        authoring_label='Actor Value',
                    ),
                ),
                repeatable=True,
                required=True,
                scope_id='effects',
            ),
            SubrecordSpec(
                sig='SCIT',
                kind=FieldKind.PARSED_WITH_RAW_FALLBACK,
                codec='struct:I,I,I,B,B,B,B',
                fields=(
                    FieldSpec(
                        name='script_effect',
                        kind='formid',
                        formlink_target='SCPT',
                        formlink_targets=('SCPT',),
                        null_allowed=True,
                        authoring_label='Script effect',
                    ),
                    FieldSpec(
                        name='magic_school',
                        kind='uint32',
                        enum_ref='magic_school_enum',
                        authoring_label='Magic school',
                    ),
                    FieldSpec(
                        name='visual_effect_name',
                        kind='uint32',
                        authoring_label='Visual effect name',
                    ),
                    FieldSpec(
                        name='hostile',
                        kind='uint8',
                        enum_ref='bool_enum',
                        authoring_label='Hostile',
                    ),
                    FieldSpec(
                        name='unknown_u8_4',
                        kind='uint8',
                        authoring_label='Unknown Byte 5',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_5',
                        kind='uint8',
                        authoring_label='Unknown Byte 6',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_6',
                        kind='uint8',
                        authoring_label='Unknown Byte 7',
                        notes='wbUnused(3)',
                    ),
                ),
                repeatable=True,
                required=True,
                scope_id='effects',
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
                repeatable=True,
                required=True,
                scope_id='effects',
            ),
        ),
        display_label='Ingredient',
        record_flags=RecordFlagsSpec(valid_mask=5152, bits=(RecordFlagBit(bit=10, name='Quest Item'),)),
    )

    records['KEYM'] = RecordSpec(
        sig='KEYM',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='model_filename',
                        kind='zstring',
                        authoring_label='Model FileName',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='ICON',
                kind=FieldKind.PARSED,
                display_label='Icon FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='icon_filename',
                        kind='zstring',
                        authoring_label='Icon FileName',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='SCRI',
                kind=FieldKind.PARSED,
                display_label='Script',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='script',
                        kind='formid',
                        formlink_target='SCPT',
                        formlink_targets=('SCPT',),
                        authoring_label='Script',
                    ),
                ),
                formlink_target='SCPT',
                formlink_targets=('SCPT',),
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                display_label='Data',
                codec='struct:i,f',
                fields=(
                    FieldSpec(
                        name='value',
                        kind='int32',
                        authoring_label='Value',
                    ),
                    FieldSpec(
                        name='weight',
                        kind='float32',
                        authoring_label='Weight',
                    ),
                ),
                required=True,
            ),
        ),
        display_label='Key',
        record_flags=RecordFlagsSpec(valid_mask=5152, bits=(RecordFlagBit(bit=10, name='Quest Item'),)),
    )

    records['LAND'] = RecordSpec(
        sig='LAND',
        subrecords=(
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                display_label='Flags',
                codec='uint32',
                fields=(
                    FieldSpec(
                        name='flags',
                        kind='uint32',
                        enum_ref='LAND.DATA.flags',
                        authoring_label='Flags',
                    ),
                ),
                enum_ref='LAND.DATA.flags',
            ),
            SubrecordSpec(sig='VNML', kind=FieldKind.CUSTOM_CODEC, display_label='Vertex Normals', codec='esp_authoring_core::land::heightmap'),
            SubrecordSpec(sig='VHGT', kind=FieldKind.CUSTOM_CODEC, display_label='Vertex Height Map', codec='esp_authoring_core::land::heightmap'),
            SubrecordSpec(
                sig='VCLR',
                kind=FieldKind.PARSED,
                display_label='Vertex Colors',
                codec='array_struct:',
                fields=(
                    FieldSpec(
                        name='row',
                        kind='struct',
                        nested_fields=(
                            FieldSpec(
                                name='red',
                                kind='uint8',
                                authoring_label='Red',
                            ),
                            FieldSpec(
                                name='green',
                                kind='uint8',
                                authoring_label='Green',
                            ),
                            FieldSpec(
                                name='blue',
                                kind='uint8',
                                authoring_label='Blue',
                            ),
                        ),
                        array=ArraySpec(layout='row_array', element_codec='B,B,B'),
                        authoring_label='Row',
                    ),
                ),
                array=ArraySpec(layout='row_array'),
                row_label='Vertex Colors',
            ),
            SubrecordSpec(
                sig='BTXT',
                kind=FieldKind.PARSED,
                codec='struct:I,B,B,h',
                fields=(
                    FieldSpec(
                        name='texture',
                        kind='formid',
                        formlink_target='LTEX',
                        formlink_targets=('LTEX',),
                        null_allowed=True,
                        authoring_label='Texture',
                    ),
                    FieldSpec(
                        name='quadrant',
                        kind='uint8',
                        enum_ref='quadrant_enum',
                        authoring_label='Quadrant',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Unknown Byte 3',
                        notes='wbUnused(1)',
                    ),
                    FieldSpec(
                        name='layer',
                        kind='int16',
                        authoring_label='Layer',
                    ),
                ),
                repeatable=True,
                scope_id='layers',
            ),
            SubrecordSpec(
                sig='ATXT',
                kind=FieldKind.PARSED,
                codec='struct:I,B,B,h',
                fields=(
                    FieldSpec(
                        name='texture',
                        kind='formid',
                        formlink_target='LTEX',
                        formlink_targets=('LTEX',),
                        null_allowed=True,
                        authoring_label='Texture',
                    ),
                    FieldSpec(
                        name='quadrant',
                        kind='uint8',
                        enum_ref='quadrant_enum',
                        authoring_label='Quadrant',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Unknown Byte 3',
                        notes='wbUnused(1)',
                    ),
                    FieldSpec(
                        name='layer',
                        kind='int16',
                        authoring_label='Layer',
                    ),
                ),
                repeatable=True,
                scope_id='layers',
            ),
            SubrecordSpec(
                sig='VTXT',
                kind=FieldKind.PARSED,
                display_label='Alpha Layer Data',
                codec='array_struct:H,B,B,f',
                fields=(
                    FieldSpec(
                        name='alpha_layer_data_position',
                        kind='uint16',
                        authoring_label='Alpha Layer Data Position',
                    ),
                    FieldSpec(
                        name='unknown_u8_1',
                        kind='uint8',
                        authoring_label='Alpha Layer Data Unknown Byte 2',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Alpha Layer Data Unknown Byte 3',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='alpha_layer_data_opacity',
                        kind='float32',
                        authoring_label='Alpha Layer Data Opacity',
                    ),
                ),
                repeatable=True,
                array=ArraySpec(layout='row_array', element_codec='H,B,B,f'),
                row_label='Alpha Layer Data',
                scope_id='layers',
            ),
            SubrecordSpec(
                sig='VTEX',
                kind=FieldKind.PARSED,
                display_label='Landscape Textures',
                codec='formid_array',
                fields=(
                    FieldSpec(
                        name='landscape_textures_texture',
                        kind='formid',
                        formlink_target='LTEX',
                        formlink_targets=('LTEX',),
                        null_allowed=True,
                        authoring_label='Landscape Textures Texture',
                    ),
                ),
                formlink_target='LTEX',
                formlink_targets=('LTEX',),
                null_allowed=True,
            ),
        ),
        display_label='Landscape',
        record_flags=RecordFlagsSpec(valid_mask=266272, bits=(RecordFlagBit(bit=18, name='Compressed'),)),
    )

    records['LIGH'] = RecordSpec(
        sig='LIGH',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='model_filename',
                        kind='zstring',
                        authoring_label='Model FileName',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='SCRI',
                kind=FieldKind.PARSED,
                display_label='Script',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='script',
                        kind='formid',
                        formlink_target='SCPT',
                        formlink_targets=('SCPT',),
                        authoring_label='Script',
                    ),
                ),
                formlink_target='SCPT',
                formlink_targets=('SCPT',),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='ICON',
                kind=FieldKind.PARSED,
                display_label='Icon FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='icon_filename',
                        kind='zstring',
                        authoring_label='Icon FileName',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED_WITH_RAW_FALLBACK,
                display_label='Data',
                codec='struct:i,I,B,B,B,B,I,f,f,I,f',
                fields=(
                    FieldSpec(
                        name='time',
                        kind='int32',
                        authoring_label='Time',
                    ),
                    FieldSpec(
                        name='radius',
                        kind='uint32',
                        authoring_label='Radius',
                    ),
                    FieldSpec(
                        name='color_red',
                        kind='uint8',
                        authoring_label='Color Red',
                    ),
                    FieldSpec(
                        name='color_green',
                        kind='uint8',
                        authoring_label='Color Green',
                    ),
                    FieldSpec(
                        name='color_blue',
                        kind='uint8',
                        authoring_label='Color Blue',
                    ),
                    FieldSpec(
                        name='unknown_u8_5',
                        kind='uint8',
                        authoring_label='Color Unknown Byte 6',
                        notes='wbUnused(1)',
                    ),
                    FieldSpec(
                        name='flags',
                        kind='uint32',
                        enum_ref='LIGH.DATA.flags',
                        authoring_label='Flags',
                    ),
                    FieldSpec(
                        name='falloff_exponent',
                        kind='float32',
                        authoring_label='Falloff Exponent',
                    ),
                    FieldSpec(
                        name='fov',
                        kind='float32',
                        authoring_label='FOV',
                    ),
                    FieldSpec(
                        name='value',
                        kind='uint32',
                        authoring_label='Value',
                    ),
                    FieldSpec(
                        name='weight',
                        kind='float32',
                        authoring_label='Weight',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FNAM',
                kind=FieldKind.PARSED,
                display_label='Fade value',
                codec='float32',
                fields=(
                    FieldSpec(name='float32_0', kind='float32'),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='SNAM',
                kind=FieldKind.PARSED,
                display_label='Sound',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='sound',
                        kind='formid',
                        formlink_target='SOUN',
                        formlink_targets=('SOUN',),
                        authoring_label='Sound',
                    ),
                ),
                formlink_target='SOUN',
                formlink_targets=('SOUN',),
            ),
        ),
        display_label='Light',
        record_flags=RecordFlagsSpec(valid_mask=5152, bits=(RecordFlagBit(bit=10, name='Quest'),)),
    )

    records['LSCR'] = RecordSpec(
        sig='LSCR',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='ICON',
                kind=FieldKind.PARSED,
                display_label='Icon FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='icon_filename',
                        kind='zstring',
                        authoring_label='Icon FileName',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='DESC',
                kind=FieldKind.PARSED,
                display_label='Description',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='description',
                        kind='zstring',
                        authoring_label='Description',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='LNAM',
                kind=FieldKind.PARSED,
                display_label='Locations',
                codec='struct:I,I,h,h',
                fields=(
                    FieldSpec(
                        name='direct',
                        kind='formid',
                        formlink_targets=('CELL', 'WRLD'),
                        null_allowed=True,
                        authoring_label='Direct',
                    ),
                    FieldSpec(
                        name='indirect_world',
                        kind='formid',
                        formlink_target='WRLD',
                        formlink_targets=('WRLD',),
                        null_allowed=True,
                        authoring_label='Indirect World',
                    ),
                    FieldSpec(
                        name='indirect_grid_y',
                        kind='int16',
                        authoring_label='Indirect Grid Y',
                    ),
                    FieldSpec(
                        name='indirect_grid_x',
                        kind='int16',
                        authoring_label='Indirect Grid X',
                    ),
                ),
                repeatable=True,
            ),
        ),
        display_label='Load Screen',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['LTEX'] = RecordSpec(
        sig='LTEX',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='ICON',
                kind=FieldKind.PARSED,
                display_label='Icon FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='icon_filename',
                        kind='zstring',
                        authoring_label='Icon FileName',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='HNAM',
                kind=FieldKind.PARSED,
                display_label='Havok Data',
                codec='struct:B,B,B',
                fields=(
                    FieldSpec(
                        name='material_type',
                        kind='uint8',
                        enum_ref='LTEX.HNAM.material_type',
                        authoring_label='Material Type',
                    ),
                    FieldSpec(
                        name='friction',
                        kind='uint8',
                        authoring_label='Friction',
                    ),
                    FieldSpec(
                        name='restitution',
                        kind='uint8',
                        authoring_label='Restitution',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='SNAM',
                kind=FieldKind.PARSED,
                display_label='Texture Specular Exponent',
                codec='uint8',
                fields=(
                    FieldSpec(
                        name='texture_specular_exponent',
                        kind='uint8',
                        authoring_label='Texture Specular Exponent',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='GNAM',
                kind=FieldKind.PARSED,
                display_label='Grass',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='grass',
                        kind='formid',
                        formlink_target='GRAS',
                        formlink_targets=('GRAS',),
                        authoring_label='Grass',
                    ),
                ),
                repeatable=True,
                formlink_target='GRAS',
                formlink_targets=('GRAS',),
            ),
        ),
        display_label='Landscape Texture',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['LVLC'] = RecordSpec(
        sig='LVLC',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='LVLD',
                kind=FieldKind.PARSED,
                display_label='Chance none',
                codec='uint8',
                fields=(
                    FieldSpec(
                        name='chance_none',
                        kind='uint8',
                        authoring_label='Chance none',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='LVLF',
                kind=FieldKind.PARSED,
                display_label='Flags',
                codec='uint8',
                fields=(
                    FieldSpec(
                        name='flags',
                        kind='uint8',
                        enum_ref='LVLC.LVLF.flags',
                        authoring_label='Flags',
                    ),
                ),
                required=True,
                enum_ref='LVLC.LVLF.flags',
            ),
            SubrecordSpec(
                sig='LVLO',
                kind=FieldKind.PARSED_WITH_RAW_FALLBACK,
                codec='struct:H,B,B,I,H,B,B',
                fields=(
                    FieldSpec(
                        name='level',
                        kind='uint16',
                        authoring_label='Level',
                    ),
                    FieldSpec(
                        name='unknown_u8_1',
                        kind='uint8',
                        authoring_label='Unknown Byte 2',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Unknown Byte 3',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='creature',
                        kind='formid',
                        formlink_targets=('CREA', 'LVLC', 'NPC_'),
                        authoring_label='Creature',
                    ),
                    FieldSpec(
                        name='count',
                        kind='uint16',
                        authoring_label='Count',
                    ),
                    FieldSpec(
                        name='unknown_u8_5',
                        kind='uint8',
                        authoring_label='Unknown Byte 6',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='unknown_u8_6',
                        kind='uint8',
                        authoring_label='Unknown Byte 7',
                        notes='wbUnused(2)',
                    ),
                ),
                repeatable=True,
                scope_id='leveled_list_entries',
            ),
            SubrecordSpec(
                sig='SCRI',
                kind=FieldKind.PARSED,
                display_label='Script',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='script',
                        kind='formid',
                        formlink_target='SCPT',
                        formlink_targets=('SCPT',),
                        authoring_label='Script',
                    ),
                ),
                formlink_target='SCPT',
                formlink_targets=('SCPT',),
            ),
            SubrecordSpec(
                sig='TNAM',
                kind=FieldKind.PARSED,
                display_label='Creature template',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='creature_template',
                        kind='formid',
                        formlink_targets=('CREA', 'NPC_'),
                        authoring_label='Creature template',
                    ),
                ),
                formlink_targets=('CREA', 'NPC_'),
            ),
        ),
        display_label='Leveled Creature',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['LVLI'] = RecordSpec(
        sig='LVLI',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='LVLD',
                kind=FieldKind.PARSED,
                display_label='Chance none',
                codec='uint8',
                fields=(
                    FieldSpec(
                        name='chance_none',
                        kind='uint8',
                        authoring_label='Chance none',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='LVLF',
                kind=FieldKind.PARSED,
                display_label='Flags',
                codec='uint8',
                fields=(
                    FieldSpec(
                        name='flags',
                        kind='uint8',
                        enum_ref='LVLI.LVLF.flags',
                        authoring_label='Flags',
                    ),
                ),
                required=True,
                enum_ref='LVLI.LVLF.flags',
            ),
            SubrecordSpec(
                sig='LVLO',
                kind=FieldKind.PARSED_WITH_RAW_FALLBACK,
                codec='struct:H,B,B,I,H,B,B',
                fields=(
                    FieldSpec(
                        name='level',
                        kind='uint16',
                        authoring_label='Level',
                    ),
                    FieldSpec(
                        name='unknown_u8_1',
                        kind='uint8',
                        authoring_label='Unknown Byte 2',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Unknown Byte 3',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='item',
                        kind='formid',
                        formlink_targets=('ALCH', 'AMMO', 'APPA', 'ARMO', 'BOOK', 'CLOT', 'INGR', 'KEYM', 'LIGH', 'LVLI', 'MISC', 'SGST', 'SLGM', 'WEAP'),
                        authoring_label='Item',
                    ),
                    FieldSpec(
                        name='count',
                        kind='uint16',
                        authoring_label='Count',
                    ),
                    FieldSpec(
                        name='unknown_u8_5',
                        kind='uint8',
                        authoring_label='Unknown Byte 6',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='unknown_u8_6',
                        kind='uint8',
                        authoring_label='Unknown Byte 7',
                        notes='wbUnused(2)',
                    ),
                ),
                repeatable=True,
                scope_id='leveled_list_entries',
            ),
            SubrecordSpec(sig='DATA', kind=FieldKind.RAW),
        ),
        display_label='Leveled Item',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['LVSP'] = RecordSpec(
        sig='LVSP',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='LVLD',
                kind=FieldKind.PARSED,
                display_label='Chance none',
                codec='uint8',
                fields=(
                    FieldSpec(
                        name='chance_none',
                        kind='uint8',
                        authoring_label='Chance none',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='LVLF',
                kind=FieldKind.PARSED,
                display_label='Flags',
                codec='uint8',
                fields=(
                    FieldSpec(
                        name='flags',
                        kind='uint8',
                        enum_ref='LVSP.LVLF.flags',
                        authoring_label='Flags',
                    ),
                ),
                required=True,
                enum_ref='LVSP.LVLF.flags',
            ),
            SubrecordSpec(
                sig='LVLO',
                kind=FieldKind.PARSED_WITH_RAW_FALLBACK,
                codec='struct:H,B,B,I,H,B,B',
                fields=(
                    FieldSpec(
                        name='level',
                        kind='uint16',
                        authoring_label='Level',
                    ),
                    FieldSpec(
                        name='unknown_u8_1',
                        kind='uint8',
                        authoring_label='Unknown Byte 2',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Unknown Byte 3',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='spell',
                        kind='formid',
                        formlink_targets=('LVSP', 'SPEL'),
                        authoring_label='Spell',
                    ),
                    FieldSpec(
                        name='count',
                        kind='uint16',
                        authoring_label='Count',
                    ),
                    FieldSpec(
                        name='unknown_u8_5',
                        kind='uint8',
                        authoring_label='Unknown Byte 6',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='unknown_u8_6',
                        kind='uint8',
                        authoring_label='Unknown Byte 7',
                        notes='wbUnused(2)',
                    ),
                ),
                repeatable=True,
                scope_id='leveled_list_entries',
            ),
        ),
        display_label='Leveled Spell',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['MGEF'] = RecordSpec(
        sig='MGEF',
        subrecords=(
            SubrecordSpec(sig='EDID', kind=FieldKind.RAW, display_label='Magic Effect Code', required=True),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='DESC',
                kind=FieldKind.PARSED,
                display_label='Description',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='description',
                        kind='zstring',
                        authoring_label='Description',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='ICON',
                kind=FieldKind.PARSED,
                display_label='Icon FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='icon_filename',
                        kind='zstring',
                        authoring_label='Icon FileName',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='model_filename',
                        kind='zstring',
                        authoring_label='Model FileName',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED_WITH_RAW_FALLBACK,
                display_label='Data',
                codec='struct:I,f,I,I,i,H,B,B,I,f,I,I,I,I,I,I,f,f',
                fields=(
                    FieldSpec(
                        name='flags',
                        kind='uint32',
                        enum_ref='MGEF.DATA.flags',
                        authoring_label='Flags',
                    ),
                    FieldSpec(
                        name='base_cost',
                        kind='float32',
                        authoring_label='Base cost',
                    ),
                    FieldSpec(
                        name='assoc_item',
                        kind='uint32',
                        authoring_label='Assoc. Item',
                        notes='width-preserving union fallback for wbMGEFFAssocItemDecider',
                    ),
                    FieldSpec(
                        name='magic_school',
                        kind='uint32',
                        enum_ref='magic_school_enum',
                        authoring_label='Magic School',
                    ),
                    FieldSpec(
                        name='resist_value',
                        kind='int32',
                        enum_ref='MGEF.DATA.resist_value',
                        authoring_label='Resist value',
                    ),
                    FieldSpec(
                        name='counter_effect_count',
                        kind='uint16',
                        authoring_label='Counter Effect Count',
                    ),
                    FieldSpec(
                        name='unknown_u8_6',
                        kind='uint8',
                        authoring_label='Unknown Byte 7',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='unknown_u8_7',
                        kind='uint8',
                        authoring_label='Unknown Byte 8',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='light',
                        kind='formid',
                        formlink_target='LIGH',
                        formlink_targets=('LIGH',),
                        null_allowed=True,
                        authoring_label='Light',
                    ),
                    FieldSpec(
                        name='projectile_speed',
                        kind='float32',
                        authoring_label='Projectile speed',
                    ),
                    FieldSpec(
                        name='effect_shader',
                        kind='formid',
                        formlink_target='EFSH',
                        formlink_targets=('EFSH',),
                        null_allowed=True,
                        authoring_label='Effect Shader',
                    ),
                    FieldSpec(
                        name='enchant_effect',
                        kind='formid',
                        formlink_target='EFSH',
                        formlink_targets=('EFSH',),
                        null_allowed=True,
                        authoring_label='Enchant effect',
                    ),
                    FieldSpec(
                        name='casting_sound',
                        kind='formid',
                        formlink_target='SOUN',
                        formlink_targets=('SOUN',),
                        null_allowed=True,
                        authoring_label='Casting sound',
                    ),
                    FieldSpec(
                        name='bolt_sound',
                        kind='formid',
                        formlink_target='SOUN',
                        formlink_targets=('SOUN',),
                        null_allowed=True,
                        authoring_label='Bolt sound',
                    ),
                    FieldSpec(
                        name='hit_sound',
                        kind='formid',
                        formlink_target='SOUN',
                        formlink_targets=('SOUN',),
                        null_allowed=True,
                        authoring_label='Hit sound',
                    ),
                    FieldSpec(
                        name='area_sound',
                        kind='formid',
                        formlink_target='SOUN',
                        formlink_targets=('SOUN',),
                        null_allowed=True,
                        authoring_label='Area sound',
                    ),
                    FieldSpec(
                        name='constant_effect_enchantment_factor',
                        kind='float32',
                        authoring_label='Constant Effect enchantment factor',
                    ),
                    FieldSpec(
                        name='constant_effect_barter_factor',
                        kind='float32',
                        authoring_label='Constant Effect barter factor',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='ESCE',
                kind=FieldKind.PARSED,
                display_label='Counter Effects',
                codec='array_struct:I',
                fields=(
                    FieldSpec(
                        name='counter_effects_counter_effect_code',
                        kind='uint32',
                        authoring_label='Counter Effects Counter Effect Code',
                    ),
                ),
                array=ArraySpec(layout='row_array', element_codec='I'),
                row_label='Counter Effects',
            ),
        ),
        display_label='Magic Effect',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['MISC'] = RecordSpec(
        sig='MISC',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='model_filename',
                        kind='zstring',
                        authoring_label='Model FileName',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='ICON',
                kind=FieldKind.PARSED,
                display_label='Icon FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='icon_filename',
                        kind='zstring',
                        authoring_label='Icon FileName',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='SCRI',
                kind=FieldKind.PARSED,
                display_label='Script',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='script',
                        kind='formid',
                        formlink_target='SCPT',
                        formlink_targets=('SCPT',),
                        authoring_label='Script',
                    ),
                ),
                formlink_target='SCPT',
                formlink_targets=('SCPT',),
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                codec='struct:I,I',
                fields=(
                    FieldSpec(
                        name='union_0',
                        kind='uint32',
                        notes='width-preserving union fallback for wbMISCActorValueDecider',
                    ),
                    FieldSpec(
                        name='union_1',
                        kind='uint32',
                        notes='width-preserving union fallback for wbMISCActorValueDecider',
                    ),
                ),
                required=True,
            ),
        ),
        display_label='Misc. Item',
        record_flags=RecordFlagsSpec(valid_mask=5152, bits=(RecordFlagBit(bit=10, name='Quest Item'),)),
    )

    records['NPC_'] = RecordSpec(
        sig='NPC_',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='model_filename',
                        kind='zstring',
                        authoring_label='Model FileName',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='ACBS',
                kind=FieldKind.PARSED,
                display_label='Configuration',
                codec='struct:I,H,H,H,h,H,H',
                fields=(
                    FieldSpec(
                        name='flags',
                        kind='uint32',
                        enum_ref='NPC_.ACBS.flags',
                        authoring_label='Flags',
                    ),
                    FieldSpec(
                        name='base_spell_points',
                        kind='uint16',
                        authoring_label='Base spell points',
                    ),
                    FieldSpec(
                        name='fatigue',
                        kind='uint16',
                        authoring_label='Fatigue',
                    ),
                    FieldSpec(
                        name='barter_gold',
                        kind='uint16',
                        authoring_label='Barter gold',
                    ),
                    FieldSpec(
                        name='level_offset',
                        kind='int16',
                        authoring_label='Level (offset)',
                    ),
                    FieldSpec(
                        name='calc_min',
                        kind='uint16',
                        authoring_label='Calc min',
                    ),
                    FieldSpec(
                        name='calc_max',
                        kind='uint16',
                        authoring_label='Calc max',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='SNAM',
                kind=FieldKind.PARSED,
                display_label='Factions',
                codec='struct:I,b,B,B,B',
                fields=(
                    FieldSpec(
                        name='faction',
                        kind='formid',
                        formlink_target='FACT',
                        formlink_targets=('FACT',),
                        authoring_label='Faction',
                    ),
                    FieldSpec(
                        name='rank',
                        kind='int8',
                        authoring_label='Rank',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Unknown Byte 3',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Unknown Byte 4',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_4',
                        kind='uint8',
                        authoring_label='Unknown Byte 5',
                        notes='wbUnused(3)',
                    ),
                ),
                repeatable=True,
            ),
            SubrecordSpec(
                sig='INAM',
                kind=FieldKind.PARSED,
                display_label='Death item',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='death_item',
                        kind='formid',
                        formlink_target='LVLI',
                        formlink_targets=('LVLI',),
                        authoring_label='Death item',
                    ),
                ),
                formlink_target='LVLI',
                formlink_targets=('LVLI',),
            ),
            SubrecordSpec(
                sig='RNAM',
                kind=FieldKind.PARSED,
                display_label='Race',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='race',
                        kind='formid',
                        formlink_target='RACE',
                        formlink_targets=('RACE',),
                        authoring_label='Race',
                    ),
                ),
                required=True,
                formlink_target='RACE',
                formlink_targets=('RACE',),
            ),
            SubrecordSpec(sig='SPLO', kind=FieldKind.RAW, display_label='Spell', repeatable=True),
            SubrecordSpec(
                sig='SCRI',
                kind=FieldKind.PARSED,
                display_label='Script',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='script',
                        kind='formid',
                        formlink_target='SCPT',
                        formlink_targets=('SCPT',),
                        authoring_label='Script',
                    ),
                ),
                formlink_target='SCPT',
                formlink_targets=('SCPT',),
            ),
            SubrecordSpec(
                sig='CNTO',
                kind=FieldKind.PARSED,
                display_label='Items',
                codec='struct:I,i',
                fields=(
                    FieldSpec(
                        name='item',
                        kind='formid',
                        formlink_targets=('ALCH', 'AMMO', 'APPA', 'ARMO', 'BOOK', 'CLOT', 'INGR', 'KEYM', 'LIGH', 'LVLI', 'MISC', 'SGST', 'SLGM', 'WEAP'),
                        authoring_label='Item',
                    ),
                    FieldSpec(
                        name='count',
                        kind='int32',
                        authoring_label='Count',
                    ),
                ),
                repeatable=True,
            ),
            SubrecordSpec(
                sig='AIDT',
                kind=FieldKind.PARSED,
                display_label='AI Data',
                codec='struct:B,B,B,B,I,b,B,B,B',
                fields=(
                    FieldSpec(
                        name='aggression',
                        kind='uint8',
                        authoring_label='Aggression',
                    ),
                    FieldSpec(
                        name='confidence',
                        kind='uint8',
                        authoring_label='Confidence',
                    ),
                    FieldSpec(
                        name='energy_level',
                        kind='uint8',
                        authoring_label='Energy Level',
                    ),
                    FieldSpec(
                        name='responsibility',
                        kind='uint8',
                        authoring_label='Responsibility',
                    ),
                    FieldSpec(
                        name='buys_sells_and_services',
                        kind='uint32',
                        enum_ref='service_flags',
                        authoring_label='Buys/Sells and Services',
                    ),
                    FieldSpec(
                        name='teaches',
                        kind='int8',
                        enum_ref='skill_enum',
                        authoring_label='Teaches',
                    ),
                    FieldSpec(
                        name='maximum_training_level',
                        kind='uint8',
                        authoring_label='Maximum training level',
                    ),
                    FieldSpec(
                        name='unknown_u8_7',
                        kind='uint8',
                        authoring_label='Unknown Byte 8',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='unknown_u8_8',
                        kind='uint8',
                        authoring_label='Unknown Byte 9',
                        notes='wbUnused(2)',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='PKID',
                kind=FieldKind.PARSED,
                display_label='AI Package',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='ai_package',
                        kind='formid',
                        formlink_target='PACK',
                        formlink_targets=('PACK',),
                        authoring_label='AI Package',
                    ),
                ),
                repeatable=True,
                formlink_target='PACK',
                formlink_targets=('PACK',),
            ),
            SubrecordSpec(sig='KFFZ', kind=FieldKind.RAW, display_label='Animations'),
            SubrecordSpec(
                sig='CNAM',
                kind=FieldKind.PARSED,
                display_label='Class',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='class',
                        kind='formid',
                        formlink_target='CLAS',
                        formlink_targets=('CLAS',),
                        authoring_label='Class',
                    ),
                ),
                required=True,
                formlink_target='CLAS',
                formlink_targets=('CLAS',),
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                display_label='Stats',
                codec='struct:B,B,B,B,B,B,B,B,B,B,B,B,B,B,B,B,B,B,B,B,B,H,B,B,B,B,B,B,B,B,B,B',
                fields=(
                    FieldSpec(
                        name='armorer',
                        kind='uint8',
                        authoring_label='Armorer',
                    ),
                    FieldSpec(
                        name='athletics',
                        kind='uint8',
                        authoring_label='Athletics',
                    ),
                    FieldSpec(
                        name='blade',
                        kind='uint8',
                        authoring_label='Blade',
                    ),
                    FieldSpec(
                        name='block',
                        kind='uint8',
                        authoring_label='Block',
                    ),
                    FieldSpec(
                        name='blunt',
                        kind='uint8',
                        authoring_label='Blunt',
                    ),
                    FieldSpec(
                        name='hand_to_hand',
                        kind='uint8',
                        authoring_label='Hand to Hand',
                    ),
                    FieldSpec(
                        name='heavy_armor',
                        kind='uint8',
                        authoring_label='Heavy Armor',
                    ),
                    FieldSpec(
                        name='alchemy',
                        kind='uint8',
                        authoring_label='Alchemy',
                    ),
                    FieldSpec(
                        name='alteration',
                        kind='uint8',
                        authoring_label='Alteration',
                    ),
                    FieldSpec(
                        name='conjuration',
                        kind='uint8',
                        authoring_label='Conjuration',
                    ),
                    FieldSpec(
                        name='destruction',
                        kind='uint8',
                        authoring_label='Destruction',
                    ),
                    FieldSpec(
                        name='illusion',
                        kind='uint8',
                        authoring_label='Illusion',
                    ),
                    FieldSpec(
                        name='mysticism',
                        kind='uint8',
                        authoring_label='Mysticism',
                    ),
                    FieldSpec(
                        name='restoration',
                        kind='uint8',
                        authoring_label='Restoration',
                    ),
                    FieldSpec(
                        name='acrobatics',
                        kind='uint8',
                        authoring_label='Acrobatics',
                    ),
                    FieldSpec(
                        name='light_armor',
                        kind='uint8',
                        authoring_label='Light Armor',
                    ),
                    FieldSpec(
                        name='marksman',
                        kind='uint8',
                        authoring_label='Marksman',
                    ),
                    FieldSpec(
                        name='mercantile',
                        kind='uint8',
                        authoring_label='Mercantile',
                    ),
                    FieldSpec(
                        name='security',
                        kind='uint8',
                        authoring_label='Security',
                    ),
                    FieldSpec(
                        name='sneak',
                        kind='uint8',
                        authoring_label='Sneak',
                    ),
                    FieldSpec(
                        name='speechcraft',
                        kind='uint8',
                        authoring_label='Speechcraft',
                    ),
                    FieldSpec(
                        name='health',
                        kind='uint16',
                        authoring_label='Health',
                    ),
                    FieldSpec(
                        name='unknown_u8_22',
                        kind='uint8',
                        authoring_label='Unknown Byte 23',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='unknown_u8_23',
                        kind='uint8',
                        authoring_label='Unknown Byte 24',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='strength',
                        kind='uint8',
                        authoring_label='Strength',
                    ),
                    FieldSpec(
                        name='intelligence',
                        kind='uint8',
                        authoring_label='Intelligence',
                    ),
                    FieldSpec(
                        name='willpower',
                        kind='uint8',
                        authoring_label='Willpower',
                    ),
                    FieldSpec(
                        name='agility',
                        kind='uint8',
                        authoring_label='Agility',
                    ),
                    FieldSpec(
                        name='speed',
                        kind='uint8',
                        authoring_label='Speed',
                    ),
                    FieldSpec(
                        name='endurance',
                        kind='uint8',
                        authoring_label='Endurance',
                    ),
                    FieldSpec(
                        name='personality',
                        kind='uint8',
                        authoring_label='Personality',
                    ),
                    FieldSpec(
                        name='luck',
                        kind='uint8',
                        authoring_label='Luck',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='HNAM',
                kind=FieldKind.PARSED,
                display_label='Hair',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='hair',
                        kind='formid',
                        formlink_target='HAIR',
                        formlink_targets=('HAIR',),
                        authoring_label='Hair',
                    ),
                ),
                formlink_target='HAIR',
                formlink_targets=('HAIR',),
            ),
            SubrecordSpec(
                sig='LNAM',
                kind=FieldKind.PARSED,
                display_label='Hair length',
                codec='float32',
                fields=(
                    FieldSpec(name='float32_0', kind='float32'),
                ),
            ),
            SubrecordSpec(
                sig='ENAM',
                kind=FieldKind.PARSED,
                display_label='Eyes',
                codec='formid_array',
                fields=(
                    FieldSpec(
                        name='eyes_eyes',
                        kind='formid',
                        formlink_target='EYES',
                        formlink_targets=('EYES',),
                        authoring_label='Eyes Eyes',
                    ),
                ),
                formlink_target='EYES',
                formlink_targets=('EYES',),
            ),
            SubrecordSpec(
                sig='HCLR',
                kind=FieldKind.PARSED,
                display_label='Hair color',
                codec='struct:B,B,B,B',
                fields=(
                    FieldSpec(
                        name='hair_color_red',
                        kind='uint8',
                        authoring_label='Hair color Red',
                    ),
                    FieldSpec(
                        name='hair_color_green',
                        kind='uint8',
                        authoring_label='Hair color Green',
                    ),
                    FieldSpec(
                        name='hair_color_blue',
                        kind='uint8',
                        authoring_label='Hair color Blue',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Hair color Unknown Byte 4',
                        notes='wbUnused(1)',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='ZNAM',
                kind=FieldKind.PARSED,
                display_label='Combat Style',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='combat_style',
                        kind='formid',
                        formlink_target='CSTY',
                        formlink_targets=('CSTY',),
                        authoring_label='Combat Style',
                    ),
                ),
                formlink_target='CSTY',
                formlink_targets=('CSTY',),
            ),
            SubrecordSpec(
                sig='FGGS',
                kind=FieldKind.PARSED,
                display_label='Facegen Symmetric Geometry',
                codec='array_struct:f',
                fields=(
                    FieldSpec(
                        name='facegen_symmetric_geometry_bone_morph_key',
                        kind='float32',
                        authoring_label='Facegen Symmetric Geometry Bone Morph Key',
                    ),
                ),
                repeatable=True,
                required=True,
                array=ArraySpec(layout='row_array', element_codec='f'),
                row_label='Facegen Symmetric Geometry',
                scope_id='facegen_data',
            ),
            SubrecordSpec(
                sig='FGGA',
                kind=FieldKind.PARSED,
                display_label='Facegen Asymmetric Geometry',
                codec='array_struct:f',
                fields=(
                    FieldSpec(
                        name='facegen_asymmetric_geometry_bone_morph_key',
                        kind='float32',
                        authoring_label='Facegen Asymmetric Geometry Bone Morph Key',
                    ),
                ),
                repeatable=True,
                required=True,
                array=ArraySpec(layout='row_array', element_codec='f'),
                row_label='Facegen Asymmetric Geometry',
                scope_id='facegen_data',
            ),
            SubrecordSpec(
                sig='FGTS',
                kind=FieldKind.PARSED,
                display_label='Facegen Symmetric Texture',
                codec='array_struct:f',
                fields=(
                    FieldSpec(
                        name='facegen_symmetric_texture_color_morph_key',
                        kind='float32',
                        authoring_label='Facegen Symmetric Texture Color Morph Key',
                    ),
                ),
                repeatable=True,
                required=True,
                array=ArraySpec(layout='row_array', element_codec='f'),
                row_label='Facegen Symmetric Texture',
                scope_id='facegen_data',
            ),
            SubrecordSpec(
                sig='FNAM',
                kind=FieldKind.PARSED,
                display_label='Unknown',
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='unknown',
                        kind='bytes',
                        authoring_label='Unknown',
                    ),
                ),
                required=True,
            ),
        ),
        display_label='Non-Player Character',
        record_flags=RecordFlagsSpec(valid_mask=791584, bits=(RecordFlagBit(bit=10, name='Quest Item'), RecordFlagBit(bit=18, name='Compressed'), RecordFlagBit(bit=19, name='Starts Dead'))),
    )

    records['PACK'] = RecordSpec(
        sig='PACK',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(sig='PKDT', kind=FieldKind.RAW, display_label='General', required=True),
            SubrecordSpec(
                sig='PLDT',
                kind=FieldKind.PARSED,
                display_label='Location',
                codec='struct:I,I,i',
                fields=(
                    FieldSpec(
                        name='type',
                        kind='uint32',
                        enum_ref='PACK.PLDT.type',
                        authoring_label='Type',
                    ),
                    FieldSpec(
                        name='location',
                        kind='uint32',
                        authoring_label='Location',
                        notes='width-preserving union fallback for wbPxDTLocationDecider',
                    ),
                    FieldSpec(
                        name='radius',
                        kind='int32',
                        authoring_label='Radius',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='PSDT',
                kind=FieldKind.PARSED,
                display_label='Schedule',
                codec='struct:b,b,b,b,I',
                fields=(
                    FieldSpec(
                        name='month',
                        kind='int8',
                        authoring_label='Month',
                    ),
                    FieldSpec(
                        name='day_of_week',
                        kind='int8',
                        enum_ref='package_schedule_day_of_week_enum',
                        authoring_label='Day Of Week',
                    ),
                    FieldSpec(
                        name='date',
                        kind='int8',
                        enum_ref='package_schedule_day_of_month_enum',
                        authoring_label='Date',
                    ),
                    FieldSpec(
                        name='time',
                        kind='int8',
                        enum_ref='package_schedule_hours_enum',
                        authoring_label='Time',
                    ),
                    FieldSpec(
                        name='duration_hours',
                        kind='uint32',
                        authoring_label='Duration (Hours)',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='PTDT',
                kind=FieldKind.PARSED,
                display_label='Target',
                codec='struct:I,I,i',
                fields=(
                    FieldSpec(
                        name='type',
                        kind='uint32',
                        enum_ref='PACK.PTDT.type',
                        authoring_label='Type',
                    ),
                    FieldSpec(
                        name='target',
                        kind='uint32',
                        authoring_label='Target',
                        notes='width-preserving union fallback for wbPxDTLocationDecider',
                    ),
                    FieldSpec(
                        name='count',
                        kind='int32',
                        authoring_label='Count',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='CTDA',
                kind=FieldKind.RAW,
                repeatable=True,
                scope_id='conditions',
            ),
            SubrecordSpec(
                sig='CTDT',
                kind=FieldKind.RAW,
                repeatable=True,
                scope_id='conditions',
            ),
        ),
        display_label='Package',
        record_flags=RecordFlagsSpec(valid_mask=53280, bits=(RecordFlagBit(bit=14, name='Unknown 14'), RecordFlagBit(bit=15, name='Unknown 15'))),
    )

    records['PGRD'] = RecordSpec(
        sig='PGRD',
        subrecords=(
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                display_label='Point Count',
                codec='uint16',
                fields=(
                    FieldSpec(
                        name='point_count',
                        kind='uint16',
                        authoring_label='Point Count',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='PGRP',
                kind=FieldKind.PARSED,
                display_label='Points',
                codec='array_struct:f,f,f,B,B,B,B',
                fields=(
                    FieldSpec(
                        name='points_x',
                        kind='float32',
                        authoring_label='Points X',
                    ),
                    FieldSpec(
                        name='points_y',
                        kind='float32',
                        authoring_label='Points Y',
                    ),
                    FieldSpec(
                        name='points_z_even_red_orange_odd_blue',
                        kind='float32',
                        authoring_label='Points Z (Even = Red/Orange, Odd = Blue)',
                    ),
                    FieldSpec(
                        name='points_connections',
                        kind='uint8',
                        authoring_label='Points Connections',
                    ),
                    FieldSpec(
                        name='unknown_u8_4',
                        kind='uint8',
                        authoring_label='Points Unknown Byte 5',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_5',
                        kind='uint8',
                        authoring_label='Points Unknown Byte 6',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_6',
                        kind='uint8',
                        authoring_label='Points Unknown Byte 7',
                        notes='wbUnused(3)',
                    ),
                ),
                required=True,
                array=ArraySpec(layout='row_array', element_codec='f,f,f,B,B,B,B'),
                row_label='Points',
            ),
            SubrecordSpec(
                sig='PGAG',
                kind=FieldKind.PARSED,
                display_label='Auto-Generated Point Sets',
                codec='array_struct:B',
                fields=(
                    FieldSpec(
                        name='auto_generated_point_sets_set',
                        kind='uint8',
                        enum_ref='pgag_flags',
                        authoring_label='Auto-Generated Point Sets Set',
                    ),
                ),
                array=ArraySpec(layout='row_array', element_codec='B'),
                row_label='Auto-Generated Point Sets',
            ),
            SubrecordSpec(
                sig='PGRR',
                kind=FieldKind.PARSED,
                display_label='Point-to-Point Connections',
                codec='array_struct:',
                fields=(
                    FieldSpec(
                        name='point',
                        kind='int16',
                        array=ArraySpec(layout='row_array', element_codec='h'),
                        authoring_label='Point',
                    ),
                ),
                array=ArraySpec(layout='row_array'),
                row_label='Point-to-Point Connections',
            ),
            SubrecordSpec(
                sig='PGRI',
                kind=FieldKind.PARSED,
                display_label='Inter-Cell Connections',
                codec='array_struct:H,B,B,f,f,f',
                fields=(
                    FieldSpec(
                        name='inter_cell_connections_point',
                        kind='uint16',
                        authoring_label='Inter-Cell Connections Point',
                    ),
                    FieldSpec(
                        name='unknown_u8_1',
                        kind='uint8',
                        authoring_label='Inter-Cell Connections Unknown Byte 2',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Inter-Cell Connections Unknown Byte 3',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='inter_cell_connections_x',
                        kind='float32',
                        authoring_label='Inter-Cell Connections X',
                    ),
                    FieldSpec(
                        name='inter_cell_connections_y',
                        kind='float32',
                        authoring_label='Inter-Cell Connections Y',
                    ),
                    FieldSpec(
                        name='inter_cell_connections_z',
                        kind='float32',
                        authoring_label='Inter-Cell Connections Z',
                    ),
                ),
                array=ArraySpec(layout='row_array', element_codec='H,B,B,f,f,f'),
                row_label='Inter-Cell Connections',
            ),
            SubrecordSpec(
                sig='PGRL',
                kind=FieldKind.PARSED,
                display_label='Point-to-Reference Mappings',
                codec='struct:I',
                fields=(
                    FieldSpec(
                        name='reference',
                        kind='formid',
                        formlink_target='REFR',
                        formlink_targets=('REFR',),
                        authoring_label='Reference',
                    ),
                    FieldSpec(
                        name='points',
                        kind='uint32',
                        array=ArraySpec(layout='row_array', element_codec='I'),
                        authoring_label='Points',
                    ),
                ),
                repeatable=True,
            ),
        ),
        display_label='Path Grid',
        record_flags=RecordFlagsSpec(valid_mask=266272, bits=(RecordFlagBit(bit=18, name='Compressed'),)),
    )

    records['PLYR'] = RecordSpec(
        sig='PLYR',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='PLYR',
                kind=FieldKind.PARSED,
                display_label='Player',
                codec='formid',
                fields=(
                    FieldSpec(name='formid_0', kind='formid'),
                ),
                required=True,
            ),
        ),
        display_label='Player Reference',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['QUST'] = RecordSpec(
        sig='QUST',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='SCRI',
                kind=FieldKind.PARSED,
                display_label='Script',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='script',
                        kind='formid',
                        formlink_target='SCPT',
                        formlink_targets=('SCPT',),
                        authoring_label='Script',
                    ),
                ),
                formlink_target='SCPT',
                formlink_targets=('SCPT',),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='ICON',
                kind=FieldKind.PARSED,
                display_label='Icon FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='icon_filename',
                        kind='zstring',
                        authoring_label='Icon FileName',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                display_label='General',
                codec='struct:B,B',
                fields=(
                    FieldSpec(
                        name='flags',
                        kind='uint8',
                        enum_ref='QUST.DATA.flags',
                        authoring_label='Flags',
                    ),
                    FieldSpec(
                        name='priority',
                        kind='uint8',
                        authoring_label='Priority',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='CTDA',
                kind=FieldKind.RAW,
                repeatable=True,
                scope_id='conditions',
            ),
            SubrecordSpec(
                sig='CTDT',
                kind=FieldKind.RAW,
                repeatable=True,
                scope_id='conditions',
            ),
            SubrecordSpec(
                sig='INDX',
                kind=FieldKind.PARSED,
                display_label='Stage index',
                codec='int16',
                fields=(
                    FieldSpec(
                        name='stage_index',
                        kind='int16',
                        authoring_label='Stage index',
                    ),
                ),
                repeatable=True,
                scope_id='stages',
            ),
            SubrecordSpec(
                sig='QSDT',
                kind=FieldKind.PARSED,
                display_label='Complete Quest',
                codec='uint8',
                fields=(
                    FieldSpec(
                        name='complete_quest',
                        kind='uint8',
                        enum_ref='bool_enum',
                        authoring_label='Complete Quest',
                    ),
                ),
                repeatable=True,
                enum_ref='bool_enum',
                scope_id='stages',
            ),
            SubrecordSpec(
                sig='CTDA',
                kind=FieldKind.RAW,
                repeatable=True,
                scope_id='stages',
            ),
            SubrecordSpec(
                sig='CTDT',
                kind=FieldKind.RAW,
                repeatable=True,
                scope_id='stages',
            ),
            SubrecordSpec(
                sig='CNAM',
                kind=FieldKind.PARSED,
                display_label='Log Entry',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='log_entry',
                        kind='zstring',
                        authoring_label='Log Entry',
                    ),
                ),
                repeatable=True,
                scope_id='stages',
            ),
            SubrecordSpec(
                sig='SCHR',
                kind=FieldKind.PARSED,
                display_label='Basic Script Data',
                codec='struct:B,B,B,B,I,I,I,I',
                fields=(
                    FieldSpec(
                        name='unknown_u8_0',
                        kind='uint8',
                        authoring_label='Unknown Byte 1',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='unknown_u8_1',
                        kind='uint8',
                        authoring_label='Unknown Byte 2',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Unknown Byte 3',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Unknown Byte 4',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='refcount',
                        kind='uint32',
                        authoring_label='RefCount',
                    ),
                    FieldSpec(
                        name='compiledsize',
                        kind='uint32',
                        authoring_label='CompiledSize',
                    ),
                    FieldSpec(
                        name='variablecount',
                        kind='uint32',
                        authoring_label='VariableCount',
                    ),
                    FieldSpec(
                        name='type',
                        kind='uint32',
                        enum_ref='QUST.SCHR.type',
                        authoring_label='Type',
                    ),
                ),
                repeatable=True,
                scope_id='stages',
            ),
            SubrecordSpec(
                sig='SCHD',
                kind=FieldKind.PARSED,
                display_label='Basic Script Data',
                codec='struct:B,B,B,B,I,I,I,I',
                fields=(
                    FieldSpec(
                        name='unknown_u8_0',
                        kind='uint8',
                        authoring_label='Unknown Byte 1',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='unknown_u8_1',
                        kind='uint8',
                        authoring_label='Unknown Byte 2',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Unknown Byte 3',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Unknown Byte 4',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='refcount',
                        kind='uint32',
                        authoring_label='RefCount',
                    ),
                    FieldSpec(
                        name='compiledsize',
                        kind='uint32',
                        authoring_label='CompiledSize',
                    ),
                    FieldSpec(
                        name='variablecount',
                        kind='uint32',
                        authoring_label='VariableCount',
                    ),
                    FieldSpec(
                        name='type',
                        kind='uint32',
                        enum_ref='QUST.SCHD.type',
                        authoring_label='Type',
                    ),
                    FieldSpec(
                        name='unknown',
                        kind='bytes',
                        authoring_label='Unknown',
                    ),
                ),
                repeatable=True,
                scope_id='stages',
            ),
            SubrecordSpec(
                sig='SCDA',
                kind=FieldKind.PARSED,
                display_label='Compiled result script',
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='compiled_result_script',
                        kind='bytes',
                        authoring_label='Compiled result script',
                    ),
                ),
                repeatable=True,
                scope_id='stages',
            ),
            SubrecordSpec(
                sig='SCTX',
                kind=FieldKind.RAW,
                display_label='Result script source',
                repeatable=True,
                scope_id='stages',
            ),
            SubrecordSpec(
                sig='SCRO',
                kind=FieldKind.PARSED,
                display_label='Global Reference',
                codec='formid',
                fields=(
                    FieldSpec(name='formid_0', kind='formid'),
                ),
                repeatable=True,
                scope_id='stages',
            ),
            SubrecordSpec(
                sig='SCRV',
                kind=FieldKind.PARSED,
                display_label='Local Variable',
                codec='uint32',
                fields=(
                    FieldSpec(
                        name='local_variable',
                        kind='uint32',
                        authoring_label='Local Variable',
                    ),
                ),
                repeatable=True,
                scope_id='stages',
            ),
            SubrecordSpec(
                sig='QSTA',
                kind=FieldKind.PARSED,
                codec='struct:I,B,B,B,B',
                fields=(
                    FieldSpec(
                        name='target',
                        kind='formid',
                        formlink_targets=('ACHR', 'ACRE', 'REFR'),
                        authoring_label='Target',
                    ),
                    FieldSpec(
                        name='compass_marker_ignores_locks',
                        kind='uint8',
                        enum_ref='bool_enum',
                        authoring_label='Compass Marker Ignores Locks',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Unknown Byte 3',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Unknown Byte 4',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_4',
                        kind='uint8',
                        authoring_label='Unknown Byte 5',
                        notes='wbUnused(3)',
                    ),
                ),
                repeatable=True,
                scope_id='targets',
            ),
            SubrecordSpec(
                sig='CTDA',
                kind=FieldKind.RAW,
                repeatable=True,
                scope_id='targets',
            ),
            SubrecordSpec(
                sig='CTDT',
                kind=FieldKind.RAW,
                repeatable=True,
                scope_id='targets',
            ),
        ),
        display_label='Quest',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['RACE'] = RecordSpec(
        sig='RACE',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='DESC',
                kind=FieldKind.PARSED,
                display_label='Description',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='description',
                        kind='zstring',
                        authoring_label='Description',
                    ),
                ),
            ),
            SubrecordSpec(sig='SPLO', kind=FieldKind.RAW, display_label='Spell', repeatable=True),
            SubrecordSpec(
                sig='XNAM',
                kind=FieldKind.PARSED,
                display_label='Relations',
                codec='struct:I,i',
                fields=(
                    FieldSpec(
                        name='faction',
                        kind='formid',
                        formlink_targets=('FACT', 'RACE'),
                        authoring_label='Faction',
                    ),
                    FieldSpec(
                        name='modifier',
                        kind='int32',
                        authoring_label='Modifier',
                    ),
                ),
                repeatable=True,
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                codec='struct:B,B,f,f,f,f,I',
                fields=(
                    FieldSpec(
                        name='skill_boosts',
                        kind='struct',
                        nested_fields=(
                            FieldSpec(
                                name='skill',
                                kind='int8',
                                enum_ref='major_skill_enum',
                                authoring_label='Skill',
                            ),
                            FieldSpec(
                                name='boost',
                                kind='int8',
                                authoring_label='Boost',
                            ),
                        ),
                        array=ArraySpec(layout='row_array', element_codec='b,b'),
                        authoring_label='Skill Boosts',
                    ),
                    FieldSpec(
                        name='unknown_u8_1',
                        kind='uint8',
                        authoring_label='Unknown Byte 2',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Unknown Byte 3',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='male_height',
                        kind='float32',
                        authoring_label='Male Height',
                    ),
                    FieldSpec(
                        name='female_height',
                        kind='float32',
                        authoring_label='Female Height',
                    ),
                    FieldSpec(
                        name='male_weight',
                        kind='float32',
                        authoring_label='Male Weight',
                    ),
                    FieldSpec(
                        name='female_weight',
                        kind='float32',
                        authoring_label='Female Weight',
                    ),
                    FieldSpec(
                        name='playable',
                        kind='uint32',
                        enum_ref='bool_enum',
                        authoring_label='Playable',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='VNAM',
                kind=FieldKind.PARSED,
                display_label='Voice',
                codec='struct:I,I',
                fields=(
                    FieldSpec(
                        name='male',
                        kind='formid',
                        formlink_target='RACE',
                        formlink_targets=('RACE',),
                        null_allowed=True,
                        authoring_label='Male',
                    ),
                    FieldSpec(
                        name='female',
                        kind='formid',
                        formlink_target='RACE',
                        formlink_targets=('RACE',),
                        null_allowed=True,
                        authoring_label='Female',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='DNAM',
                kind=FieldKind.PARSED,
                display_label='Default Hair',
                codec='struct:I,I',
                fields=(
                    FieldSpec(
                        name='male',
                        kind='formid',
                        formlink_target='HAIR',
                        formlink_targets=('HAIR',),
                        authoring_label='Male',
                    ),
                    FieldSpec(
                        name='female',
                        kind='formid',
                        formlink_target='HAIR',
                        formlink_targets=('HAIR',),
                        authoring_label='Female',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='CNAM',
                kind=FieldKind.PARSED,
                display_label='Default Hair Color',
                codec='uint8',
                fields=(
                    FieldSpec(
                        name='default_hair_color',
                        kind='uint8',
                        authoring_label='Default Hair Color',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='PNAM',
                kind=FieldKind.PARSED,
                display_label='FaceGen - Main clamp',
                codec='float32',
                fields=(
                    FieldSpec(name='float32_0', kind='float32'),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='UNAM',
                kind=FieldKind.PARSED,
                display_label='FaceGen - Face clamp',
                codec='float32',
                fields=(
                    FieldSpec(name='float32_0', kind='float32'),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='ATTR',
                kind=FieldKind.PARSED,
                display_label='Base Attributes',
                codec='struct:B,B,B,B,B,B,B,B,B,B,B,B,B,B,B,B',
                fields=(
                    FieldSpec(
                        name='male_strength',
                        kind='uint8',
                        authoring_label='Male Strength',
                    ),
                    FieldSpec(
                        name='male_intelligence',
                        kind='uint8',
                        authoring_label='Male Intelligence',
                    ),
                    FieldSpec(
                        name='male_willpower',
                        kind='uint8',
                        authoring_label='Male Willpower',
                    ),
                    FieldSpec(
                        name='male_agility',
                        kind='uint8',
                        authoring_label='Male Agility',
                    ),
                    FieldSpec(
                        name='male_speed',
                        kind='uint8',
                        authoring_label='Male Speed',
                    ),
                    FieldSpec(
                        name='male_endurance',
                        kind='uint8',
                        authoring_label='Male Endurance',
                    ),
                    FieldSpec(
                        name='male_personality',
                        kind='uint8',
                        authoring_label='Male Personality',
                    ),
                    FieldSpec(
                        name='male_luck',
                        kind='uint8',
                        authoring_label='Male Luck',
                    ),
                    FieldSpec(
                        name='female_strength',
                        kind='uint8',
                        authoring_label='Female Strength',
                    ),
                    FieldSpec(
                        name='female_intelligence',
                        kind='uint8',
                        authoring_label='Female Intelligence',
                    ),
                    FieldSpec(
                        name='female_willpower',
                        kind='uint8',
                        authoring_label='Female Willpower',
                    ),
                    FieldSpec(
                        name='female_agility',
                        kind='uint8',
                        authoring_label='Female Agility',
                    ),
                    FieldSpec(
                        name='female_speed',
                        kind='uint8',
                        authoring_label='Female Speed',
                    ),
                    FieldSpec(
                        name='female_endurance',
                        kind='uint8',
                        authoring_label='Female Endurance',
                    ),
                    FieldSpec(
                        name='female_personality',
                        kind='uint8',
                        authoring_label='Female Personality',
                    ),
                    FieldSpec(
                        name='female_luck',
                        kind='uint8',
                        authoring_label='Female Luck',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='NAM0',
                kind=FieldKind.PARSED,
                display_label='Face Data Marker',
                codec='empty',
                fields=(
                    FieldSpec(
                        name='face_data_marker',
                        kind='empty',
                        authoring_label='Face Data Marker',
                        notes='wbEmpty(NAM0, \'Face Data Marker\')',
                    ),
                ),
                repeatable=True,
                required=True,
                scope_id='face_data',
            ),
            SubrecordSpec(
                sig='NAM1',
                kind=FieldKind.PARSED,
                display_label='Body Data Marker',
                codec='empty',
                fields=(
                    FieldSpec(
                        name='body_data_marker',
                        kind='empty',
                        authoring_label='Body Data Marker',
                        notes='wbEmpty(NAM1, \'Body Data Marker\').SetRequired',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='MNAM',
                kind=FieldKind.PARSED,
                display_label='Male Body Data Marker',
                codec='empty',
                fields=(
                    FieldSpec(
                        name='male_body_data_marker',
                        kind='empty',
                        authoring_label='Male Body Data Marker',
                        notes='wbEmpty(MNAM, \'Male Body Data Marker\')',
                    ),
                ),
                repeatable=True,
                required=True,
                authoring_layout='row_group',
                authoring_key='group_male_body_data',
                scope_id='male_body_data',
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='model_filename',
                        kind='zstring',
                        authoring_label='Model FileName',
                    ),
                ),
                repeatable=True,
                required=True,
                authoring_layout='row_group',
                authoring_key='group_male_body_data',
                scope_id='male_body_data',
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
                repeatable=True,
                required=True,
                authoring_layout='row_group',
                authoring_key='group_male_body_data',
                scope_id='male_body_data',
            ),
            SubrecordSpec(
                sig='INDX',
                kind=FieldKind.PARSED,
                display_label='Index',
                codec='uint32',
                fields=(
                    FieldSpec(
                        name='index',
                        kind='uint32',
                        enum_ref='body_part_index_enum',
                        authoring_label='Index',
                    ),
                ),
                repeatable=True,
                required=True,
                enum_ref='body_part_index_enum',
                authoring_layout='row_group',
                authoring_key='group_male_body_data',
                scope_id='male_body_data',
            ),
            SubrecordSpec(
                sig='ICON',
                kind=FieldKind.PARSED,
                display_label='Icon FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='icon_filename',
                        kind='zstring',
                        authoring_label='Icon FileName',
                    ),
                ),
                repeatable=True,
                required=True,
                authoring_layout='row_group',
                authoring_key='group_male_body_data',
                scope_id='male_body_data',
            ),
            SubrecordSpec(
                sig='FNAM',
                kind=FieldKind.PARSED,
                display_label='Female Body Data Marker',
                codec='empty',
                fields=(
                    FieldSpec(
                        name='female_body_data_marker',
                        kind='empty',
                        authoring_label='Female Body Data Marker',
                        notes='wbEmpty(FNAM, \'Female Body Data Marker\')',
                    ),
                ),
                repeatable=True,
                required=True,
                authoring_layout='row_group',
                authoring_key='group_female_body_data',
                scope_id='female_body_data',
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='model_filename',
                        kind='zstring',
                        authoring_label='Model FileName',
                    ),
                ),
                repeatable=True,
                required=True,
                authoring_layout='row_group',
                authoring_key='group_female_body_data',
                scope_id='female_body_data',
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
                repeatable=True,
                required=True,
                authoring_layout='row_group',
                authoring_key='group_female_body_data',
                scope_id='female_body_data',
            ),
            SubrecordSpec(
                sig='INDX',
                kind=FieldKind.PARSED,
                display_label='Index',
                codec='uint32',
                fields=(
                    FieldSpec(
                        name='index',
                        kind='uint32',
                        enum_ref='body_part_index_enum',
                        authoring_label='Index',
                    ),
                ),
                repeatable=True,
                required=True,
                enum_ref='body_part_index_enum',
                authoring_layout='row_group',
                authoring_key='group_female_body_data',
                scope_id='female_body_data',
            ),
            SubrecordSpec(
                sig='ICON',
                kind=FieldKind.PARSED,
                display_label='Icon FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='icon_filename',
                        kind='zstring',
                        authoring_label='Icon FileName',
                    ),
                ),
                repeatable=True,
                required=True,
                authoring_layout='row_group',
                authoring_key='group_female_body_data',
                scope_id='female_body_data',
            ),
            SubrecordSpec(
                sig='HNAM',
                kind=FieldKind.PARSED,
                display_label='Hairs',
                codec='formid_array',
                fields=(
                    FieldSpec(
                        name='hairs_hair',
                        kind='formid',
                        formlink_target='HAIR',
                        formlink_targets=('HAIR',),
                        authoring_label='Hairs Hair',
                    ),
                ),
                required=True,
                formlink_target='HAIR',
                formlink_targets=('HAIR',),
            ),
            SubrecordSpec(
                sig='ENAM',
                kind=FieldKind.PARSED,
                display_label='Eyes',
                codec='formid_array',
                fields=(
                    FieldSpec(
                        name='eyes_eye',
                        kind='formid',
                        formlink_target='EYES',
                        formlink_targets=('EYES',),
                        authoring_label='Eyes Eye',
                    ),
                ),
                required=True,
                formlink_target='EYES',
                formlink_targets=('EYES',),
            ),
            SubrecordSpec(
                sig='FGGS',
                kind=FieldKind.PARSED,
                display_label='Facegen Symmetric Geometry',
                codec='array_struct:f',
                fields=(
                    FieldSpec(
                        name='facegen_symmetric_geometry_bone_morph_key',
                        kind='float32',
                        authoring_label='Facegen Symmetric Geometry Bone Morph Key',
                    ),
                ),
                repeatable=True,
                required=True,
                array=ArraySpec(layout='row_array', element_codec='f'),
                row_label='Facegen Symmetric Geometry',
                scope_id='facegen_data',
            ),
            SubrecordSpec(
                sig='FGGA',
                kind=FieldKind.PARSED,
                display_label='Facegen Asymmetric Geometry',
                codec='array_struct:f',
                fields=(
                    FieldSpec(
                        name='facegen_asymmetric_geometry_bone_morph_key',
                        kind='float32',
                        authoring_label='Facegen Asymmetric Geometry Bone Morph Key',
                    ),
                ),
                repeatable=True,
                required=True,
                array=ArraySpec(layout='row_array', element_codec='f'),
                row_label='Facegen Asymmetric Geometry',
                scope_id='facegen_data',
            ),
            SubrecordSpec(
                sig='FGTS',
                kind=FieldKind.PARSED,
                display_label='Facegen Symmetric Texture',
                codec='array_struct:f',
                fields=(
                    FieldSpec(
                        name='facegen_symmetric_texture_color_morph_key',
                        kind='float32',
                        authoring_label='Facegen Symmetric Texture Color Morph Key',
                    ),
                ),
                repeatable=True,
                required=True,
                array=ArraySpec(layout='row_array', element_codec='f'),
                row_label='Facegen Symmetric Texture',
                scope_id='facegen_data',
            ),
            SubrecordSpec(
                sig='SNAM',
                kind=FieldKind.PARSED,
                display_label='Unknown',
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='unknown',
                        kind='bytes',
                        authoring_label='Unknown',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='HEAD',
                kind=FieldKind.PARSED,
                display_label='Head',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='head',
                        kind='formid',
                        authoring_label='Head',
                    ),
                ),
                repeatable=True,
            ),
        ),
        display_label='Race',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['REFR'] = RecordSpec(
        sig='REFR',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='NAME',
                kind=FieldKind.PARSED,
                display_label='Base',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='base',
                        kind='formid',
                        formlink_targets=('ACTI', 'ALCH', 'AMMO', 'APPA', 'ARMO', 'BOOK', 'CLOT', 'CONT', 'DOOR', 'FLOR', 'FURN', 'GRAS', 'INGR', 'KEYM', 'LIGH', 'LVLC', 'MISC', 'SBSP', 'SGST', 'SLGM', 'SOUN', 'STAT', 'TREE', 'WEAP'),
                        authoring_label='Base',
                    ),
                ),
                formlink_targets=('ACTI', 'ALCH', 'AMMO', 'APPA', 'ARMO', 'BOOK', 'CLOT', 'CONT', 'DOOR', 'FLOR', 'FURN', 'GRAS', 'INGR', 'KEYM', 'LIGH', 'LVLC', 'MISC', 'SBSP', 'SGST', 'SLGM', 'SOUN', 'STAT', 'TREE', 'WEAP'),
            ),
            SubrecordSpec(
                sig='XTEL',
                kind=FieldKind.PARSED,
                display_label='Teleport Destination',
                codec='struct:I,f,f,f,f,f,f',
                fields=(
                    FieldSpec(
                        name='door',
                        kind='formid',
                        formlink_target='REFR',
                        formlink_targets=('REFR',),
                        authoring_label='Door',
                    ),
                    FieldSpec(
                        name='position_x',
                        kind='float32',
                        authoring_label='Position X',
                    ),
                    FieldSpec(
                        name='position_y',
                        kind='float32',
                        authoring_label='Position Y',
                    ),
                    FieldSpec(
                        name='position_z',
                        kind='float32',
                        authoring_label='Position Z',
                    ),
                    FieldSpec(
                        name='rotation_x',
                        kind='float32',
                        authoring_label='Rotation X',
                    ),
                    FieldSpec(
                        name='rotation_y',
                        kind='float32',
                        authoring_label='Rotation Y',
                    ),
                    FieldSpec(
                        name='rotation_z',
                        kind='float32',
                        authoring_label='Rotation Z',
                    ),
                ),
            ),
            SubrecordSpec(sig='XLOC', kind=FieldKind.RAW, display_label='Lock information'),
            SubrecordSpec(
                sig='XESP',
                kind=FieldKind.PARSED,
                display_label='Enable Parent',
                codec='struct:I,B,B,B,B',
                fields=(
                    FieldSpec(
                        name='reference',
                        kind='formid',
                        formlink_targets=('ACHR', 'ACRE', 'PLYR', 'REFR'),
                        authoring_label='Reference',
                    ),
                    FieldSpec(
                        name='set_enable_state_to_opposite_of_parent',
                        kind='uint8',
                        enum_ref='bool_enum',
                        authoring_label='Set Enable State To Opposite Of Parent',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Unknown Byte 3',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Unknown Byte 4',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_4',
                        kind='uint8',
                        authoring_label='Unknown Byte 5',
                        notes='wbUnused(3)',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='XTRG',
                kind=FieldKind.PARSED,
                display_label='Target',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='target',
                        kind='formid',
                        formlink_targets=('ACHR', 'ACRE', 'REFR'),
                        authoring_label='Target',
                    ),
                ),
                formlink_targets=('ACHR', 'ACRE', 'REFR'),
            ),
            SubrecordSpec(
                sig='XSED',
                kind=FieldKind.PARSED,
                display_label='Speed Tree',
                codec='struct:B',
                fields=(
                    FieldSpec(
                        name='seed',
                        kind='uint8',
                        authoring_label='Seed',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='XLOD',
                kind=FieldKind.PARSED,
                display_label='Distant LOD Data',
                codec='array_struct:f',
                fields=(
                    FieldSpec(
                        name='distant_lod_data_unknown',
                        kind='float32',
                        authoring_label='Distant LOD Data Unknown',
                    ),
                ),
                array=ArraySpec(layout='row_array', element_codec='f'),
                row_label='Distant LOD Data',
            ),
            SubrecordSpec(
                sig='XCHG',
                kind=FieldKind.PARSED,
                display_label='Charge',
                codec='float32',
                fields=(
                    FieldSpec(name='float32_0', kind='float32'),
                ),
            ),
            SubrecordSpec(
                sig='XHLT',
                kind=FieldKind.PARSED,
                display_label='Health',
                codec='int32',
                fields=(
                    FieldSpec(
                        name='health',
                        kind='int32',
                        authoring_label='Health',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='XPCI',
                kind=FieldKind.PARSED,
                display_label='Unused',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='unused',
                        kind='formid',
                        formlink_target='CELL',
                        formlink_targets=('CELL',),
                        authoring_label='Unused',
                    ),
                ),
                repeatable=True,
                formlink_target='CELL',
                formlink_targets=('CELL',),
                authoring_layout='row_group',
                authoring_key='group_unused',
                scope_id='unused',
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Unused',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='unused',
                        kind='zstring',
                        authoring_label='Unused',
                    ),
                ),
                repeatable=True,
                authoring_layout='row_group',
                authoring_key='group_unused',
                scope_id='unused',
            ),
            SubrecordSpec(
                sig='XLCM',
                kind=FieldKind.PARSED,
                display_label='Level Modifier',
                codec='int32',
                fields=(
                    FieldSpec(
                        name='level_modifier',
                        kind='int32',
                        authoring_label='Level Modifier',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='XRTM',
                kind=FieldKind.PARSED,
                display_label='Reference Teleport Marker',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='reference_teleport_marker',
                        kind='formid',
                        formlink_target='REFR',
                        formlink_targets=('REFR',),
                        authoring_label='Reference Teleport Marker',
                    ),
                ),
                formlink_target='REFR',
                formlink_targets=('REFR',),
            ),
            SubrecordSpec(
                sig='XACT',
                kind=FieldKind.PARSED,
                display_label='Action Flag',
                codec='uint32',
                fields=(
                    FieldSpec(
                        name='action_flag',
                        kind='uint32',
                        enum_ref='REFR.XACT.action_flag',
                        authoring_label='Action Flag',
                    ),
                ),
                enum_ref='REFR.XACT.action_flag',
            ),
            SubrecordSpec(
                sig='XCNT',
                kind=FieldKind.PARSED,
                display_label='Count',
                codec='uint32',
                fields=(
                    FieldSpec(
                        name='count',
                        kind='uint32',
                        authoring_label='Count',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='XMRK',
                kind=FieldKind.PARSED,
                display_label='Map Marker Data',
                codec='empty',
                fields=(
                    FieldSpec(
                        name='map_marker_data',
                        kind='empty',
                        authoring_label='Map Marker Data',
                        notes='wbEmpty(XMRK, \'Map Marker Data\')',
                    ),
                ),
                repeatable=True,
                required=True,
                authoring_layout='row_group',
                authoring_key='group_map_marker',
                scope_id='map_marker',
            ),
            SubrecordSpec(
                sig='FNAM',
                kind=FieldKind.PARSED,
                display_label='Map Flags',
                codec='uint8',
                fields=(
                    FieldSpec(
                        name='map_flags',
                        kind='uint8',
                        enum_ref='REFR.FNAM.map_flags',
                        authoring_label='Map Flags',
                    ),
                ),
                repeatable=True,
                required=True,
                enum_ref='REFR.FNAM.map_flags',
                authoring_layout='row_group',
                authoring_key='group_map_marker',
                scope_id='map_marker',
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
                repeatable=True,
                required=True,
                authoring_layout='row_group',
                authoring_key='group_map_marker',
                scope_id='map_marker',
            ),
            SubrecordSpec(
                sig='TNAM',
                kind=FieldKind.PARSED,
                codec='struct:B,B',
                fields=(
                    FieldSpec(
                        name='type',
                        kind='uint8',
                        enum_ref='REFR.TNAM.type',
                        authoring_label='Type',
                    ),
                    FieldSpec(
                        name='unknown_u8_1',
                        kind='uint8',
                        authoring_label='Unknown Byte 2',
                        notes='wbUnused(1)',
                    ),
                ),
                repeatable=True,
                required=True,
                authoring_layout='row_group',
                authoring_key='group_map_marker',
                scope_id='map_marker',
            ),
            SubrecordSpec(
                sig='ONAM',
                kind=FieldKind.PARSED,
                display_label='Open by Default',
                codec='empty',
                fields=(
                    FieldSpec(
                        name='open_by_default',
                        kind='empty',
                        authoring_label='Open by Default',
                        notes='wbEmpty(ONAM, \'Open by Default\')',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='XRGD',
                kind=FieldKind.PARSED,
                display_label='Bones',
                codec='array_struct:B,B,B,B,f,f,f,f,f,f',
                fields=(
                    FieldSpec(
                        name='bones_bone_id',
                        kind='uint8',
                        authoring_label='Bones Bone Id',
                    ),
                    FieldSpec(
                        name='unknown_u8_1',
                        kind='uint8',
                        authoring_label='Bones Unknown Byte 2',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Bones Unknown Byte 3',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Bones Unknown Byte 4',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='bones_position_x',
                        kind='float32',
                        authoring_label='Bones Position X',
                    ),
                    FieldSpec(
                        name='bones_position_y',
                        kind='float32',
                        authoring_label='Bones Position Y',
                    ),
                    FieldSpec(
                        name='bones_position_z',
                        kind='float32',
                        authoring_label='Bones Position Z',
                    ),
                    FieldSpec(
                        name='bones_rotation_x',
                        kind='float32',
                        authoring_label='Bones Rotation X',
                    ),
                    FieldSpec(
                        name='bones_rotation_y',
                        kind='float32',
                        authoring_label='Bones Rotation Y',
                    ),
                    FieldSpec(
                        name='bones_rotation_z',
                        kind='float32',
                        authoring_label='Bones Rotation Z',
                    ),
                ),
                repeatable=True,
                array=ArraySpec(layout='row_array', element_codec='B,B,B,B,f,f,f,f,f,f'),
                row_label='Bones',
                scope_id='ragdoll_data',
            ),
            SubrecordSpec(
                sig='XSCL',
                kind=FieldKind.PARSED,
                display_label='Scale',
                codec='float32',
                fields=(
                    FieldSpec(name='float32_0', kind='float32'),
                ),
            ),
            SubrecordSpec(
                sig='XSOL',
                kind=FieldKind.PARSED,
                display_label='Contained Soul',
                codec='uint8',
                fields=(
                    FieldSpec(
                        name='contained_soul',
                        kind='uint8',
                        enum_ref='soul_gem_enum',
                        authoring_label='Contained Soul',
                    ),
                ),
                enum_ref='soul_gem_enum',
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                codec='struct:f,f,f,f,f,f',
                fields=(
                    FieldSpec(
                        name='position_rotation_position_x',
                        kind='float32',
                        authoring_label='Position Rotation Position X',
                    ),
                    FieldSpec(
                        name='position_rotation_position_y',
                        kind='float32',
                        authoring_label='Position Rotation Position Y',
                    ),
                    FieldSpec(
                        name='position_rotation_position_z',
                        kind='float32',
                        authoring_label='Position Rotation Position Z',
                    ),
                    FieldSpec(
                        name='position_rotation_rotation_x',
                        kind='float32',
                        authoring_label='Position Rotation Rotation X',
                    ),
                    FieldSpec(
                        name='position_rotation_rotation_y',
                        kind='float32',
                        authoring_label='Position Rotation Rotation Y',
                    ),
                    FieldSpec(
                        name='position_rotation_rotation_z',
                        kind='float32',
                        authoring_label='Position Rotation Rotation Z',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='XOWN',
                kind=FieldKind.PARSED,
                display_label='Owner',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='owner',
                        kind='formid',
                        authoring_label='Owner',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='XRNK',
                kind=FieldKind.PARSED,
                display_label='Faction rank',
                codec='int32',
                fields=(
                    FieldSpec(
                        name='faction_rank',
                        kind='int32',
                        authoring_label='Faction rank',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='XGLB',
                kind=FieldKind.PARSED,
                display_label='Global',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='global',
                        kind='formid',
                        authoring_label='Global',
                    ),
                ),
            ),
        ),
        display_label='Placed Object',
        record_flags=RecordFlagsSpec(valid_mask=40608, bits=(RecordFlagBit(bit=7, name='Turn Off Fire'), RecordFlagBit(bit=9, name='Cast Shadows'), RecordFlagBit(bit=10, name='Persistent'), RecordFlagBit(bit=11, name='Initially Disabled'), RecordFlagBit(bit=15, name='Visible When Distant'))),
    )

    records['REGN'] = RecordSpec(
        sig='REGN',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='ICON',
                kind=FieldKind.PARSED,
                display_label='Icon FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='icon_filename',
                        kind='zstring',
                        authoring_label='Icon FileName',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='RCLR',
                kind=FieldKind.PARSED,
                display_label='Map Color',
                codec='struct:B,B,B,B',
                fields=(
                    FieldSpec(
                        name='map_color_red',
                        kind='uint8',
                        authoring_label='Map Color Red',
                    ),
                    FieldSpec(
                        name='map_color_green',
                        kind='uint8',
                        authoring_label='Map Color Green',
                    ),
                    FieldSpec(
                        name='map_color_blue',
                        kind='uint8',
                        authoring_label='Map Color Blue',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Map Color Unknown Byte 4',
                        notes='wbUnused(1)',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='WNAM',
                kind=FieldKind.PARSED,
                display_label='Worldspace',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='worldspace',
                        kind='formid',
                        formlink_target='WRLD',
                        formlink_targets=('WRLD',),
                        authoring_label='Worldspace',
                    ),
                ),
                formlink_target='WRLD',
                formlink_targets=('WRLD',),
            ),
            SubrecordSpec(
                sig='RPLI',
                kind=FieldKind.PARSED,
                display_label='Edge Fall-off',
                codec='uint32',
                fields=(
                    FieldSpec(
                        name='edge_fall_off',
                        kind='uint32',
                        authoring_label='Edge Fall-off',
                    ),
                ),
                repeatable=True,
                scope_id='region_areas',
            ),
            SubrecordSpec(
                sig='RPLD',
                kind=FieldKind.PARSED,
                display_label='Points',
                codec='array_struct:f,f',
                fields=(
                    FieldSpec(
                        name='points_x',
                        kind='float32',
                        authoring_label='Points X',
                    ),
                    FieldSpec(
                        name='points_y',
                        kind='float32',
                        authoring_label='Points Y',
                    ),
                ),
                repeatable=True,
                array=ArraySpec(layout='row_array', element_codec='f,f'),
                row_label='Points',
                scope_id='region_areas',
            ),
            SubrecordSpec(
                sig='ANAM',
                kind=FieldKind.RAW,
                repeatable=True,
                scope_id='region_areas',
            ),
            SubrecordSpec(
                sig='RDAT',
                kind=FieldKind.PARSED_WITH_RAW_FALLBACK,
                codec='struct:I,B,B,B,B',
                fields=(
                    FieldSpec(
                        name='type',
                        kind='uint32',
                        enum_ref='REGN.RDAT.type',
                        authoring_label='Type',
                    ),
                    FieldSpec(
                        name='override',
                        kind='uint8',
                        enum_ref='bool_enum',
                        authoring_label='Override',
                    ),
                    FieldSpec(
                        name='priority',
                        kind='uint8',
                        authoring_label='Priority',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Unknown Byte 4',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='unknown_u8_4',
                        kind='uint8',
                        authoring_label='Unknown Byte 5',
                        notes='wbUnused(2)',
                    ),
                ),
                repeatable=True,
                scope_id='region_data_entries',
            ),
            SubrecordSpec(
                sig='RDOT',
                kind=FieldKind.PARSED,
                display_label='Objects',
                codec='array_struct:I,H,B,B,f,B,B,B,B,H,H,f,f,f,f,f,H,H,H,B,B,B,B,B,B',
                fields=(
                    FieldSpec(
                        name='objects_object',
                        kind='formid',
                        formlink_targets=('FLOR', 'LTEX', 'STAT', 'TREE'),
                        authoring_label='Objects Object',
                    ),
                    FieldSpec(
                        name='objects_parent_index',
                        kind='uint16',
                        authoring_label='Objects Parent Index',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Objects Unknown Byte 3',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Objects Unknown Byte 4',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='objects_density',
                        kind='float32',
                        authoring_label='Objects Density',
                    ),
                    FieldSpec(
                        name='objects_clustering',
                        kind='uint8',
                        authoring_label='Objects Clustering',
                    ),
                    FieldSpec(
                        name='objects_min_slope',
                        kind='uint8',
                        authoring_label='Objects Min Slope',
                    ),
                    FieldSpec(
                        name='objects_max_slope',
                        kind='uint8',
                        authoring_label='Objects Max Slope',
                    ),
                    FieldSpec(
                        name='objects_flags',
                        kind='uint8',
                        enum_ref='REGN.RDOT.objects_flags',
                        authoring_label='Objects Flags',
                    ),
                    FieldSpec(
                        name='objects_radius_wrt_parent',
                        kind='uint16',
                        authoring_label='Objects Radius wrt Parent',
                    ),
                    FieldSpec(
                        name='objects_radius',
                        kind='uint16',
                        authoring_label='Objects Radius',
                    ),
                    FieldSpec(
                        name='objects_min_height',
                        kind='float32',
                        authoring_label='Objects Min Height',
                    ),
                    FieldSpec(
                        name='objects_max_height',
                        kind='float32',
                        authoring_label='Objects Max Height',
                    ),
                    FieldSpec(
                        name='objects_sink',
                        kind='float32',
                        authoring_label='Objects Sink',
                    ),
                    FieldSpec(
                        name='objects_sink_variance',
                        kind='float32',
                        authoring_label='Objects Sink Variance',
                    ),
                    FieldSpec(
                        name='objects_size_variance',
                        kind='float32',
                        authoring_label='Objects Size Variance',
                    ),
                    FieldSpec(
                        name='objects_angle_variance_x',
                        kind='uint16',
                        authoring_label='Objects Angle Variance X',
                    ),
                    FieldSpec(
                        name='objects_angle_variance_y',
                        kind='uint16',
                        authoring_label='Objects Angle Variance Y',
                    ),
                    FieldSpec(
                        name='objects_angle_variance_z',
                        kind='uint16',
                        authoring_label='Objects Angle Variance Z',
                    ),
                    FieldSpec(
                        name='unknown_u8_19',
                        kind='uint8',
                        authoring_label='Objects Unknown Byte 20',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='unknown_u8_20',
                        kind='uint8',
                        authoring_label='Objects Unknown Byte 21',
                        notes='wbUnused(2)',
                    ),
                    FieldSpec(
                        name='unknown_u8_21',
                        kind='uint8',
                        authoring_label='Objects Unknown Byte 22',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='unknown_u8_22',
                        kind='uint8',
                        authoring_label='Objects Unknown Byte 23',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='unknown_u8_23',
                        kind='uint8',
                        authoring_label='Objects Unknown Byte 24',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='unknown_u8_24',
                        kind='uint8',
                        authoring_label='Objects Unknown Byte 25',
                        notes='wbUnused(4)',
                    ),
                ),
                repeatable=True,
                array=ArraySpec(layout='row_array', element_codec='I,H,B,B,f,B,B,B,B,H,H,f,f,f,f,f,H,H,H,B,B,B,B,B,B'),
                row_label='Objects',
                scope_id='region_data_entries',
            ),
            SubrecordSpec(
                sig='RDMP',
                kind=FieldKind.PARSED,
                display_label='Map Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='map_name',
                        kind='zstring',
                        authoring_label='Map Name',
                    ),
                ),
                repeatable=True,
                scope_id='region_data_entries',
            ),
            SubrecordSpec(
                sig='RDGS',
                kind=FieldKind.PARSED,
                display_label='Grasses',
                codec='array_struct:I,B,B,B,B',
                fields=(
                    FieldSpec(
                        name='grasses_grass',
                        kind='formid',
                        formlink_target='GRAS',
                        formlink_targets=('GRAS',),
                        authoring_label='Grasses Grass',
                    ),
                    FieldSpec(
                        name='unknown_u8_1',
                        kind='uint8',
                        authoring_label='Grasses Unknown Byte 2',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Grasses Unknown Byte 3',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Grasses Unknown Byte 4',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='unknown_u8_4',
                        kind='uint8',
                        authoring_label='Grasses Unknown Byte 5',
                        notes='wbUnused(4)',
                    ),
                ),
                repeatable=True,
                array=ArraySpec(layout='row_array', element_codec='I,B,B,B,B'),
                row_label='Grasses',
                scope_id='region_data_entries',
            ),
            SubrecordSpec(
                sig='RDMD',
                kind=FieldKind.PARSED,
                display_label='Music Type',
                codec='uint32',
                fields=(
                    FieldSpec(
                        name='music_type',
                        kind='uint32',
                        enum_ref='music_enum',
                        authoring_label='Music Type',
                    ),
                ),
                repeatable=True,
                enum_ref='music_enum',
                scope_id='region_data_entries',
            ),
            SubrecordSpec(
                sig='RDWT',
                kind=FieldKind.PARSED,
                display_label='Weather Types',
                codec='array_struct:I,I',
                fields=(
                    FieldSpec(
                        name='weather_types_weather',
                        kind='formid',
                        formlink_target='WTHR',
                        formlink_targets=('WTHR',),
                        authoring_label='Weather Types Weather',
                    ),
                    FieldSpec(
                        name='weather_types_chance',
                        kind='uint32',
                        authoring_label='Weather Types Chance',
                    ),
                ),
                repeatable=True,
                array=ArraySpec(layout='row_array', element_codec='I,I'),
                row_label='Weather Types',
                scope_id='region_data_entries',
            ),
        ),
        display_label='Region',
        record_flags=RecordFlagsSpec(valid_mask=4192, bits=(RecordFlagBit(bit=6, name='Border Region'),)),
    )

    records['ROAD'] = RecordSpec(
        sig='ROAD',
        subrecords=(
            SubrecordSpec(
                sig='PGRP',
                kind=FieldKind.PARSED,
                display_label='Points',
                codec='array_struct:f,f,f,B,B,B,B',
                fields=(
                    FieldSpec(
                        name='points_x',
                        kind='float32',
                        authoring_label='Points X',
                    ),
                    FieldSpec(
                        name='points_y',
                        kind='float32',
                        authoring_label='Points Y',
                    ),
                    FieldSpec(
                        name='points_z_even_red_orange_odd_blue',
                        kind='float32',
                        authoring_label='Points Z (Even = Red/Orange, Odd = Blue)',
                    ),
                    FieldSpec(
                        name='points_connections',
                        kind='uint8',
                        authoring_label='Points Connections',
                    ),
                    FieldSpec(
                        name='unknown_u8_4',
                        kind='uint8',
                        authoring_label='Points Unknown Byte 5',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_5',
                        kind='uint8',
                        authoring_label='Points Unknown Byte 6',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_6',
                        kind='uint8',
                        authoring_label='Points Unknown Byte 7',
                        notes='wbUnused(3)',
                    ),
                ),
                required=True,
                array=ArraySpec(layout='row_array', element_codec='f,f,f,B,B,B,B'),
                row_label='Points',
            ),
            SubrecordSpec(
                sig='PGRR',
                kind=FieldKind.PARSED,
                display_label='Point-to-Point Connections',
                codec='array_struct:',
                fields=(
                    FieldSpec(
                        name='point',
                        kind='struct',
                        nested_fields=(
                            FieldSpec(
                                name='x',
                                kind='float32',
                                authoring_label='X',
                            ),
                            FieldSpec(
                                name='y',
                                kind='float32',
                                authoring_label='Y',
                            ),
                            FieldSpec(
                                name='z',
                                kind='float32',
                                authoring_label='Z',
                            ),
                        ),
                        array=ArraySpec(layout='row_array', element_codec='f,f,f'),
                        authoring_label='Point',
                    ),
                ),
                required=True,
                array=ArraySpec(layout='row_array'),
                row_label='Point-to-Point Connections',
            ),
        ),
        display_label='Road',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['SBSP'] = RecordSpec(
        sig='SBSP',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='DNAM',
                kind=FieldKind.PARSED,
                display_label='Bounds',
                codec='struct:f,f,f',
                fields=(
                    FieldSpec(
                        name='x',
                        kind='float32',
                        authoring_label='X',
                    ),
                    FieldSpec(
                        name='y',
                        kind='float32',
                        authoring_label='Y',
                    ),
                    FieldSpec(
                        name='z',
                        kind='float32',
                        authoring_label='Z',
                    ),
                ),
                required=True,
            ),
        ),
        display_label='Subspace',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['SCPT'] = RecordSpec(
        sig='SCPT',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='SCHD',
                kind=FieldKind.PARSED,
                display_label='Unknown (Script Header?)',
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='unknown_script_header',
                        kind='bytes',
                        authoring_label='Unknown (Script Header?)',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='SCHR',
                kind=FieldKind.PARSED,
                display_label='Basic Script Data',
                codec='struct:B,B,B,B,I,I,I,I',
                fields=(
                    FieldSpec(
                        name='unknown_u8_0',
                        kind='uint8',
                        authoring_label='Unknown Byte 1',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='unknown_u8_1',
                        kind='uint8',
                        authoring_label='Unknown Byte 2',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Unknown Byte 3',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Unknown Byte 4',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='refcount',
                        kind='uint32',
                        authoring_label='RefCount',
                    ),
                    FieldSpec(
                        name='compiledsize',
                        kind='uint32',
                        authoring_label='CompiledSize',
                    ),
                    FieldSpec(
                        name='variablecount',
                        kind='uint32',
                        authoring_label='VariableCount',
                    ),
                    FieldSpec(
                        name='type',
                        kind='uint32',
                        enum_ref='SCPT.SCHR.type',
                        authoring_label='Type',
                    ),
                ),
                repeatable=True,
            ),
            SubrecordSpec(
                sig='SCHD',
                kind=FieldKind.PARSED,
                display_label='Basic Script Data',
                codec='struct:B,B,B,B,I,I,I,I',
                fields=(
                    FieldSpec(
                        name='unknown_u8_0',
                        kind='uint8',
                        authoring_label='Unknown Byte 1',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='unknown_u8_1',
                        kind='uint8',
                        authoring_label='Unknown Byte 2',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Unknown Byte 3',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Unknown Byte 4',
                        notes='wbUnused(4)',
                    ),
                    FieldSpec(
                        name='refcount',
                        kind='uint32',
                        authoring_label='RefCount',
                    ),
                    FieldSpec(
                        name='compiledsize',
                        kind='uint32',
                        authoring_label='CompiledSize',
                    ),
                    FieldSpec(
                        name='variablecount',
                        kind='uint32',
                        authoring_label='VariableCount',
                    ),
                    FieldSpec(
                        name='type',
                        kind='uint32',
                        enum_ref='SCPT.SCHD.type',
                        authoring_label='Type',
                    ),
                    FieldSpec(
                        name='unknown',
                        kind='bytes',
                        authoring_label='Unknown',
                    ),
                ),
                repeatable=True,
            ),
            SubrecordSpec(
                sig='SCDA',
                kind=FieldKind.PARSED,
                display_label='Compiled Script',
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='compiled_script',
                        kind='bytes',
                        authoring_label='Compiled Script',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(sig='SCTX', kind=FieldKind.RAW, display_label='Script Source', required=True),
            SubrecordSpec(
                sig='SLSD',
                kind=FieldKind.PARSED,
                codec='struct:I,B,B,B,B,B,B,B,B,B,B,B,B,B',
                fields=(
                    FieldSpec(
                        name='index',
                        kind='uint32',
                        authoring_label='Index',
                    ),
                    FieldSpec(
                        name='unknown_u8_1',
                        kind='uint8',
                        authoring_label='Unknown Byte 2',
                        notes='wbUnused(12)',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Unknown Byte 3',
                        notes='wbUnused(12)',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Unknown Byte 4',
                        notes='wbUnused(12)',
                    ),
                    FieldSpec(
                        name='unknown_u8_4',
                        kind='uint8',
                        authoring_label='Unknown Byte 5',
                        notes='wbUnused(12)',
                    ),
                    FieldSpec(
                        name='unknown_u8_5',
                        kind='uint8',
                        authoring_label='Unknown Byte 6',
                        notes='wbUnused(12)',
                    ),
                    FieldSpec(
                        name='unknown_u8_6',
                        kind='uint8',
                        authoring_label='Unknown Byte 7',
                        notes='wbUnused(12)',
                    ),
                    FieldSpec(
                        name='unknown_u8_7',
                        kind='uint8',
                        authoring_label='Unknown Byte 8',
                        notes='wbUnused(12)',
                    ),
                    FieldSpec(
                        name='unknown_u8_8',
                        kind='uint8',
                        authoring_label='Unknown Byte 9',
                        notes='wbUnused(12)',
                    ),
                    FieldSpec(
                        name='unknown_u8_9',
                        kind='uint8',
                        authoring_label='Unknown Byte 10',
                        notes='wbUnused(12)',
                    ),
                    FieldSpec(
                        name='unknown_u8_10',
                        kind='uint8',
                        authoring_label='Unknown Byte 11',
                        notes='wbUnused(12)',
                    ),
                    FieldSpec(
                        name='unknown_u8_11',
                        kind='uint8',
                        authoring_label='Unknown Byte 12',
                        notes='wbUnused(12)',
                    ),
                    FieldSpec(
                        name='unknown_u8_12',
                        kind='uint8',
                        authoring_label='Unknown Byte 13',
                        notes='wbUnused(12)',
                    ),
                    FieldSpec(
                        name='islongorshort',
                        kind='uint8',
                        enum_ref='bool_enum',
                        authoring_label='IsLongOrShort',
                    ),
                    FieldSpec(
                        name='unused',
                        kind='bytes',
                        authoring_label='Unused',
                    ),
                ),
                repeatable=True,
                scope_id='local_variables',
            ),
            SubrecordSpec(
                sig='SCVR',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
                repeatable=True,
                scope_id='local_variables',
            ),
            SubrecordSpec(
                sig='SCRO',
                kind=FieldKind.PARSED,
                display_label='Global Reference',
                codec='formid',
                fields=(
                    FieldSpec(name='formid_0', kind='formid'),
                ),
                repeatable=True,
                scope_id='references',
            ),
            SubrecordSpec(
                sig='SCRV',
                kind=FieldKind.PARSED,
                display_label='Local Variable',
                codec='uint32',
                fields=(
                    FieldSpec(
                        name='local_variable',
                        kind='uint32',
                        authoring_label='Local Variable',
                    ),
                ),
                repeatable=True,
                scope_id='references',
            ),
        ),
        display_label='Script',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['SGST'] = RecordSpec(
        sig='SGST',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='model_filename',
                        kind='zstring',
                        authoring_label='Model FileName',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='ICON',
                kind=FieldKind.PARSED,
                display_label='Icon FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='icon_filename',
                        kind='zstring',
                        authoring_label='Icon FileName',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='SCRI',
                kind=FieldKind.PARSED,
                display_label='Script',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='script',
                        kind='formid',
                        formlink_target='SCPT',
                        formlink_targets=('SCPT',),
                        authoring_label='Script',
                    ),
                ),
                formlink_target='SCPT',
                formlink_targets=('SCPT',),
            ),
            SubrecordSpec(
                sig='EFID',
                kind=FieldKind.PARSED,
                display_label='Magic Effect Name',
                codec='uint32',
                fields=(
                    FieldSpec(
                        name='magic_effect_name',
                        kind='uint32',
                        authoring_label='Magic Effect Name',
                    ),
                ),
                repeatable=True,
                required=True,
                scope_id='effects',
            ),
            SubrecordSpec(
                sig='EFIT',
                kind=FieldKind.PARSED,
                codec='struct:I,I,I,I,I,i',
                fields=(
                    FieldSpec(
                        name='magic_effect_name',
                        kind='uint32',
                        authoring_label='Magic Effect Name',
                    ),
                    FieldSpec(
                        name='magnitude',
                        kind='uint32',
                        authoring_label='Magnitude',
                    ),
                    FieldSpec(
                        name='area',
                        kind='uint32',
                        authoring_label='Area',
                    ),
                    FieldSpec(
                        name='duration',
                        kind='uint32',
                        authoring_label='Duration',
                    ),
                    FieldSpec(
                        name='type',
                        kind='uint32',
                        enum_ref='effect_type_enum',
                        authoring_label='Type',
                    ),
                    FieldSpec(
                        name='actor_value',
                        kind='int32',
                        enum_ref='actor_value_enum',
                        authoring_label='Actor Value',
                    ),
                ),
                repeatable=True,
                required=True,
                scope_id='effects',
            ),
            SubrecordSpec(
                sig='SCIT',
                kind=FieldKind.PARSED_WITH_RAW_FALLBACK,
                codec='struct:I,I,I,B,B,B,B',
                fields=(
                    FieldSpec(
                        name='script_effect',
                        kind='formid',
                        formlink_target='SCPT',
                        formlink_targets=('SCPT',),
                        null_allowed=True,
                        authoring_label='Script effect',
                    ),
                    FieldSpec(
                        name='magic_school',
                        kind='uint32',
                        enum_ref='magic_school_enum',
                        authoring_label='Magic school',
                    ),
                    FieldSpec(
                        name='visual_effect_name',
                        kind='uint32',
                        authoring_label='Visual effect name',
                    ),
                    FieldSpec(
                        name='hostile',
                        kind='uint8',
                        enum_ref='bool_enum',
                        authoring_label='Hostile',
                    ),
                    FieldSpec(
                        name='unknown_u8_4',
                        kind='uint8',
                        authoring_label='Unknown Byte 5',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_5',
                        kind='uint8',
                        authoring_label='Unknown Byte 6',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_6',
                        kind='uint8',
                        authoring_label='Unknown Byte 7',
                        notes='wbUnused(3)',
                    ),
                ),
                repeatable=True,
                required=True,
                scope_id='effects',
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
                repeatable=True,
                required=True,
                scope_id='effects',
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                display_label='Data',
                codec='struct:B,I,f',
                fields=(
                    FieldSpec(
                        name='uses',
                        kind='uint8',
                        authoring_label='Uses ',
                    ),
                    FieldSpec(
                        name='value',
                        kind='uint32',
                        authoring_label='Value',
                    ),
                    FieldSpec(
                        name='weight',
                        kind='float32',
                        authoring_label='Weight',
                    ),
                ),
                required=True,
            ),
        ),
        display_label='Sigil Stone',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['SKIL'] = RecordSpec(
        sig='SKIL',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='INDX',
                kind=FieldKind.PARSED,
                display_label='Skill',
                codec='int32',
                fields=(
                    FieldSpec(
                        name='skill',
                        kind='int32',
                        enum_ref='major_skill_enum',
                        authoring_label='Skill',
                    ),
                ),
                required=True,
                enum_ref='major_skill_enum',
            ),
            SubrecordSpec(
                sig='DESC',
                kind=FieldKind.PARSED,
                display_label='Description',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='description',
                        kind='zstring',
                        authoring_label='Description',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='ICON',
                kind=FieldKind.PARSED,
                display_label='Icon FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='icon_filename',
                        kind='zstring',
                        authoring_label='Icon FileName',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                display_label='Skill Data',
                codec='struct:i,I,I',
                fields=(
                    FieldSpec(
                        name='action',
                        kind='int32',
                        enum_ref='major_skill_enum',
                        authoring_label='Action',
                    ),
                    FieldSpec(
                        name='attribute',
                        kind='uint32',
                        enum_ref='attribute_enum',
                        authoring_label='Attribute',
                    ),
                    FieldSpec(
                        name='specialization',
                        kind='uint32',
                        enum_ref='specialization_enum',
                        authoring_label='Specialization',
                    ),
                    FieldSpec(
                        name='use_values',
                        kind='float32',
                        array=ArraySpec(layout='row_array', element_codec='f'),
                        authoring_label='Use Values',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='ANAM',
                kind=FieldKind.PARSED,
                display_label='Apprentice Text',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='apprentice_text',
                        kind='zstring',
                        authoring_label='Apprentice Text',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='JNAM',
                kind=FieldKind.PARSED,
                display_label='Journeyman Text',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='journeyman_text',
                        kind='zstring',
                        authoring_label='Journeyman Text',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='ENAM',
                kind=FieldKind.PARSED,
                display_label='Expert Text',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='expert_text',
                        kind='zstring',
                        authoring_label='Expert Text',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='MNAM',
                kind=FieldKind.PARSED,
                display_label='Master Text',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='master_text',
                        kind='zstring',
                        authoring_label='Master Text',
                    ),
                ),
                required=True,
            ),
        ),
        display_label='Skill',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['SLGM'] = RecordSpec(
        sig='SLGM',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='model_filename',
                        kind='zstring',
                        authoring_label='Model FileName',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='ICON',
                kind=FieldKind.PARSED,
                display_label='Icon FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='icon_filename',
                        kind='zstring',
                        authoring_label='Icon FileName',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='SCRI',
                kind=FieldKind.PARSED,
                display_label='Script',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='script',
                        kind='formid',
                        formlink_target='SCPT',
                        formlink_targets=('SCPT',),
                        authoring_label='Script',
                    ),
                ),
                formlink_target='SCPT',
                formlink_targets=('SCPT',),
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                display_label='Data',
                codec='struct:I,f',
                fields=(
                    FieldSpec(
                        name='value',
                        kind='uint32',
                        authoring_label='Value',
                    ),
                    FieldSpec(
                        name='weight',
                        kind='float32',
                        authoring_label='Weight',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='SOUL',
                kind=FieldKind.PARSED,
                display_label='Contained Soul',
                codec='uint8',
                fields=(
                    FieldSpec(
                        name='contained_soul',
                        kind='uint8',
                        enum_ref='soul_gem_enum',
                        authoring_label='Contained Soul',
                    ),
                ),
                required=True,
                enum_ref='soul_gem_enum',
            ),
            SubrecordSpec(
                sig='SLCP',
                kind=FieldKind.PARSED,
                display_label='Maximum Capacity',
                codec='uint8',
                fields=(
                    FieldSpec(
                        name='maximum_capacity',
                        kind='uint8',
                        enum_ref='soul_gem_enum',
                        authoring_label='Maximum Capacity',
                    ),
                ),
                required=True,
                enum_ref='soul_gem_enum',
            ),
        ),
        display_label='Soul Gem',
        record_flags=RecordFlagsSpec(valid_mask=5152, bits=(RecordFlagBit(bit=10, name='Quest Item'),)),
    )

    records['SOUN'] = RecordSpec(
        sig='SOUN',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FNAM',
                kind=FieldKind.PARSED,
                display_label='Sound Filename',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='sound_filename',
                        kind='zstring',
                        authoring_label='Sound Filename',
                    ),
                ),
            ),
            SubrecordSpec(sig='SNDX', kind=FieldKind.RAW, display_label='Sound Data', required=True),
            SubrecordSpec(sig='SNDD', kind=FieldKind.RAW, display_label='Sound Data'),
        ),
        display_label='Sound',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['SPEL'] = RecordSpec(
        sig='SPEL',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='SPIT',
                kind=FieldKind.PARSED,
                display_label='Data',
                codec='struct:B,B,B,B,I,B,B,B,B,B,B,B,B',
                fields=(
                    FieldSpec(
                        name='type',
                        kind='uint8',
                        enum_ref='SPEL.SPIT.type',
                        authoring_label='Type',
                    ),
                    FieldSpec(
                        name='unknown_u8_1',
                        kind='uint8',
                        authoring_label='Unknown Byte 2',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Unknown Byte 3',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Unknown Byte 4',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='cost',
                        kind='uint32',
                        authoring_label='Cost',
                    ),
                    FieldSpec(
                        name='level',
                        kind='uint8',
                        enum_ref='SPEL.SPIT.level',
                        authoring_label='Level',
                    ),
                    FieldSpec(
                        name='unknown_u8_6',
                        kind='uint8',
                        authoring_label='Unknown Byte 7',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_7',
                        kind='uint8',
                        authoring_label='Unknown Byte 8',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_8',
                        kind='uint8',
                        authoring_label='Unknown Byte 9',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='flags',
                        kind='uint8',
                        enum_ref='SPEL.SPIT.flags',
                        authoring_label='Flags',
                    ),
                    FieldSpec(
                        name='unknown_u8_10',
                        kind='uint8',
                        authoring_label='Unknown Byte 11',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_11',
                        kind='uint8',
                        authoring_label='Unknown Byte 12',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_12',
                        kind='uint8',
                        authoring_label='Unknown Byte 13',
                        notes='wbUnused(3)',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='EFID',
                kind=FieldKind.PARSED,
                display_label='Magic Effect Name',
                codec='uint32',
                fields=(
                    FieldSpec(
                        name='magic_effect_name',
                        kind='uint32',
                        authoring_label='Magic Effect Name',
                    ),
                ),
                repeatable=True,
                required=True,
                scope_id='effects',
            ),
            SubrecordSpec(
                sig='EFIT',
                kind=FieldKind.PARSED,
                codec='struct:I,I,I,I,I,i',
                fields=(
                    FieldSpec(
                        name='magic_effect_name',
                        kind='uint32',
                        authoring_label='Magic Effect Name',
                    ),
                    FieldSpec(
                        name='magnitude',
                        kind='uint32',
                        authoring_label='Magnitude',
                    ),
                    FieldSpec(
                        name='area',
                        kind='uint32',
                        authoring_label='Area',
                    ),
                    FieldSpec(
                        name='duration',
                        kind='uint32',
                        authoring_label='Duration',
                    ),
                    FieldSpec(
                        name='type',
                        kind='uint32',
                        enum_ref='effect_type_enum',
                        authoring_label='Type',
                    ),
                    FieldSpec(
                        name='actor_value',
                        kind='int32',
                        enum_ref='actor_value_enum',
                        authoring_label='Actor Value',
                    ),
                ),
                repeatable=True,
                required=True,
                scope_id='effects',
            ),
            SubrecordSpec(
                sig='SCIT',
                kind=FieldKind.PARSED_WITH_RAW_FALLBACK,
                codec='struct:I,I,I,B,B,B,B',
                fields=(
                    FieldSpec(
                        name='script_effect',
                        kind='formid',
                        formlink_target='SCPT',
                        formlink_targets=('SCPT',),
                        null_allowed=True,
                        authoring_label='Script effect',
                    ),
                    FieldSpec(
                        name='magic_school',
                        kind='uint32',
                        enum_ref='magic_school_enum',
                        authoring_label='Magic school',
                    ),
                    FieldSpec(
                        name='visual_effect_name',
                        kind='uint32',
                        authoring_label='Visual effect name',
                    ),
                    FieldSpec(
                        name='hostile',
                        kind='uint8',
                        enum_ref='bool_enum',
                        authoring_label='Hostile',
                    ),
                    FieldSpec(
                        name='unknown_u8_4',
                        kind='uint8',
                        authoring_label='Unknown Byte 5',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_5',
                        kind='uint8',
                        authoring_label='Unknown Byte 6',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_6',
                        kind='uint8',
                        authoring_label='Unknown Byte 7',
                        notes='wbUnused(3)',
                    ),
                ),
                repeatable=True,
                required=True,
                scope_id='effects',
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
                repeatable=True,
                required=True,
                scope_id='effects',
            ),
        ),
        display_label='Spell',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    records['STAT'] = RecordSpec(
        sig='STAT',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='model_filename',
                        kind='zstring',
                        authoring_label='Model FileName',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='DMTL',
                kind=FieldKind.PARSED,
                display_label='Distant Model Texture List',
                codec='array_struct:Q,Q,Q',
                fields=(
                    FieldSpec(
                        name='distant_model_texture_list_file_hash_pc',
                        kind='uint64',
                        authoring_label='Distant Model Texture List File Hash (PC)',
                    ),
                    FieldSpec(
                        name='distant_model_texture_list_file_hash_console',
                        kind='uint64',
                        authoring_label='Distant Model Texture List File Hash (Console)',
                    ),
                    FieldSpec(
                        name='distant_model_texture_list_folder_hash',
                        kind='uint64',
                        authoring_label='Distant Model Texture List Folder Hash',
                    ),
                ),
                array=ArraySpec(layout='row_array', element_codec='Q,Q,Q'),
                row_label='Distant Model Texture List',
            ),
        ),
        display_label='Static',
        record_flags=RecordFlagsSpec(valid_mask=5152, bits=(RecordFlagBit(bit=10, name='Quest Item'),)),
    )

    records['TREE'] = RecordSpec(
        sig='TREE',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='SPT File FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='spt_file_filename',
                        kind='zstring',
                        authoring_label='SPT File FileName',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='ICON',
                kind=FieldKind.PARSED,
                display_label='Leaf Texture',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='leaf_texture',
                        kind='zstring',
                        authoring_label='Leaf Texture',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='SNAM',
                kind=FieldKind.PARSED,
                display_label='SpeedTree Seeds',
                codec='array_struct:I',
                fields=(
                    FieldSpec(
                        name='speedtree_seeds_speedtree_seed',
                        kind='uint32',
                        authoring_label='SpeedTree Seeds SpeedTree Seed',
                    ),
                ),
                array=ArraySpec(layout='row_array', element_codec='I'),
                row_label='SpeedTree Seeds',
            ),
            SubrecordSpec(
                sig='CNAM',
                kind=FieldKind.PARSED,
                display_label='Tree Data',
                codec='struct:f,f,f,f,f,i,f,f',
                fields=(
                    FieldSpec(
                        name='leaf_curvature',
                        kind='float32',
                        authoring_label='Leaf Curvature',
                    ),
                    FieldSpec(
                        name='minimum_leaf_angle',
                        kind='float32',
                        authoring_label='Minimum Leaf Angle',
                    ),
                    FieldSpec(
                        name='maximum_leaf_angle',
                        kind='float32',
                        authoring_label='Maximum Leaf Angle',
                    ),
                    FieldSpec(
                        name='branch_dimming_value',
                        kind='float32',
                        authoring_label='Branch Dimming Value',
                    ),
                    FieldSpec(
                        name='leaf_dimming_value',
                        kind='float32',
                        authoring_label='Leaf Dimming Value',
                    ),
                    FieldSpec(
                        name='shadow_radius',
                        kind='int32',
                        authoring_label='Shadow Radius',
                    ),
                    FieldSpec(
                        name='rock_speed',
                        kind='float32',
                        authoring_label='Rock Speed',
                    ),
                    FieldSpec(
                        name='rustle_speed',
                        kind='float32',
                        authoring_label='Rustle Speed',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='BNAM',
                kind=FieldKind.PARSED,
                display_label='Billboard Dimensions',
                codec='struct:f,f',
                fields=(
                    FieldSpec(
                        name='width',
                        kind='float32',
                        authoring_label='Width',
                    ),
                    FieldSpec(
                        name='height',
                        kind='float32',
                        authoring_label='Height',
                    ),
                ),
                required=True,
            ),
        ),
        display_label='Tree',
        record_flags=RecordFlagsSpec(valid_mask=5152, bits=(RecordFlagBit(bit=10, name='Quest Item'),)),
    )

    records['WATR'] = RecordSpec(
        sig='WATR',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='TNAM',
                kind=FieldKind.PARSED,
                display_label='Texture',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='texture',
                        kind='zstring',
                        authoring_label='Texture',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='ANAM',
                kind=FieldKind.PARSED,
                display_label='Opacity',
                codec='uint8',
                fields=(
                    FieldSpec(
                        name='opacity',
                        kind='uint8',
                        authoring_label='Opacity',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='FNAM',
                kind=FieldKind.PARSED,
                display_label='Flags',
                codec='uint8',
                fields=(
                    FieldSpec(
                        name='flags',
                        kind='uint8',
                        enum_ref='WATR.FNAM.flags',
                        authoring_label='Flags',
                    ),
                ),
                required=True,
                enum_ref='WATR.FNAM.flags',
            ),
            SubrecordSpec(
                sig='MNAM',
                kind=FieldKind.PARSED,
                display_label='Material ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='material_id',
                        kind='zstring',
                        authoring_label='Material ID',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='SNAM',
                kind=FieldKind.PARSED,
                display_label='Sound',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='sound',
                        kind='formid',
                        formlink_target='SOUN',
                        formlink_targets=('SOUN',),
                        authoring_label='Sound',
                    ),
                ),
                formlink_target='SOUN',
                formlink_targets=('SOUN',),
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                display_label='Data',
                codec='struct:f,f,f,f,f,f,f,f,f,f,f,B,B,B,B,B,B,B,B,B,B,B,B,B,B,B,B,f,f,f,f,f,f,f,f,f,f,H',
                fields=(
                    FieldSpec(
                        name='wind_velocity',
                        kind='float32',
                        authoring_label='Wind Velocity',
                    ),
                    FieldSpec(
                        name='wind_direction',
                        kind='float32',
                        authoring_label='Wind Direction',
                    ),
                    FieldSpec(
                        name='wave_amplitude',
                        kind='float32',
                        authoring_label='Wave Amplitude',
                    ),
                    FieldSpec(
                        name='wave_frequency',
                        kind='float32',
                        authoring_label='Wave Frequency',
                    ),
                    FieldSpec(
                        name='sun_power',
                        kind='float32',
                        authoring_label='Sun Power',
                    ),
                    FieldSpec(
                        name='reflectivity_amount',
                        kind='float32',
                        authoring_label='Reflectivity Amount',
                    ),
                    FieldSpec(
                        name='fresnel_amount',
                        kind='float32',
                        authoring_label='Fresnel Amount',
                    ),
                    FieldSpec(
                        name='scroll_x_speed',
                        kind='float32',
                        authoring_label='Scroll X Speed',
                    ),
                    FieldSpec(
                        name='scroll_y_speed',
                        kind='float32',
                        authoring_label='Scroll Y Speed',
                    ),
                    FieldSpec(
                        name='fog_distance_near',
                        kind='float32',
                        authoring_label='Fog Distance Near',
                    ),
                    FieldSpec(
                        name='fog_distance_far',
                        kind='float32',
                        authoring_label='Fog Distance Far',
                    ),
                    FieldSpec(
                        name='shallow_color_red',
                        kind='uint8',
                        authoring_label='Shallow Color Red',
                    ),
                    FieldSpec(
                        name='shallow_color_green',
                        kind='uint8',
                        authoring_label='Shallow Color Green',
                    ),
                    FieldSpec(
                        name='shallow_color_blue',
                        kind='uint8',
                        authoring_label='Shallow Color Blue',
                    ),
                    FieldSpec(
                        name='unknown_u8_14',
                        kind='uint8',
                        authoring_label='Shallow Color Unknown Byte 15',
                        notes='wbUnused(1)',
                    ),
                    FieldSpec(
                        name='deep_color_red',
                        kind='uint8',
                        authoring_label='Deep Color Red',
                    ),
                    FieldSpec(
                        name='deep_color_green',
                        kind='uint8',
                        authoring_label='Deep Color Green',
                    ),
                    FieldSpec(
                        name='deep_color_blue',
                        kind='uint8',
                        authoring_label='Deep Color Blue',
                    ),
                    FieldSpec(
                        name='unknown_u8_18',
                        kind='uint8',
                        authoring_label='Deep Color Unknown Byte 19',
                        notes='wbUnused(1)',
                    ),
                    FieldSpec(
                        name='reflection_color_red',
                        kind='uint8',
                        authoring_label='Reflection Color Red',
                    ),
                    FieldSpec(
                        name='reflection_color_green',
                        kind='uint8',
                        authoring_label='Reflection Color Green',
                    ),
                    FieldSpec(
                        name='reflection_color_blue',
                        kind='uint8',
                        authoring_label='Reflection Color Blue',
                    ),
                    FieldSpec(
                        name='unknown_u8_22',
                        kind='uint8',
                        authoring_label='Reflection Color Unknown Byte 23',
                        notes='wbUnused(1)',
                    ),
                    FieldSpec(
                        name='texture_blend',
                        kind='uint8',
                        authoring_label='Texture Blend',
                    ),
                    FieldSpec(
                        name='unknown_u8_24',
                        kind='uint8',
                        authoring_label='Unknown Byte 25',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_25',
                        kind='uint8',
                        authoring_label='Unknown Byte 26',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_26',
                        kind='uint8',
                        authoring_label='Unknown Byte 27',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='rain_simulator_force',
                        kind='float32',
                        authoring_label='Rain Simulator Force',
                    ),
                    FieldSpec(
                        name='rain_simulator_velocity',
                        kind='float32',
                        authoring_label='Rain Simulator Velocity',
                    ),
                    FieldSpec(
                        name='rain_simulator_falloff',
                        kind='float32',
                        authoring_label='Rain Simulator Falloff',
                    ),
                    FieldSpec(
                        name='rain_simulator_dampner',
                        kind='float32',
                        authoring_label='Rain Simulator Dampner',
                    ),
                    FieldSpec(
                        name='rain_simulator_starting_size',
                        kind='float32',
                        authoring_label='Rain Simulator Starting Size',
                    ),
                    FieldSpec(
                        name='displacement_simulator_force',
                        kind='float32',
                        authoring_label='Displacement Simulator Force',
                    ),
                    FieldSpec(
                        name='displacement_simulator_velocity',
                        kind='float32',
                        authoring_label='Displacement Simulator Velocity',
                    ),
                    FieldSpec(
                        name='displacement_simulator_falloff',
                        kind='float32',
                        authoring_label='Displacement Simulator Falloff',
                    ),
                    FieldSpec(
                        name='displacement_simulator_dampner',
                        kind='float32',
                        authoring_label='Displacement Simulator Dampner',
                    ),
                    FieldSpec(
                        name='displacement_simulator_starting_size',
                        kind='float32',
                        authoring_label='Displacement Simulator Starting Size',
                    ),
                    FieldSpec(
                        name='damage',
                        kind='uint16',
                        authoring_label='Damage',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='GNAM',
                kind=FieldKind.PARSED,
                display_label='Related Waters',
                codec='struct:I,I,I',
                fields=(
                    FieldSpec(
                        name='daytime',
                        kind='formid',
                        formlink_target='WATR',
                        formlink_targets=('WATR',),
                        null_allowed=True,
                        authoring_label='Daytime',
                    ),
                    FieldSpec(
                        name='nighttime',
                        kind='formid',
                        formlink_target='WATR',
                        formlink_targets=('WATR',),
                        null_allowed=True,
                        authoring_label='Nighttime',
                    ),
                    FieldSpec(
                        name='underwater',
                        kind='formid',
                        formlink_target='WATR',
                        formlink_targets=('WATR',),
                        null_allowed=True,
                        authoring_label='Underwater',
                    ),
                ),
                required=True,
            ),
        ),
        display_label='Water',
        record_flags=RecordFlagsSpec(valid_mask=5152, bits=(RecordFlagBit(bit=10, name='Quest Item'),)),
    )

    records['WEAP'] = RecordSpec(
        sig='WEAP',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='model_filename',
                        kind='zstring',
                        authoring_label='Model FileName',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='ICON',
                kind=FieldKind.PARSED,
                display_label='Icon FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='icon_filename',
                        kind='zstring',
                        authoring_label='Icon FileName',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='SCRI',
                kind=FieldKind.PARSED,
                display_label='Script',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='script',
                        kind='formid',
                        formlink_target='SCPT',
                        formlink_targets=('SCPT',),
                        authoring_label='Script',
                    ),
                ),
                formlink_target='SCPT',
                formlink_targets=('SCPT',),
            ),
            SubrecordSpec(
                sig='EITM',
                kind=FieldKind.PARSED,
                display_label='Effect',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='effect',
                        kind='formid',
                        formlink_target='ENCH',
                        formlink_targets=('ENCH',),
                        authoring_label='Effect',
                    ),
                ),
                repeatable=True,
                formlink_target='ENCH',
                formlink_targets=('ENCH',),
                scope_id='enchantment',
            ),
            SubrecordSpec(
                sig='EAMT',
                kind=FieldKind.PARSED,
                display_label='Capacity',
                codec='uint16',
                fields=(
                    FieldSpec(
                        name='capacity',
                        kind='uint16',
                        authoring_label='Capacity',
                    ),
                ),
                repeatable=True,
                scope_id='enchantment',
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                display_label='Data',
                codec='struct:B,B,B,B,f,f,I,I,I,f,H',
                fields=(
                    FieldSpec(
                        name='type',
                        kind='uint8',
                        enum_ref='WEAP.DATA.type',
                        authoring_label='Type',
                    ),
                    FieldSpec(
                        name='unknown_u8_1',
                        kind='uint8',
                        authoring_label='Unknown Byte 2',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_2',
                        kind='uint8',
                        authoring_label='Unknown Byte 3',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='unknown_u8_3',
                        kind='uint8',
                        authoring_label='Unknown Byte 4',
                        notes='wbUnused(3)',
                    ),
                    FieldSpec(
                        name='speed',
                        kind='float32',
                        authoring_label='Speed',
                    ),
                    FieldSpec(
                        name='reach',
                        kind='float32',
                        authoring_label='Reach',
                    ),
                    FieldSpec(
                        name='ignores_normal_weapon_resistance',
                        kind='uint32',
                        enum_ref='bool_enum',
                        authoring_label='Ignores Normal Weapon Resistance',
                    ),
                    FieldSpec(
                        name='value',
                        kind='uint32',
                        authoring_label='Value',
                    ),
                    FieldSpec(
                        name='health',
                        kind='uint32',
                        authoring_label='Health',
                    ),
                    FieldSpec(
                        name='weight',
                        kind='float32',
                        authoring_label='Weight',
                    ),
                    FieldSpec(
                        name='damage',
                        kind='uint16',
                        authoring_label='Damage',
                    ),
                ),
                required=True,
            ),
        ),
        display_label='Weapon',
        record_flags=RecordFlagsSpec(valid_mask=5152, bits=(RecordFlagBit(bit=10, name='Quest Item'),)),
    )

    records['WRLD'] = RecordSpec(
        sig='WRLD',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='FULL',
                kind=FieldKind.PARSED,
                display_label='Name',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='name',
                        kind='zstring',
                        authoring_label='Name',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='WNAM',
                kind=FieldKind.PARSED,
                display_label='Parent Worldspace',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='parent_worldspace',
                        kind='formid',
                        formlink_target='WRLD',
                        formlink_targets=('WRLD',),
                        authoring_label='Parent Worldspace',
                    ),
                ),
                formlink_target='WRLD',
                formlink_targets=('WRLD',),
            ),
            SubrecordSpec(
                sig='CNAM',
                kind=FieldKind.PARSED,
                display_label='Climate',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='climate',
                        kind='formid',
                        formlink_target='CLMT',
                        formlink_targets=('CLMT',),
                        authoring_label='Climate',
                    ),
                ),
                formlink_target='CLMT',
                formlink_targets=('CLMT',),
            ),
            SubrecordSpec(
                sig='NAM2',
                kind=FieldKind.PARSED,
                display_label='Water',
                codec='formid',
                fields=(
                    FieldSpec(
                        name='water',
                        kind='formid',
                        formlink_target='WATR',
                        formlink_targets=('WATR',),
                        authoring_label='Water',
                    ),
                ),
                formlink_target='WATR',
                formlink_targets=('WATR',),
            ),
            SubrecordSpec(
                sig='ICON',
                kind=FieldKind.PARSED,
                display_label='Map Image',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='map_image',
                        kind='zstring',
                        authoring_label='Map Image',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MNAM',
                kind=FieldKind.PARSED,
                display_label='World Map Data',
                codec='struct:i,i,h,h,h,h',
                fields=(
                    FieldSpec(
                        name='usable_dimensions_x',
                        kind='int32',
                        authoring_label='Usable Dimensions X',
                    ),
                    FieldSpec(
                        name='usable_dimensions_y',
                        kind='int32',
                        authoring_label='Usable Dimensions Y',
                    ),
                    FieldSpec(
                        name='cell_coordinates_nw_cell_x',
                        kind='int16',
                        authoring_label='Cell Coordinates NW Cell X',
                    ),
                    FieldSpec(
                        name='cell_coordinates_nw_cell_y',
                        kind='int16',
                        authoring_label='Cell Coordinates NW Cell Y',
                    ),
                    FieldSpec(
                        name='cell_coordinates_se_cell_x',
                        kind='int16',
                        authoring_label='Cell Coordinates SE Cell X',
                    ),
                    FieldSpec(
                        name='cell_coordinates_se_cell_y',
                        kind='int16',
                        authoring_label='Cell Coordinates SE Cell Y',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                display_label='Flags',
                codec='uint8',
                fields=(
                    FieldSpec(
                        name='flags',
                        kind='uint8',
                        enum_ref='WRLD.DATA.flags',
                        authoring_label='Flags',
                    ),
                ),
                required=True,
                enum_ref='WRLD.DATA.flags',
            ),
            SubrecordSpec(
                sig='NAM0',
                kind=FieldKind.PARSED,
                display_label='Min',
                codec='struct:f,f',
                fields=(
                    FieldSpec(
                        name='x',
                        kind='float32',
                        authoring_label='X',
                    ),
                    FieldSpec(
                        name='y',
                        kind='float32',
                        authoring_label='Y',
                    ),
                ),
                repeatable=True,
                required=True,
                scope_id='worldspace_bounds',
            ),
            SubrecordSpec(
                sig='NAM9',
                kind=FieldKind.PARSED,
                display_label='Max',
                codec='struct:f,f',
                fields=(
                    FieldSpec(
                        name='x',
                        kind='float32',
                        authoring_label='X',
                    ),
                    FieldSpec(
                        name='y',
                        kind='float32',
                        authoring_label='Y',
                    ),
                ),
                repeatable=True,
                required=True,
                scope_id='worldspace_bounds',
            ),
            SubrecordSpec(
                sig='SNAM',
                kind=FieldKind.PARSED,
                display_label='Music',
                codec='uint32',
                fields=(
                    FieldSpec(
                        name='music',
                        kind='uint32',
                        enum_ref='music_enum',
                        authoring_label='Music',
                    ),
                ),
                enum_ref='music_enum',
            ),
            SubrecordSpec(
                sig='OFST',
                kind=FieldKind.PARSED,
                display_label='Offsets',
                codec='array_struct:',
                fields=(
                    FieldSpec(
                        name='row',
                        kind='uint32',
                        array=ArraySpec(layout='row_array', element_codec='I'),
                        authoring_label='Row',
                    ),
                ),
                array=ArraySpec(layout='row_array'),
                row_label='Offsets',
            ),
        ),
        display_label='Worldspace',
        record_flags=RecordFlagsSpec(valid_mask=528416, bits=(RecordFlagBit(bit=19, name='Can\'t Wait'),)),
    )

    records['WTHR'] = RecordSpec(
        sig='WTHR',
        subrecords=(
            SubrecordSpec(
                sig='EDID',
                kind=FieldKind.PARSED,
                display_label='Editor ID',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='editor_id',
                        kind='zstring',
                        authoring_label='Editor ID',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='CNAM',
                kind=FieldKind.PARSED,
                display_label='Cloud Texture Lower Layer',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='cloud_texture_lower_layer',
                        kind='zstring',
                        authoring_label='Cloud Texture Lower Layer',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='DNAM',
                kind=FieldKind.PARSED,
                display_label='Cloud Texture Upper Layer',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='cloud_texture_upper_layer',
                        kind='zstring',
                        authoring_label='Cloud Texture Upper Layer',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODL',
                kind=FieldKind.PARSED,
                display_label='Precipitation Model FileName',
                codec='zstring',
                fields=(
                    FieldSpec(
                        name='precipitation_model_filename',
                        kind='zstring',
                        authoring_label='Precipitation Model FileName',
                    ),
                ),
            ),
            SubrecordSpec(
                sig='MODB',
                kind=FieldKind.PARSED,
                codec='bytes',
                fields=(
                    FieldSpec(
                        name='model_information',
                        kind='bytes',
                        authoring_label='Model Information',
                    ),
                ),
            ),
            SubrecordSpec(sig='NAM0', kind=FieldKind.RAW, display_label='Weather Colors', required=True),
            SubrecordSpec(sig='FNAM', kind=FieldKind.RAW, display_label='Fog Distance', required=True),
            SubrecordSpec(
                sig='HNAM',
                kind=FieldKind.PARSED,
                display_label='HDR Data',
                codec='struct:f,f,f,f,f,f,f,f,f,f,f,f,f,f',
                fields=(
                    FieldSpec(
                        name='eye_adapt_speed',
                        kind='float32',
                        authoring_label='Eye Adapt Speed',
                    ),
                    FieldSpec(
                        name='blur_radius',
                        kind='float32',
                        authoring_label='Blur Radius',
                    ),
                    FieldSpec(
                        name='blur_passes',
                        kind='float32',
                        authoring_label='Blur Passes',
                    ),
                    FieldSpec(
                        name='emissive_mult',
                        kind='float32',
                        authoring_label='Emissive Mult',
                    ),
                    FieldSpec(
                        name='target_lum',
                        kind='float32',
                        authoring_label='Target LUM',
                    ),
                    FieldSpec(
                        name='upper_lum_clamp',
                        kind='float32',
                        authoring_label='Upper LUM Clamp',
                    ),
                    FieldSpec(
                        name='bright_scale',
                        kind='float32',
                        authoring_label='Bright Scale',
                    ),
                    FieldSpec(
                        name='bright_clamp',
                        kind='float32',
                        authoring_label='Bright Clamp',
                    ),
                    FieldSpec(
                        name='lum_ramp_no_tex',
                        kind='float32',
                        authoring_label='LUM Ramp No Tex',
                    ),
                    FieldSpec(
                        name='lum_ramp_min',
                        kind='float32',
                        authoring_label='LUM Ramp Min',
                    ),
                    FieldSpec(
                        name='lum_ramp_max',
                        kind='float32',
                        authoring_label='LUM Ramp Max',
                    ),
                    FieldSpec(
                        name='sunlight_dimmer',
                        kind='float32',
                        authoring_label='Sunlight Dimmer',
                    ),
                    FieldSpec(
                        name='grass_dimmer',
                        kind='float32',
                        authoring_label='Grass Dimmer',
                    ),
                    FieldSpec(
                        name='tree_dimmer',
                        kind='float32',
                        authoring_label='Tree Dimmer',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='DATA',
                kind=FieldKind.PARSED,
                display_label='Data',
                codec='struct:B,B,B,B,B,B,B,B,B,B,B,B,B,B,B',
                fields=(
                    FieldSpec(
                        name='wind_speed',
                        kind='uint8',
                        authoring_label='Wind Speed',
                    ),
                    FieldSpec(
                        name='cloud_speed_lower',
                        kind='uint8',
                        authoring_label='Cloud Speed (Lower)',
                    ),
                    FieldSpec(
                        name='cloud_speed_upper',
                        kind='uint8',
                        authoring_label='Cloud Speed (Upper)',
                    ),
                    FieldSpec(
                        name='trans_delta',
                        kind='uint8',
                        authoring_label='Trans Delta',
                    ),
                    FieldSpec(
                        name='sun_glare',
                        kind='uint8',
                        authoring_label='Sun Glare',
                    ),
                    FieldSpec(
                        name='sun_damage',
                        kind='uint8',
                        authoring_label='Sun Damage',
                    ),
                    FieldSpec(
                        name='precipitation_begin_fade_in',
                        kind='uint8',
                        authoring_label='Precipitation - Begin Fade In',
                    ),
                    FieldSpec(
                        name='precipitation_end_fade_out',
                        kind='uint8',
                        authoring_label='Precipitation - End Fade Out',
                    ),
                    FieldSpec(
                        name='thunder_lightning_begin_fade_in',
                        kind='uint8',
                        authoring_label='Thunder/Lightning - Begin Fade In',
                    ),
                    FieldSpec(
                        name='thunder_lightning_end_fade_out',
                        kind='uint8',
                        authoring_label='Thunder/Lightning - End Fade Out',
                    ),
                    FieldSpec(
                        name='thunder_lightning_frequency',
                        kind='uint8',
                        authoring_label='Thunder/Lightning - Frequency',
                    ),
                    FieldSpec(
                        name='flags',
                        kind='uint8',
                        enum_ref='WTHR.DATA.flags',
                        authoring_label='Flags ',
                    ),
                    FieldSpec(
                        name='lightning_color_red',
                        kind='uint8',
                        authoring_label='Lightning Color Red',
                    ),
                    FieldSpec(
                        name='lightning_color_green',
                        kind='uint8',
                        authoring_label='Lightning Color Green',
                    ),
                    FieldSpec(
                        name='lightning_color_blue',
                        kind='uint8',
                        authoring_label='Lightning Color Blue',
                    ),
                ),
                required=True,
            ),
            SubrecordSpec(
                sig='SNAM',
                kind=FieldKind.PARSED,
                display_label='Sound',
                codec='struct:I,I',
                fields=(
                    FieldSpec(
                        name='sound',
                        kind='formid',
                        formlink_targets=('SNDR', 'SOUN'),
                        null_allowed=True,
                        authoring_label='Sound',
                    ),
                    FieldSpec(
                        name='type',
                        kind='uint32',
                        enum_ref='WTHR.SNAM.type',
                        authoring_label='Type',
                    ),
                ),
                repeatable=True,
            ),
        ),
        display_label='Weather',
        record_flags=RecordFlagsSpec(valid_mask=4128),
    )

    return GameSchema(
        game='oblivion',
        records=records,
        header_version=1.0,
        localized_support=False,
        enums=enums,
    )
