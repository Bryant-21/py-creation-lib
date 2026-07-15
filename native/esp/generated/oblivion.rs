// Auto-generated schema_forge Rust schema for oblivion.
// DO NOT EDIT BY HAND.
// Regenerate via `uv run python -m tools.schema_forge export --source forge --output-format rust --game <game>`.

pub const GAME: &str = "oblivion";
pub const AUTHORING_SCHEMA_JSON: &str = r#"{
  "game": "oblivion",
  "header_version": 1.0,
  "localized_support": false,
  "records": [
    {
      "id": "ACHR",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "NAME",
          "kind": "parsed",
          "display_label": "Base",
          "codec": "formid",
          "fields": [
            {
              "id": "base",
              "kind": "formid",
              "display_label": "Base",
              "formlink_target": "NPC_",
              "formlink_targets": [
                "NPC_"
              ]
            }
          ],
          "required": true,
          "formlink_target": "NPC_",
          "formlink_targets": [
            "NPC_"
          ]
        },
        {
          "id": "XPCI",
          "kind": "parsed",
          "display_label": "Unused",
          "codec": "formid",
          "fields": [
            {
              "id": "unused",
              "kind": "formid",
              "display_label": "Unused",
              "formlink_target": "CELL",
              "formlink_targets": [
                "CELL"
              ]
            }
          ],
          "repeatable": true,
          "formlink_target": "CELL",
          "formlink_targets": [
            "CELL"
          ],
          "authoring_layout": "row_group",
          "authoring_key": "group_unused",
          "scope_id": "unused"
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Unused",
          "codec": "zstring",
          "fields": [
            {
              "id": "unused",
              "kind": "zstring",
              "display_label": "Unused"
            }
          ],
          "repeatable": true,
          "authoring_layout": "row_group",
          "authoring_key": "group_unused",
          "scope_id": "unused"
        },
        {
          "id": "XLOD",
          "kind": "parsed",
          "display_label": "Distant LOD Data",
          "codec": "array_struct:f",
          "fields": [
            {
              "id": "distant_lod_data_unknown",
              "kind": "float32",
              "display_label": "Distant LOD Data Unknown"
            }
          ],
          "array": {
            "layout": "row_array",
            "element_codec": "f"
          },
          "row_label": "Distant LOD Data"
        },
        {
          "id": "XESP",
          "kind": "parsed",
          "display_label": "Enable Parent",
          "codec": "struct:I,B,B,B,B",
          "fields": [
            {
              "id": "reference",
              "kind": "formid",
              "display_label": "Reference",
              "formlink_targets": [
                "ACHR",
                "ACRE",
                "PLYR",
                "REFR"
              ]
            },
            {
              "id": "set_enable_state_to_opposite_of_parent",
              "kind": "uint8",
              "display_label": "Set Enable State To Opposite Of Parent",
              "enum_ref": "bool_enum"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Unknown Byte 3"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Unknown Byte 4"
            },
            {
              "id": "unknown_u8_4",
              "kind": "uint8",
              "display_label": "Unknown Byte 5"
            }
          ]
        },
        {
          "id": "XMRC",
          "kind": "parsed",
          "display_label": "Merchant container",
          "codec": "formid",
          "fields": [
            {
              "id": "merchant_container",
              "kind": "formid",
              "display_label": "Merchant container",
              "formlink_target": "REFR",
              "formlink_targets": [
                "REFR"
              ]
            }
          ],
          "formlink_target": "REFR",
          "formlink_targets": [
            "REFR"
          ]
        },
        {
          "id": "XHRS",
          "kind": "parsed",
          "display_label": "Horse",
          "codec": "formid",
          "fields": [
            {
              "id": "horse",
              "kind": "formid",
              "display_label": "Horse",
              "formlink_target": "ACRE",
              "formlink_targets": [
                "ACRE"
              ]
            }
          ],
          "formlink_target": "ACRE",
          "formlink_targets": [
            "ACRE"
          ]
        },
        {
          "id": "XRGD",
          "kind": "parsed",
          "display_label": "Bones",
          "codec": "array_struct:B,B,B,B,f,f,f,f,f,f",
          "fields": [
            {
              "id": "bones_bone_id",
              "kind": "uint8",
              "display_label": "Bones Bone Id"
            },
            {
              "id": "unknown_u8_1",
              "kind": "uint8",
              "display_label": "Bones Unknown Byte 2"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Bones Unknown Byte 3"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Bones Unknown Byte 4"
            },
            {
              "id": "bones_position_x",
              "kind": "float32",
              "display_label": "Bones Position X"
            },
            {
              "id": "bones_position_y",
              "kind": "float32",
              "display_label": "Bones Position Y"
            },
            {
              "id": "bones_position_z",
              "kind": "float32",
              "display_label": "Bones Position Z"
            },
            {
              "id": "bones_rotation_x",
              "kind": "float32",
              "display_label": "Bones Rotation X"
            },
            {
              "id": "bones_rotation_y",
              "kind": "float32",
              "display_label": "Bones Rotation Y"
            },
            {
              "id": "bones_rotation_z",
              "kind": "float32",
              "display_label": "Bones Rotation Z"
            }
          ],
          "repeatable": true,
          "array": {
            "layout": "row_array",
            "element_codec": "B,B,B,B,f,f,f,f,f,f"
          },
          "row_label": "Bones",
          "scope_id": "ragdoll_data"
        },
        {
          "id": "XSCL",
          "kind": "parsed",
          "display_label": "Scale",
          "codec": "float32",
          "fields": [
            {
              "id": "float32_0",
              "kind": "float32"
            }
          ]
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "codec": "struct:f,f,f,f,f,f",
          "fields": [
            {
              "id": "position_rotation_position_x",
              "kind": "float32",
              "display_label": "Position Rotation Position X"
            },
            {
              "id": "position_rotation_position_y",
              "kind": "float32",
              "display_label": "Position Rotation Position Y"
            },
            {
              "id": "position_rotation_position_z",
              "kind": "float32",
              "display_label": "Position Rotation Position Z"
            },
            {
              "id": "position_rotation_rotation_x",
              "kind": "float32",
              "display_label": "Position Rotation Rotation X"
            },
            {
              "id": "position_rotation_rotation_y",
              "kind": "float32",
              "display_label": "Position Rotation Rotation Y"
            },
            {
              "id": "position_rotation_rotation_z",
              "kind": "float32",
              "display_label": "Position Rotation Rotation Z"
            }
          ],
          "required": true
        }
      ],
      "display_label": "Placed NPC",
      "record_flags": {
        "valid_mask": 39968,
        "bits": [
          {
            "bit": 10,
            "name": "Persistent"
          },
          {
            "bit": 11,
            "name": "Initially Disabled"
          },
          {
            "bit": 15,
            "name": "Visible When Distant"
          }
        ]
      }
    },
    {
      "id": "ACRE",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "NAME",
          "kind": "parsed",
          "display_label": "Base",
          "codec": "formid",
          "fields": [
            {
              "id": "base",
              "kind": "formid",
              "display_label": "Base",
              "formlink_target": "CREA",
              "formlink_targets": [
                "CREA"
              ]
            }
          ],
          "required": true,
          "formlink_target": "CREA",
          "formlink_targets": [
            "CREA"
          ]
        },
        {
          "id": "XOWN",
          "kind": "parsed",
          "display_label": "Owner",
          "codec": "formid",
          "fields": [
            {
              "id": "owner",
              "kind": "formid",
              "display_label": "Owner",
              "formlink_targets": [
                "FACT",
                "NPC_"
              ]
            }
          ],
          "repeatable": true,
          "formlink_targets": [
            "FACT",
            "NPC_"
          ],
          "scope_id": "ownership"
        },
        {
          "id": "XRNK",
          "kind": "parsed",
          "display_label": "Faction rank",
          "codec": "int32",
          "fields": [
            {
              "id": "faction_rank",
              "kind": "int32",
              "display_label": "Faction rank"
            }
          ],
          "repeatable": true,
          "scope_id": "ownership"
        },
        {
          "id": "XGLB",
          "kind": "parsed",
          "display_label": "Global",
          "codec": "formid",
          "fields": [
            {
              "id": "global",
              "kind": "formid",
              "display_label": "Global",
              "formlink_target": "GLOB",
              "formlink_targets": [
                "GLOB"
              ]
            }
          ],
          "repeatable": true,
          "formlink_target": "GLOB",
          "formlink_targets": [
            "GLOB"
          ],
          "scope_id": "ownership"
        },
        {
          "id": "XRGD",
          "kind": "parsed",
          "display_label": "Bones",
          "codec": "array_struct:B,B,B,B,f,f,f,f,f,f",
          "fields": [
            {
              "id": "bones_bone_id",
              "kind": "uint8",
              "display_label": "Bones Bone Id"
            },
            {
              "id": "unknown_u8_1",
              "kind": "uint8",
              "display_label": "Bones Unknown Byte 2"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Bones Unknown Byte 3"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Bones Unknown Byte 4"
            },
            {
              "id": "bones_position_x",
              "kind": "float32",
              "display_label": "Bones Position X"
            },
            {
              "id": "bones_position_y",
              "kind": "float32",
              "display_label": "Bones Position Y"
            },
            {
              "id": "bones_position_z",
              "kind": "float32",
              "display_label": "Bones Position Z"
            },
            {
              "id": "bones_rotation_x",
              "kind": "float32",
              "display_label": "Bones Rotation X"
            },
            {
              "id": "bones_rotation_y",
              "kind": "float32",
              "display_label": "Bones Rotation Y"
            },
            {
              "id": "bones_rotation_z",
              "kind": "float32",
              "display_label": "Bones Rotation Z"
            }
          ],
          "repeatable": true,
          "array": {
            "layout": "row_array",
            "element_codec": "B,B,B,B,f,f,f,f,f,f"
          },
          "row_label": "Bones",
          "scope_id": "ragdoll_data"
        },
        {
          "id": "XLOD",
          "kind": "parsed",
          "display_label": "Distant LOD Data",
          "codec": "array_struct:f",
          "fields": [
            {
              "id": "distant_lod_data_unknown",
              "kind": "float32",
              "display_label": "Distant LOD Data Unknown"
            }
          ],
          "array": {
            "layout": "row_array",
            "element_codec": "f"
          },
          "row_label": "Distant LOD Data"
        },
        {
          "id": "XESP",
          "kind": "parsed",
          "display_label": "Enable Parent",
          "codec": "struct:I,B,B,B,B",
          "fields": [
            {
              "id": "reference",
              "kind": "formid",
              "display_label": "Reference",
              "formlink_targets": [
                "ACHR",
                "ACRE",
                "PLYR",
                "REFR"
              ]
            },
            {
              "id": "set_enable_state_to_opposite_of_parent",
              "kind": "uint8",
              "display_label": "Set Enable State To Opposite Of Parent",
              "enum_ref": "bool_enum"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Unknown Byte 3"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Unknown Byte 4"
            },
            {
              "id": "unknown_u8_4",
              "kind": "uint8",
              "display_label": "Unknown Byte 5"
            }
          ]
        },
        {
          "id": "XSCL",
          "kind": "parsed",
          "display_label": "Scale",
          "codec": "float32",
          "fields": [
            {
              "id": "float32_0",
              "kind": "float32"
            }
          ]
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "codec": "struct:f,f,f,f,f,f",
          "fields": [
            {
              "id": "position_rotation_position_x",
              "kind": "float32",
              "display_label": "Position Rotation Position X"
            },
            {
              "id": "position_rotation_position_y",
              "kind": "float32",
              "display_label": "Position Rotation Position Y"
            },
            {
              "id": "position_rotation_position_z",
              "kind": "float32",
              "display_label": "Position Rotation Position Z"
            },
            {
              "id": "position_rotation_rotation_x",
              "kind": "float32",
              "display_label": "Position Rotation Rotation X"
            },
            {
              "id": "position_rotation_rotation_y",
              "kind": "float32",
              "display_label": "Position Rotation Rotation Y"
            },
            {
              "id": "position_rotation_rotation_z",
              "kind": "float32",
              "display_label": "Position Rotation Rotation Z"
            }
          ],
          "required": true
        }
      ],
      "display_label": "Placed Creature",
      "record_flags": {
        "valid_mask": 39968,
        "bits": [
          {
            "bit": 10,
            "name": "Persistent"
          },
          {
            "bit": 11,
            "name": "Initially Disabled"
          },
          {
            "bit": 15,
            "name": "Visible When Distant"
          }
        ]
      }
    },
    {
      "id": "ACTI",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "model_filename",
              "kind": "zstring",
              "display_label": "Model FileName"
            }
          ]
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ]
        },
        {
          "id": "SCRI",
          "kind": "parsed",
          "display_label": "Script",
          "codec": "formid",
          "fields": [
            {
              "id": "script",
              "kind": "formid",
              "display_label": "Script",
              "formlink_target": "SCPT",
              "formlink_targets": [
                "SCPT"
              ]
            }
          ],
          "formlink_target": "SCPT",
          "formlink_targets": [
            "SCPT"
          ]
        },
        {
          "id": "SNAM",
          "kind": "parsed",
          "display_label": "Sound",
          "codec": "formid",
          "fields": [
            {
              "id": "sound",
              "kind": "formid",
              "display_label": "Sound",
              "formlink_target": "SOUN",
              "formlink_targets": [
                "SOUN"
              ]
            }
          ],
          "formlink_target": "SOUN",
          "formlink_targets": [
            "SOUN"
          ]
        }
      ],
      "display_label": "Activator",
      "record_flags": {
        "valid_mask": 136224,
        "bits": [
          {
            "bit": 10,
            "name": "Quest Item"
          },
          {
            "bit": 17,
            "name": "Dangerous"
          }
        ]
      }
    },
    {
      "id": "ALCH",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "model_filename",
              "kind": "zstring",
              "display_label": "Model FileName"
            }
          ]
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ]
        },
        {
          "id": "ICON",
          "kind": "parsed",
          "display_label": "Icon FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "icon_filename",
              "kind": "zstring",
              "display_label": "Icon FileName"
            }
          ],
          "required": true
        },
        {
          "id": "SCRI",
          "kind": "parsed",
          "display_label": "Script",
          "codec": "formid",
          "fields": [
            {
              "id": "script",
              "kind": "formid",
              "display_label": "Script",
              "formlink_target": "SCPT",
              "formlink_targets": [
                "SCPT"
              ]
            }
          ],
          "formlink_target": "SCPT",
          "formlink_targets": [
            "SCPT"
          ]
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "display_label": "Weight",
          "codec": "float32",
          "fields": [
            {
              "id": "float32_0",
              "kind": "float32"
            }
          ],
          "required": true
        },
        {
          "id": "ENIT",
          "kind": "parsed",
          "display_label": "Data",
          "codec": "struct:i,B,B,B,B",
          "fields": [
            {
              "id": "value",
              "kind": "int32",
              "display_label": "Value"
            },
            {
              "id": "flags",
              "kind": "uint8",
              "display_label": "Flags",
              "enum_ref": "ALCH.ENIT.flags"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Unknown Byte 3"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Unknown Byte 4"
            },
            {
              "id": "unknown_u8_4",
              "kind": "uint8",
              "display_label": "Unknown Byte 5"
            }
          ],
          "required": true
        },
        {
          "id": "EFID",
          "kind": "parsed",
          "display_label": "Magic Effect Name",
          "codec": "uint32",
          "fields": [
            {
              "id": "magic_effect_name",
              "kind": "uint32",
              "display_label": "Magic Effect Name"
            }
          ],
          "repeatable": true,
          "required": true,
          "scope_id": "effects"
        },
        {
          "id": "EFIT",
          "kind": "parsed",
          "codec": "struct:I,I,I,I,I,i",
          "fields": [
            {
              "id": "magic_effect_name",
              "kind": "uint32",
              "display_label": "Magic Effect Name"
            },
            {
              "id": "magnitude",
              "kind": "uint32",
              "display_label": "Magnitude"
            },
            {
              "id": "area",
              "kind": "uint32",
              "display_label": "Area"
            },
            {
              "id": "duration",
              "kind": "uint32",
              "display_label": "Duration"
            },
            {
              "id": "type",
              "kind": "uint32",
              "display_label": "Type",
              "enum_ref": "effect_type_enum"
            },
            {
              "id": "actor_value",
              "kind": "int32",
              "display_label": "Actor Value",
              "enum_ref": "actor_value_enum"
            }
          ],
          "repeatable": true,
          "required": true,
          "scope_id": "effects"
        },
        {
          "id": "SCIT",
          "kind": "parsed_with_raw_fallback",
          "codec": "struct:I,I,I,B,B,B,B",
          "fields": [
            {
              "id": "script_effect",
              "kind": "formid",
              "display_label": "Script effect",
              "formlink_target": "SCPT",
              "formlink_targets": [
                "SCPT"
              ],
              "null_allowed": true
            },
            {
              "id": "magic_school",
              "kind": "uint32",
              "display_label": "Magic school",
              "enum_ref": "magic_school_enum"
            },
            {
              "id": "visual_effect_name",
              "kind": "uint32",
              "display_label": "Visual effect name"
            },
            {
              "id": "hostile",
              "kind": "uint8",
              "display_label": "Hostile",
              "enum_ref": "bool_enum"
            },
            {
              "id": "unknown_u8_4",
              "kind": "uint8",
              "display_label": "Unknown Byte 5"
            },
            {
              "id": "unknown_u8_5",
              "kind": "uint8",
              "display_label": "Unknown Byte 6"
            },
            {
              "id": "unknown_u8_6",
              "kind": "uint8",
              "display_label": "Unknown Byte 7"
            }
          ],
          "repeatable": true,
          "required": true,
          "scope_id": "effects"
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ],
          "repeatable": true,
          "required": true,
          "scope_id": "effects"
        }
      ],
      "display_label": "Potion",
      "record_flags": {
        "valid_mask": 5152,
        "bits": [
          {
            "bit": 10,
            "name": "Quest Item"
          }
        ]
      }
    },
    {
      "id": "AMMO",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "model_filename",
              "kind": "zstring",
              "display_label": "Model FileName"
            }
          ]
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ]
        },
        {
          "id": "ICON",
          "kind": "parsed",
          "display_label": "Icon FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "icon_filename",
              "kind": "zstring",
              "display_label": "Icon FileName"
            }
          ],
          "required": true
        },
        {
          "id": "EITM",
          "kind": "parsed",
          "display_label": "Effect",
          "codec": "formid",
          "fields": [
            {
              "id": "effect",
              "kind": "formid",
              "display_label": "Effect",
              "formlink_target": "ENCH",
              "formlink_targets": [
                "ENCH"
              ]
            }
          ],
          "repeatable": true,
          "formlink_target": "ENCH",
          "formlink_targets": [
            "ENCH"
          ],
          "scope_id": "enchantment"
        },
        {
          "id": "EAMT",
          "kind": "parsed",
          "display_label": "Capacity",
          "codec": "uint16",
          "fields": [
            {
              "id": "capacity",
              "kind": "uint16",
              "display_label": "Capacity"
            }
          ],
          "repeatable": true,
          "scope_id": "enchantment"
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "display_label": "Data",
          "codec": "struct:f,B,B,B,B,I,f,H",
          "fields": [
            {
              "id": "speed",
              "kind": "float32",
              "display_label": "Speed"
            },
            {
              "id": "ignores_normal_weapon_resistance",
              "kind": "uint8",
              "display_label": "Ignores Normal Weapon Resistance",
              "enum_ref": "bool_enum"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Unknown Byte 3"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Unknown Byte 4"
            },
            {
              "id": "unknown_u8_4",
              "kind": "uint8",
              "display_label": "Unknown Byte 5"
            },
            {
              "id": "value",
              "kind": "uint32",
              "display_label": "Value"
            },
            {
              "id": "weight",
              "kind": "float32",
              "display_label": "Weight"
            },
            {
              "id": "damage",
              "kind": "uint16",
              "display_label": "Damage"
            }
          ],
          "required": true
        }
      ],
      "display_label": "Ammunition",
      "record_flags": {
        "valid_mask": 5152,
        "bits": [
          {
            "bit": 10,
            "name": "Quest Item"
          }
        ]
      }
    },
    {
      "id": "ANIO",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "model_filename",
              "kind": "zstring",
              "display_label": "Model FileName"
            }
          ]
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ]
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "display_label": "Idle Animation",
          "codec": "formid",
          "fields": [
            {
              "id": "idle_animation",
              "kind": "formid",
              "display_label": "Idle Animation",
              "formlink_target": "IDLE",
              "formlink_targets": [
                "IDLE"
              ]
            }
          ],
          "required": true,
          "formlink_target": "IDLE",
          "formlink_targets": [
            "IDLE"
          ]
        }
      ],
      "display_label": "Animated Object",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "APPA",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "model_filename",
              "kind": "zstring",
              "display_label": "Model FileName"
            }
          ]
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ]
        },
        {
          "id": "ICON",
          "kind": "parsed",
          "display_label": "Icon FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "icon_filename",
              "kind": "zstring",
              "display_label": "Icon FileName"
            }
          ],
          "required": true
        },
        {
          "id": "SCRI",
          "kind": "parsed",
          "display_label": "Script",
          "codec": "formid",
          "fields": [
            {
              "id": "script",
              "kind": "formid",
              "display_label": "Script",
              "formlink_target": "SCPT",
              "formlink_targets": [
                "SCPT"
              ]
            }
          ],
          "formlink_target": "SCPT",
          "formlink_targets": [
            "SCPT"
          ]
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "display_label": "Data",
          "codec": "struct:B,I,f,f",
          "fields": [
            {
              "id": "type",
              "kind": "uint8",
              "display_label": "Type",
              "enum_ref": "APPA.DATA.type"
            },
            {
              "id": "value",
              "kind": "uint32",
              "display_label": "Value"
            },
            {
              "id": "weight",
              "kind": "float32",
              "display_label": "Weight"
            },
            {
              "id": "quality",
              "kind": "float32",
              "display_label": "Quality"
            }
          ],
          "required": true
        }
      ],
      "display_label": "Apparatus",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "ARMO",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "SCRI",
          "kind": "parsed",
          "display_label": "Script",
          "codec": "formid",
          "fields": [
            {
              "id": "script",
              "kind": "formid",
              "display_label": "Script",
              "formlink_target": "SCPT",
              "formlink_targets": [
                "SCPT"
              ]
            }
          ],
          "formlink_target": "SCPT",
          "formlink_targets": [
            "SCPT"
          ]
        },
        {
          "id": "EITM",
          "kind": "parsed",
          "display_label": "Effect",
          "codec": "formid",
          "fields": [
            {
              "id": "effect",
              "kind": "formid",
              "display_label": "Effect",
              "formlink_target": "ENCH",
              "formlink_targets": [
                "ENCH"
              ]
            }
          ],
          "repeatable": true,
          "formlink_target": "ENCH",
          "formlink_targets": [
            "ENCH"
          ],
          "scope_id": "enchantment"
        },
        {
          "id": "EAMT",
          "kind": "parsed",
          "display_label": "Capacity",
          "codec": "uint16",
          "fields": [
            {
              "id": "capacity",
              "kind": "uint16",
              "display_label": "Capacity"
            }
          ],
          "repeatable": true,
          "scope_id": "enchantment"
        },
        {
          "id": "BMDT",
          "kind": "parsed",
          "display_label": "Flags",
          "codec": "struct:H,B,B",
          "fields": [
            {
              "id": "biped_flags",
              "kind": "uint16",
              "display_label": "Biped Flags",
              "enum_ref": "biped_flags"
            },
            {
              "id": "general_flags",
              "kind": "uint8",
              "display_label": "General Flags",
              "enum_ref": "ARMO.BMDT.general_flags"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Unknown Byte 3"
            }
          ],
          "required": true
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "Biped Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "biped_model_filename",
              "kind": "zstring",
              "display_label": "Biped Model FileName"
            }
          ],
          "repeatable": true,
          "scope_id": "male"
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ],
          "repeatable": true,
          "scope_id": "male"
        },
        {
          "id": "MOD2",
          "kind": "parsed",
          "display_label": "World Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "world_model_filename",
              "kind": "zstring",
              "display_label": "World Model FileName"
            }
          ],
          "repeatable": true,
          "scope_id": "male"
        },
        {
          "id": "MO2B",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ],
          "repeatable": true,
          "scope_id": "male"
        },
        {
          "id": "ICON",
          "kind": "parsed",
          "display_label": "Icon Image",
          "codec": "zstring",
          "fields": [
            {
              "id": "icon_image",
              "kind": "zstring",
              "display_label": "Icon Image"
            }
          ],
          "repeatable": true,
          "scope_id": "male"
        },
        {
          "id": "MOD3",
          "kind": "parsed",
          "display_label": "Biped Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "biped_model_filename",
              "kind": "zstring",
              "display_label": "Biped Model FileName"
            }
          ],
          "repeatable": true,
          "scope_id": "female"
        },
        {
          "id": "MO3B",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ],
          "repeatable": true,
          "scope_id": "female"
        },
        {
          "id": "MOD4",
          "kind": "parsed",
          "display_label": "World Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "world_model_filename",
              "kind": "zstring",
              "display_label": "World Model FileName"
            }
          ],
          "repeatable": true,
          "scope_id": "female"
        },
        {
          "id": "MO4B",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ],
          "repeatable": true,
          "scope_id": "female"
        },
        {
          "id": "ICO2",
          "kind": "parsed",
          "display_label": "Icon Image",
          "codec": "zstring",
          "fields": [
            {
              "id": "icon_image",
              "kind": "zstring",
              "display_label": "Icon Image"
            }
          ],
          "repeatable": true,
          "scope_id": "female"
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "display_label": "Data",
          "codec": "struct:H,I,I,f",
          "fields": [
            {
              "id": "armor",
              "kind": "uint16",
              "display_label": "Armor"
            },
            {
              "id": "value",
              "kind": "uint32",
              "display_label": "Value"
            },
            {
              "id": "health",
              "kind": "uint32",
              "display_label": "Health"
            },
            {
              "id": "weight",
              "kind": "float32",
              "display_label": "Weight"
            }
          ],
          "required": true
        }
      ],
      "display_label": "Armor",
      "record_flags": {
        "valid_mask": 5152,
        "bits": [
          {
            "bit": 10,
            "name": "Quest Item"
          }
        ]
      }
    },
    {
      "id": "BOOK",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "model_filename",
              "kind": "zstring",
              "display_label": "Model FileName"
            }
          ]
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ]
        },
        {
          "id": "ICON",
          "kind": "parsed",
          "display_label": "Icon FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "icon_filename",
              "kind": "zstring",
              "display_label": "Icon FileName"
            }
          ],
          "required": true
        },
        {
          "id": "SCRI",
          "kind": "parsed",
          "display_label": "Script",
          "codec": "formid",
          "fields": [
            {
              "id": "script",
              "kind": "formid",
              "display_label": "Script",
              "formlink_target": "SCPT",
              "formlink_targets": [
                "SCPT"
              ]
            }
          ],
          "formlink_target": "SCPT",
          "formlink_targets": [
            "SCPT"
          ]
        },
        {
          "id": "EITM",
          "kind": "parsed",
          "display_label": "Effect",
          "codec": "formid",
          "fields": [
            {
              "id": "effect",
              "kind": "formid",
              "display_label": "Effect",
              "formlink_target": "ENCH",
              "formlink_targets": [
                "ENCH"
              ]
            }
          ],
          "repeatable": true,
          "formlink_target": "ENCH",
          "formlink_targets": [
            "ENCH"
          ],
          "scope_id": "enchantment"
        },
        {
          "id": "EAMT",
          "kind": "parsed",
          "display_label": "Capacity",
          "codec": "uint16",
          "fields": [
            {
              "id": "capacity",
              "kind": "uint16",
              "display_label": "Capacity"
            }
          ],
          "repeatable": true,
          "scope_id": "enchantment"
        },
        {
          "id": "DESC",
          "kind": "parsed",
          "display_label": "Description",
          "codec": "zstring",
          "fields": [
            {
              "id": "description",
              "kind": "zstring",
              "display_label": "Description"
            }
          ]
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "display_label": "Data",
          "codec": "struct:B,b,I,f",
          "fields": [
            {
              "id": "flags",
              "kind": "uint8",
              "display_label": "Flags",
              "enum_ref": "BOOK.DATA.flags"
            },
            {
              "id": "teaches",
              "kind": "int8",
              "display_label": "Teaches",
              "enum_ref": "skill_enum"
            },
            {
              "id": "value",
              "kind": "uint32",
              "display_label": "Value"
            },
            {
              "id": "weight",
              "kind": "float32",
              "display_label": "Weight"
            }
          ],
          "required": true
        }
      ],
      "display_label": "Book",
      "record_flags": {
        "valid_mask": 5152,
        "bits": [
          {
            "bit": 10,
            "name": "Quest Item"
          }
        ]
      }
    },
    {
      "id": "BSGN",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "ICON",
          "kind": "parsed",
          "display_label": "Constellation Filename",
          "codec": "zstring",
          "fields": [
            {
              "id": "constellation_filename",
              "kind": "zstring",
              "display_label": "Constellation Filename"
            }
          ]
        },
        {
          "id": "DESC",
          "kind": "parsed",
          "display_label": "Description",
          "codec": "zstring",
          "fields": [
            {
              "id": "description",
              "kind": "zstring",
              "display_label": "Description"
            }
          ]
        },
        {
          "id": "SPLO",
          "kind": "raw",
          "display_label": "Spell",
          "repeatable": true
        }
      ],
      "display_label": "Birthsign",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "CELL",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "display_label": "Flags",
          "codec": "uint8",
          "fields": [
            {
              "id": "flags",
              "kind": "uint8",
              "display_label": "Flags",
              "enum_ref": "CELL.DATA.flags"
            }
          ],
          "required": true,
          "enum_ref": "CELL.DATA.flags"
        },
        {
          "id": "XCLL",
          "kind": "parsed",
          "display_label": "Lighting",
          "codec": "struct:B,B,B,B,B,B,B,B,B,B,B,B,f,f,i,i,f,f",
          "fields": [
            {
              "id": "ambient_color_red",
              "kind": "uint8",
              "display_label": "Ambient Color Red"
            },
            {
              "id": "ambient_color_green",
              "kind": "uint8",
              "display_label": "Ambient Color Green"
            },
            {
              "id": "ambient_color_blue",
              "kind": "uint8",
              "display_label": "Ambient Color Blue"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Ambient Color Unknown Byte 4"
            },
            {
              "id": "directional_color_red",
              "kind": "uint8",
              "display_label": "Directional Color Red"
            },
            {
              "id": "directional_color_green",
              "kind": "uint8",
              "display_label": "Directional Color Green"
            },
            {
              "id": "directional_color_blue",
              "kind": "uint8",
              "display_label": "Directional Color Blue"
            },
            {
              "id": "unknown_u8_7",
              "kind": "uint8",
              "display_label": "Directional Color Unknown Byte 8"
            },
            {
              "id": "fog_color_red",
              "kind": "uint8",
              "display_label": "Fog Color Red"
            },
            {
              "id": "fog_color_green",
              "kind": "uint8",
              "display_label": "Fog Color Green"
            },
            {
              "id": "fog_color_blue",
              "kind": "uint8",
              "display_label": "Fog Color Blue"
            },
            {
              "id": "unknown_u8_11",
              "kind": "uint8",
              "display_label": "Fog Color Unknown Byte 12"
            },
            {
              "id": "fog_near",
              "kind": "float32",
              "display_label": "Fog Near"
            },
            {
              "id": "fog_far",
              "kind": "float32",
              "display_label": "Fog Far"
            },
            {
              "id": "directional_rotation_xy",
              "kind": "int32",
              "display_label": "Directional Rotation XY"
            },
            {
              "id": "directional_rotation_z",
              "kind": "int32",
              "display_label": "Directional Rotation Z"
            },
            {
              "id": "directional_fade",
              "kind": "float32",
              "display_label": "Directional Fade"
            },
            {
              "id": "fog_clip_dist",
              "kind": "float32",
              "display_label": "Fog Clip Dist"
            }
          ]
        },
        {
          "id": "XCLR",
          "kind": "parsed",
          "display_label": "Regions",
          "codec": "formid_array",
          "fields": [
            {
              "id": "regions_region",
              "kind": "formid",
              "display_label": "Regions Region",
              "formlink_target": "REGN",
              "formlink_targets": [
                "REGN"
              ]
            }
          ],
          "formlink_target": "REGN",
          "formlink_targets": [
            "REGN"
          ]
        },
        {
          "id": "XCMT",
          "kind": "parsed",
          "display_label": "Music",
          "codec": "uint8",
          "fields": [
            {
              "id": "music",
              "kind": "uint8",
              "display_label": "Music",
              "enum_ref": "music_enum"
            }
          ],
          "enum_ref": "music_enum"
        },
        {
          "id": "XCLW",
          "kind": "parsed",
          "display_label": "Water Height",
          "codec": "float32",
          "fields": [
            {
              "id": "float32_0",
              "kind": "float32"
            }
          ]
        },
        {
          "id": "XCCM",
          "kind": "parsed",
          "display_label": "Climate",
          "codec": "formid",
          "fields": [
            {
              "id": "climate",
              "kind": "formid",
              "display_label": "Climate",
              "formlink_target": "CLMT",
              "formlink_targets": [
                "CLMT"
              ]
            }
          ],
          "formlink_target": "CLMT",
          "formlink_targets": [
            "CLMT"
          ]
        },
        {
          "id": "XCWT",
          "kind": "parsed",
          "display_label": "Water",
          "codec": "formid",
          "fields": [
            {
              "id": "water",
              "kind": "formid",
              "display_label": "Water",
              "formlink_target": "WATR",
              "formlink_targets": [
                "WATR"
              ]
            }
          ],
          "formlink_target": "WATR",
          "formlink_targets": [
            "WATR"
          ]
        },
        {
          "id": "XCLC",
          "kind": "parsed",
          "display_label": "Grid",
          "codec": "struct:i,i",
          "fields": [
            {
              "id": "x",
              "kind": "int32",
              "display_label": "X"
            },
            {
              "id": "y",
              "kind": "int32",
              "display_label": "Y"
            }
          ]
        },
        {
          "id": "XOWN",
          "kind": "parsed",
          "display_label": "Owner",
          "codec": "formid",
          "fields": [
            {
              "id": "owner",
              "kind": "formid",
              "display_label": "Owner"
            }
          ]
        },
        {
          "id": "XRNK",
          "kind": "parsed",
          "display_label": "Faction rank",
          "codec": "int32",
          "fields": [
            {
              "id": "faction_rank",
              "kind": "int32",
              "display_label": "Faction rank"
            }
          ]
        },
        {
          "id": "XGLB",
          "kind": "parsed",
          "display_label": "Global",
          "codec": "formid",
          "fields": [
            {
              "id": "global",
              "kind": "formid",
              "display_label": "Global"
            }
          ]
        },
        {
          "id": "XTLI",
          "kind": "parsed",
          "display_label": "Threat Level",
          "codec": "uint32",
          "fields": [
            {
              "id": "threat_level",
              "kind": "uint32",
              "display_label": "Threat Level"
            }
          ]
        }
      ],
      "display_label": "Cell",
      "record_flags": {
        "valid_mask": 660512,
        "bits": [
          {
            "bit": 10,
            "name": "Persistent"
          },
          {
            "bit": 17,
            "name": "Off Limits"
          },
          {
            "bit": 19,
            "name": "Can't Wait"
          }
        ]
      }
    },
    {
      "id": "CLAS",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "DESC",
          "kind": "parsed",
          "display_label": "Description",
          "codec": "zstring",
          "fields": [
            {
              "id": "description",
              "kind": "zstring",
              "display_label": "Description"
            }
          ]
        },
        {
          "id": "ICON",
          "kind": "parsed",
          "display_label": "Image Filename",
          "codec": "zstring",
          "fields": [
            {
              "id": "image_filename",
              "kind": "zstring",
              "display_label": "Image Filename"
            }
          ]
        },
        {
          "id": "DATA",
          "kind": "parsed_with_raw_fallback",
          "display_label": "Data",
          "codec": "struct:I,I,I,i,i,i,i,i,i,i,I,I,b,B,H",
          "fields": [
            {
              "id": "primary_attributes_attribute_1",
              "kind": "uint32",
              "display_label": "Primary Attributes Attribute #1",
              "enum_ref": "attribute_enum"
            },
            {
              "id": "primary_attributes_attribute_2",
              "kind": "uint32",
              "display_label": "Primary Attributes Attribute #2",
              "enum_ref": "attribute_enum"
            },
            {
              "id": "specialization",
              "kind": "uint32",
              "display_label": "Specialization",
              "enum_ref": "specialization_enum"
            },
            {
              "id": "major_skills_skill_1",
              "kind": "int32",
              "display_label": "Major Skills Skill #1",
              "enum_ref": "major_skill_enum"
            },
            {
              "id": "major_skills_skill_2",
              "kind": "int32",
              "display_label": "Major Skills Skill #2",
              "enum_ref": "major_skill_enum"
            },
            {
              "id": "major_skills_skill_3",
              "kind": "int32",
              "display_label": "Major Skills Skill #3",
              "enum_ref": "major_skill_enum"
            },
            {
              "id": "major_skills_skill_4",
              "kind": "int32",
              "display_label": "Major Skills Skill #4",
              "enum_ref": "major_skill_enum"
            },
            {
              "id": "major_skills_skill_5",
              "kind": "int32",
              "display_label": "Major Skills Skill #5",
              "enum_ref": "major_skill_enum"
            },
            {
              "id": "major_skills_skill_6",
              "kind": "int32",
              "display_label": "Major Skills Skill #6",
              "enum_ref": "major_skill_enum"
            },
            {
              "id": "major_skills_skill_7",
              "kind": "int32",
              "display_label": "Major Skills Skill #7",
              "enum_ref": "major_skill_enum"
            },
            {
              "id": "flags",
              "kind": "uint32",
              "display_label": "Flags",
              "enum_ref": "CLAS.DATA.flags"
            },
            {
              "id": "buys_sells_and_services",
              "kind": "uint32",
              "display_label": "Buys/Sells and Services",
              "enum_ref": "service_flags"
            },
            {
              "id": "teaches",
              "kind": "int8",
              "display_label": "Teaches",
              "enum_ref": "skill_enum"
            },
            {
              "id": "maximum_training_level",
              "kind": "uint8",
              "display_label": "Maximum training level"
            },
            {
              "id": "unused",
              "kind": "uint16",
              "display_label": "Unused"
            }
          ]
        }
      ],
      "display_label": "Class",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "CLMT",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "WLST",
          "kind": "parsed",
          "display_label": "Weather Types",
          "codec": "array_struct:I,i",
          "fields": [
            {
              "id": "weather_types_weather",
              "kind": "formid",
              "display_label": "Weather Types Weather",
              "formlink_target": "WTHR",
              "formlink_targets": [
                "WTHR"
              ]
            },
            {
              "id": "weather_types_chance",
              "kind": "int32",
              "display_label": "Weather Types Chance"
            }
          ],
          "array": {
            "layout": "row_array",
            "element_codec": "I,i"
          },
          "row_label": "Weather Types"
        },
        {
          "id": "FNAM",
          "kind": "parsed",
          "display_label": "Sun Texture",
          "codec": "zstring",
          "fields": [
            {
              "id": "sun_texture",
              "kind": "zstring",
              "display_label": "Sun Texture"
            }
          ]
        },
        {
          "id": "GNAM",
          "kind": "parsed",
          "display_label": "Sun Glare Texture",
          "codec": "zstring",
          "fields": [
            {
              "id": "sun_glare_texture",
              "kind": "zstring",
              "display_label": "Sun Glare Texture"
            }
          ]
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "model_filename",
              "kind": "zstring",
              "display_label": "Model FileName"
            }
          ]
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ]
        },
        {
          "id": "TNAM",
          "kind": "parsed",
          "display_label": "Timing",
          "codec": "struct:B,B,B,B,B,B",
          "fields": [
            {
              "id": "sunrise_begin",
              "kind": "uint8",
              "display_label": "Sunrise Begin"
            },
            {
              "id": "sunrise_end",
              "kind": "uint8",
              "display_label": "Sunrise End"
            },
            {
              "id": "sunset_begin",
              "kind": "uint8",
              "display_label": "Sunset Begin"
            },
            {
              "id": "sunset_end",
              "kind": "uint8",
              "display_label": "Sunset End"
            },
            {
              "id": "volatility",
              "kind": "uint8",
              "display_label": "Volatility"
            },
            {
              "id": "moons_phase_length",
              "kind": "uint8",
              "display_label": "Moons / Phase Length"
            }
          ],
          "required": true
        }
      ],
      "display_label": "Climate",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "CLOT",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "SCRI",
          "kind": "parsed",
          "display_label": "Script",
          "codec": "formid",
          "fields": [
            {
              "id": "script",
              "kind": "formid",
              "display_label": "Script",
              "formlink_target": "SCPT",
              "formlink_targets": [
                "SCPT"
              ]
            }
          ],
          "formlink_target": "SCPT",
          "formlink_targets": [
            "SCPT"
          ]
        },
        {
          "id": "EITM",
          "kind": "parsed",
          "display_label": "Effect",
          "codec": "formid",
          "fields": [
            {
              "id": "effect",
              "kind": "formid",
              "display_label": "Effect",
              "formlink_target": "ENCH",
              "formlink_targets": [
                "ENCH"
              ]
            }
          ],
          "repeatable": true,
          "formlink_target": "ENCH",
          "formlink_targets": [
            "ENCH"
          ],
          "scope_id": "enchantment"
        },
        {
          "id": "EAMT",
          "kind": "parsed",
          "display_label": "Capacity",
          "codec": "uint16",
          "fields": [
            {
              "id": "capacity",
              "kind": "uint16",
              "display_label": "Capacity"
            }
          ],
          "repeatable": true,
          "scope_id": "enchantment"
        },
        {
          "id": "BMDT",
          "kind": "parsed",
          "display_label": "Flags",
          "codec": "struct:H,B,B",
          "fields": [
            {
              "id": "biped_flags",
              "kind": "uint16",
              "display_label": "Biped Flags",
              "enum_ref": "biped_flags"
            },
            {
              "id": "general_flags",
              "kind": "uint8",
              "display_label": "General Flags",
              "enum_ref": "CLOT.BMDT.general_flags"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Unknown Byte 3"
            }
          ],
          "required": true
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "Biped Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "biped_model_filename",
              "kind": "zstring",
              "display_label": "Biped Model FileName"
            }
          ],
          "repeatable": true,
          "scope_id": "male"
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ],
          "repeatable": true,
          "scope_id": "male"
        },
        {
          "id": "MOD2",
          "kind": "parsed",
          "display_label": "World Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "world_model_filename",
              "kind": "zstring",
              "display_label": "World Model FileName"
            }
          ],
          "repeatable": true,
          "scope_id": "male"
        },
        {
          "id": "MO2B",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ],
          "repeatable": true,
          "scope_id": "male"
        },
        {
          "id": "ICON",
          "kind": "parsed",
          "display_label": "Icon Image",
          "codec": "zstring",
          "fields": [
            {
              "id": "icon_image",
              "kind": "zstring",
              "display_label": "Icon Image"
            }
          ],
          "repeatable": true,
          "scope_id": "male"
        },
        {
          "id": "MOD3",
          "kind": "parsed",
          "display_label": "Biped Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "biped_model_filename",
              "kind": "zstring",
              "display_label": "Biped Model FileName"
            }
          ],
          "repeatable": true,
          "scope_id": "female"
        },
        {
          "id": "MO3B",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ],
          "repeatable": true,
          "scope_id": "female"
        },
        {
          "id": "MOD4",
          "kind": "parsed",
          "display_label": "World Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "world_model_filename",
              "kind": "zstring",
              "display_label": "World Model FileName"
            }
          ],
          "repeatable": true,
          "scope_id": "female"
        },
        {
          "id": "MO4B",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ],
          "repeatable": true,
          "scope_id": "female"
        },
        {
          "id": "ICO2",
          "kind": "parsed",
          "display_label": "Icon Image",
          "codec": "zstring",
          "fields": [
            {
              "id": "icon_image",
              "kind": "zstring",
              "display_label": "Icon Image"
            }
          ],
          "repeatable": true,
          "scope_id": "female"
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "display_label": "Data",
          "codec": "struct:I,f",
          "fields": [
            {
              "id": "value",
              "kind": "uint32",
              "display_label": "Value"
            },
            {
              "id": "weight",
              "kind": "float32",
              "display_label": "Weight"
            }
          ],
          "required": true
        }
      ],
      "display_label": "Clothing",
      "record_flags": {
        "valid_mask": 5152,
        "bits": [
          {
            "bit": 10,
            "name": "Quest Item"
          }
        ]
      }
    },
    {
      "id": "CONT",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "model_filename",
              "kind": "zstring",
              "display_label": "Model FileName"
            }
          ]
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ]
        },
        {
          "id": "SCRI",
          "kind": "parsed",
          "display_label": "Script",
          "codec": "formid",
          "fields": [
            {
              "id": "script",
              "kind": "formid",
              "display_label": "Script",
              "formlink_target": "SCPT",
              "formlink_targets": [
                "SCPT"
              ]
            }
          ],
          "formlink_target": "SCPT",
          "formlink_targets": [
            "SCPT"
          ]
        },
        {
          "id": "CNTO",
          "kind": "parsed",
          "display_label": "Items",
          "codec": "struct:I,i",
          "fields": [
            {
              "id": "item",
              "kind": "formid",
              "display_label": "Item",
              "formlink_targets": [
                "ALCH",
                "AMMO",
                "APPA",
                "ARMO",
                "BOOK",
                "CLOT",
                "INGR",
                "KEYM",
                "LIGH",
                "LVLI",
                "MISC",
                "SGST",
                "SLGM",
                "WEAP"
              ]
            },
            {
              "id": "count",
              "kind": "int32",
              "display_label": "Count"
            }
          ],
          "repeatable": true
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "display_label": "Data",
          "codec": "struct:B,f",
          "fields": [
            {
              "id": "flags",
              "kind": "uint8",
              "display_label": "Flags",
              "enum_ref": "CONT.DATA.flags"
            },
            {
              "id": "weight",
              "kind": "float32",
              "display_label": "Weight"
            }
          ],
          "required": true
        },
        {
          "id": "SNAM",
          "kind": "parsed",
          "display_label": "Open Sound",
          "codec": "formid",
          "fields": [
            {
              "id": "open_sound",
              "kind": "formid",
              "display_label": "Open Sound",
              "formlink_target": "SOUN",
              "formlink_targets": [
                "SOUN"
              ]
            }
          ],
          "formlink_target": "SOUN",
          "formlink_targets": [
            "SOUN"
          ]
        },
        {
          "id": "QNAM",
          "kind": "parsed",
          "display_label": "Close Sound",
          "codec": "formid",
          "fields": [
            {
              "id": "close_sound",
              "kind": "formid",
              "display_label": "Close Sound",
              "formlink_target": "SOUN",
              "formlink_targets": [
                "SOUN"
              ]
            }
          ],
          "formlink_target": "SOUN",
          "formlink_targets": [
            "SOUN"
          ]
        }
      ],
      "display_label": "Container",
      "record_flags": {
        "valid_mask": 5152,
        "bits": [
          {
            "bit": 10,
            "name": "Quest Item"
          }
        ]
      }
    },
    {
      "id": "CREA",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "model_filename",
              "kind": "zstring",
              "display_label": "Model FileName"
            }
          ]
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ]
        },
        {
          "id": "CNTO",
          "kind": "parsed",
          "display_label": "Items",
          "codec": "struct:I,i",
          "fields": [
            {
              "id": "item",
              "kind": "formid",
              "display_label": "Item",
              "formlink_targets": [
                "ALCH",
                "AMMO",
                "APPA",
                "ARMO",
                "BOOK",
                "CLOT",
                "INGR",
                "KEYM",
                "LIGH",
                "LVLI",
                "MISC",
                "SGST",
                "SLGM",
                "WEAP"
              ]
            },
            {
              "id": "count",
              "kind": "int32",
              "display_label": "Count"
            }
          ],
          "repeatable": true
        },
        {
          "id": "SPLO",
          "kind": "raw",
          "display_label": "Spell",
          "repeatable": true
        },
        {
          "id": "NIFZ",
          "kind": "raw",
          "display_label": "Model List"
        },
        {
          "id": "NIFT",
          "kind": "raw",
          "display_label": "Model List Textures",
          "required": true
        },
        {
          "id": "ACBS",
          "kind": "parsed",
          "display_label": "Configuration",
          "codec": "struct:I,H,H,H,h,H,H",
          "fields": [
            {
              "id": "flags",
              "kind": "uint32",
              "display_label": "Flags",
              "enum_ref": "CREA.ACBS.flags"
            },
            {
              "id": "base_spell_points",
              "kind": "uint16",
              "display_label": "Base spell points"
            },
            {
              "id": "fatigue",
              "kind": "uint16",
              "display_label": "Fatigue"
            },
            {
              "id": "barter_gold",
              "kind": "uint16",
              "display_label": "Barter gold"
            },
            {
              "id": "level_offset",
              "kind": "int16",
              "display_label": "Level (offset)"
            },
            {
              "id": "calc_min",
              "kind": "uint16",
              "display_label": "Calc min"
            },
            {
              "id": "calc_max",
              "kind": "uint16",
              "display_label": "Calc max"
            }
          ],
          "required": true
        },
        {
          "id": "SNAM",
          "kind": "parsed",
          "display_label": "Factions",
          "codec": "struct:I,b,B,B,B",
          "fields": [
            {
              "id": "faction",
              "kind": "formid",
              "display_label": "Faction",
              "formlink_target": "FACT",
              "formlink_targets": [
                "FACT"
              ]
            },
            {
              "id": "rank",
              "kind": "int8",
              "display_label": "Rank"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Unknown Byte 3"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Unknown Byte 4"
            },
            {
              "id": "unknown_u8_4",
              "kind": "uint8",
              "display_label": "Unknown Byte 5"
            }
          ],
          "repeatable": true
        },
        {
          "id": "INAM",
          "kind": "parsed",
          "display_label": "Death item",
          "codec": "formid",
          "fields": [
            {
              "id": "death_item",
              "kind": "formid",
              "display_label": "Death item",
              "formlink_target": "LVLI",
              "formlink_targets": [
                "LVLI"
              ]
            }
          ],
          "formlink_target": "LVLI",
          "formlink_targets": [
            "LVLI"
          ]
        },
        {
          "id": "SCRI",
          "kind": "parsed",
          "display_label": "Script",
          "codec": "formid",
          "fields": [
            {
              "id": "script",
              "kind": "formid",
              "display_label": "Script",
              "formlink_target": "SCPT",
              "formlink_targets": [
                "SCPT"
              ]
            }
          ],
          "formlink_target": "SCPT",
          "formlink_targets": [
            "SCPT"
          ]
        },
        {
          "id": "AIDT",
          "kind": "parsed",
          "display_label": "AI Data",
          "codec": "struct:B,B,B,B,I,b,B,B,B",
          "fields": [
            {
              "id": "aggression",
              "kind": "uint8",
              "display_label": "Aggression"
            },
            {
              "id": "confidence",
              "kind": "uint8",
              "display_label": "Confidence"
            },
            {
              "id": "energy_level",
              "kind": "uint8",
              "display_label": "Energy Level"
            },
            {
              "id": "responsibility",
              "kind": "uint8",
              "display_label": "Responsibility"
            },
            {
              "id": "buys_sells_and_services",
              "kind": "uint32",
              "display_label": "Buys/Sells and Services",
              "enum_ref": "service_flags"
            },
            {
              "id": "teaches",
              "kind": "int8",
              "display_label": "Teaches",
              "enum_ref": "skill_enum"
            },
            {
              "id": "maximum_training_level",
              "kind": "uint8",
              "display_label": "Maximum training level"
            },
            {
              "id": "unknown_u8_7",
              "kind": "uint8",
              "display_label": "Unknown Byte 8"
            },
            {
              "id": "unknown_u8_8",
              "kind": "uint8",
              "display_label": "Unknown Byte 9"
            }
          ],
          "required": true
        },
        {
          "id": "PKID",
          "kind": "parsed",
          "display_label": "AI Package",
          "codec": "formid",
          "fields": [
            {
              "id": "ai_package",
              "kind": "formid",
              "display_label": "AI Package",
              "formlink_target": "PACK",
              "formlink_targets": [
                "PACK"
              ]
            }
          ],
          "repeatable": true,
          "formlink_target": "PACK",
          "formlink_targets": [
            "PACK"
          ]
        },
        {
          "id": "KFFZ",
          "kind": "raw",
          "display_label": "Animations"
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "display_label": "Creature Data",
          "codec": "struct:B,B,B,B,B,B,H,B,B,H,B,B,B,B,B,B,B,B",
          "fields": [
            {
              "id": "type",
              "kind": "uint8",
              "display_label": "Type",
              "enum_ref": "CREA.DATA.type"
            },
            {
              "id": "combat_skill",
              "kind": "uint8",
              "display_label": "Combat Skill"
            },
            {
              "id": "magic_skill",
              "kind": "uint8",
              "display_label": "Magic Skill"
            },
            {
              "id": "stealth_skill",
              "kind": "uint8",
              "display_label": "Stealth Skill"
            },
            {
              "id": "soul",
              "kind": "uint8",
              "display_label": "Soul",
              "enum_ref": "soul_gem_enum"
            },
            {
              "id": "unknown_u8_5",
              "kind": "uint8",
              "display_label": "Unknown Byte 6"
            },
            {
              "id": "health",
              "kind": "uint16",
              "display_label": "Health"
            },
            {
              "id": "unknown_u8_7",
              "kind": "uint8",
              "display_label": "Unknown Byte 8"
            },
            {
              "id": "unknown_u8_8",
              "kind": "uint8",
              "display_label": "Unknown Byte 9"
            },
            {
              "id": "attack_damage",
              "kind": "uint16",
              "display_label": "Attack Damage"
            },
            {
              "id": "strength",
              "kind": "uint8",
              "display_label": "Strength"
            },
            {
              "id": "intelligence",
              "kind": "uint8",
              "display_label": "Intelligence"
            },
            {
              "id": "willpower",
              "kind": "uint8",
              "display_label": "Willpower"
            },
            {
              "id": "agility",
              "kind": "uint8",
              "display_label": "Agility"
            },
            {
              "id": "speed",
              "kind": "uint8",
              "display_label": "Speed"
            },
            {
              "id": "endurance",
              "kind": "uint8",
              "display_label": "Endurance"
            },
            {
              "id": "personality",
              "kind": "uint8",
              "display_label": "Personality"
            },
            {
              "id": "luck",
              "kind": "uint8",
              "display_label": "Luck"
            }
          ],
          "required": true
        },
        {
          "id": "RNAM",
          "kind": "parsed",
          "display_label": "Attack reach",
          "codec": "uint8",
          "fields": [
            {
              "id": "attack_reach",
              "kind": "uint8",
              "display_label": "Attack reach"
            }
          ],
          "required": true
        },
        {
          "id": "ZNAM",
          "kind": "parsed",
          "display_label": "Combat Style",
          "codec": "formid",
          "fields": [
            {
              "id": "combat_style",
              "kind": "formid",
              "display_label": "Combat Style",
              "formlink_target": "CSTY",
              "formlink_targets": [
                "CSTY"
              ]
            }
          ],
          "formlink_target": "CSTY",
          "formlink_targets": [
            "CSTY"
          ]
        },
        {
          "id": "TNAM",
          "kind": "parsed",
          "display_label": "Turning Speed",
          "codec": "float32",
          "fields": [
            {
              "id": "float32_0",
              "kind": "float32"
            }
          ],
          "required": true
        },
        {
          "id": "BNAM",
          "kind": "parsed",
          "display_label": "Base Scale",
          "codec": "float32",
          "fields": [
            {
              "id": "float32_0",
              "kind": "float32"
            }
          ],
          "required": true
        },
        {
          "id": "WNAM",
          "kind": "parsed",
          "display_label": "Foot Weight",
          "codec": "float32",
          "fields": [
            {
              "id": "float32_0",
              "kind": "float32"
            }
          ],
          "required": true
        },
        {
          "id": "NAM0",
          "kind": "parsed",
          "display_label": "Blood Spray",
          "codec": "zstring",
          "fields": [
            {
              "id": "blood_spray",
              "kind": "zstring",
              "display_label": "Blood Spray"
            }
          ]
        },
        {
          "id": "NAM1",
          "kind": "parsed",
          "display_label": "Blood Decal",
          "codec": "zstring",
          "fields": [
            {
              "id": "blood_decal",
              "kind": "zstring",
              "display_label": "Blood Decal"
            }
          ]
        },
        {
          "id": "CSCR",
          "kind": "parsed",
          "display_label": "Inherits Sounds from",
          "codec": "formid",
          "fields": [
            {
              "id": "inherits_sounds_from",
              "kind": "formid",
              "display_label": "Inherits Sounds from",
              "formlink_target": "CREA",
              "formlink_targets": [
                "CREA"
              ]
            }
          ],
          "formlink_target": "CREA",
          "formlink_targets": [
            "CREA"
          ]
        },
        {
          "id": "CSDT",
          "kind": "parsed",
          "display_label": "Type",
          "codec": "uint32",
          "fields": [
            {
              "id": "type",
              "kind": "uint32",
              "display_label": "Type",
              "enum_ref": "CREA.CSDT.type"
            }
          ],
          "repeatable": true,
          "enum_ref": "CREA.CSDT.type",
          "scope_id": "sound_types"
        },
        {
          "id": "CSDI",
          "kind": "parsed",
          "display_label": "Sound",
          "codec": "formid",
          "fields": [
            {
              "id": "sound",
              "kind": "formid",
              "display_label": "Sound",
              "formlink_target": "SOUN",
              "formlink_targets": [
                "SOUN"
              ],
              "null_allowed": true
            }
          ],
          "repeatable": true,
          "required": true,
          "formlink_target": "SOUN",
          "formlink_targets": [
            "SOUN"
          ],
          "null_allowed": true,
          "scope_id": "sound_types"
        },
        {
          "id": "CSDC",
          "kind": "parsed",
          "display_label": "Sound Chance",
          "codec": "uint8",
          "fields": [
            {
              "id": "sound_chance",
              "kind": "uint8",
              "display_label": "Sound Chance"
            }
          ],
          "repeatable": true,
          "required": true,
          "scope_id": "sound_types"
        }
      ],
      "display_label": "Creature",
      "record_flags": {
        "valid_mask": 529440,
        "bits": [
          {
            "bit": 10,
            "name": "Quest Item"
          },
          {
            "bit": 19,
            "name": "Starts Dead"
          }
        ]
      }
    },
    {
      "id": "CSTY",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "CSTD",
          "kind": "parsed_with_raw_fallback",
          "display_label": "Standard",
          "codec": "struct:B,B,B,B,f,f,f,f,f,f,f,f,B,B,B,B,f,f,f,B,B,B,B,f,f,B,B,B,B,B,B,B,B,f,f,B,B,B,B,f,f,f,f,f,f,f,B,B,B,B,f,I",
          "fields": [
            {
              "id": "dodge_chance",
              "kind": "uint8",
              "display_label": "Dodge % Chance"
            },
            {
              "id": "left_right_chance",
              "kind": "uint8",
              "display_label": "Left/Right % Chance"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Unknown Byte 3"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Unknown Byte 4"
            },
            {
              "id": "dodge_l_r_timer_min",
              "kind": "float32",
              "display_label": "Dodge L/R Timer Min"
            },
            {
              "id": "dodge_l_r_timer_max",
              "kind": "float32",
              "display_label": "Dodge L/R Timer Max"
            },
            {
              "id": "dodge_forward_timer_min",
              "kind": "float32",
              "display_label": "Dodge Forward Timer Min"
            },
            {
              "id": "dodge_forward_timer_max",
              "kind": "float32",
              "display_label": "Dodge Forward Timer Max"
            },
            {
              "id": "dodge_back_timer_min",
              "kind": "float32",
              "display_label": "Dodge Back Timer Min"
            },
            {
              "id": "dodge_back_timer_max",
              "kind": "float32",
              "display_label": "Dodge Back Timer Max"
            },
            {
              "id": "idle_timer_min",
              "kind": "float32",
              "display_label": "Idle Timer Min"
            },
            {
              "id": "idle_timer_max",
              "kind": "float32",
              "display_label": "Idle Timer Max"
            },
            {
              "id": "block_chance",
              "kind": "uint8",
              "display_label": "Block % Chance"
            },
            {
              "id": "attack_chance",
              "kind": "uint8",
              "display_label": "Attack % Chance"
            },
            {
              "id": "unknown_u8_14",
              "kind": "uint8",
              "display_label": "Unknown Byte 15"
            },
            {
              "id": "unknown_u8_15",
              "kind": "uint8",
              "display_label": "Unknown Byte 16"
            },
            {
              "id": "recoil_stagger_bonus_to_attack",
              "kind": "float32",
              "display_label": "Recoil/Stagger Bonus to Attack"
            },
            {
              "id": "unconscious_bonus_to_attack",
              "kind": "float32",
              "display_label": "Unconscious Bonus to Attack"
            },
            {
              "id": "hand_to_hand_bonus_to_attack",
              "kind": "float32",
              "display_label": "Hand-To-Hand Bonus to Attack"
            },
            {
              "id": "power_attack_chance",
              "kind": "uint8",
              "display_label": "Power Attack % Chance"
            },
            {
              "id": "unknown_u8_20",
              "kind": "uint8",
              "display_label": "Unknown Byte 21"
            },
            {
              "id": "unknown_u8_21",
              "kind": "uint8",
              "display_label": "Unknown Byte 22"
            },
            {
              "id": "unknown_u8_22",
              "kind": "uint8",
              "display_label": "Unknown Byte 23"
            },
            {
              "id": "recoil_stagger_bonus_to_power_attack",
              "kind": "float32",
              "display_label": "Recoil/Stagger Bonus to Power Attack"
            },
            {
              "id": "unconscious_bonus_to_power_attack",
              "kind": "float32",
              "display_label": "Unconscious Bonus to Power Attack"
            },
            {
              "id": "power_attack_normal",
              "kind": "uint8",
              "display_label": "Power Attack Normal"
            },
            {
              "id": "power_attack_forward",
              "kind": "uint8",
              "display_label": "Power Attack Forward"
            },
            {
              "id": "power_attack_back",
              "kind": "uint8",
              "display_label": "Power Attack Back"
            },
            {
              "id": "power_attack_left",
              "kind": "uint8",
              "display_label": "Power Attack Left"
            },
            {
              "id": "power_attack_right",
              "kind": "uint8",
              "display_label": "Power Attack Right"
            },
            {
              "id": "unknown_u8_30",
              "kind": "uint8",
              "display_label": "Unknown Byte 31"
            },
            {
              "id": "unknown_u8_31",
              "kind": "uint8",
              "display_label": "Unknown Byte 32"
            },
            {
              "id": "unknown_u8_32",
              "kind": "uint8",
              "display_label": "Unknown Byte 33"
            },
            {
              "id": "hold_timer_min",
              "kind": "float32",
              "display_label": "Hold Timer Min"
            },
            {
              "id": "hold_timer_max",
              "kind": "float32",
              "display_label": "Hold Timer Max"
            },
            {
              "id": "flags",
              "kind": "uint8",
              "display_label": "Flags",
              "enum_ref": "CSTY.CSTD.flags"
            },
            {
              "id": "acrobatic_dodge_chance",
              "kind": "uint8",
              "display_label": "Acrobatic Dodge % Chance"
            },
            {
              "id": "unknown_u8_37",
              "kind": "uint8",
              "display_label": "Unknown Byte 38"
            },
            {
              "id": "unknown_u8_38",
              "kind": "uint8",
              "display_label": "Unknown Byte 39"
            },
            {
              "id": "range_mult_optimal",
              "kind": "float32",
              "display_label": "Range Mult (Optimal)"
            },
            {
              "id": "range_mult_max",
              "kind": "float32",
              "display_label": "Range Mult (Max)"
            },
            {
              "id": "switch_distance_melee",
              "kind": "float32",
              "display_label": "Switch Distance (Melee)"
            },
            {
              "id": "switch_distance_ranged",
              "kind": "float32",
              "display_label": "Switch Distance (Ranged)"
            },
            {
              "id": "buff_standoff_distance",
              "kind": "float32",
              "display_label": "Buff standoff Distance"
            },
            {
              "id": "ranged_standoff_distance",
              "kind": "float32",
              "display_label": "Ranged standoff Distance"
            },
            {
              "id": "group_standoff_distance",
              "kind": "float32",
              "display_label": "Group standoff Distance"
            },
            {
              "id": "rushing_attack_chance",
              "kind": "uint8",
              "display_label": "Rushing Attack % Chance"
            },
            {
              "id": "unknown_u8_47",
              "kind": "uint8",
              "display_label": "Unknown Byte 48"
            },
            {
              "id": "unknown_u8_48",
              "kind": "uint8",
              "display_label": "Unknown Byte 49"
            },
            {
              "id": "unknown_u8_49",
              "kind": "uint8",
              "display_label": "Unknown Byte 50"
            },
            {
              "id": "rushing_attack_distance_mult",
              "kind": "float32",
              "display_label": "Rushing Attack Distance Mult"
            },
            {
              "id": "do_not_acquire",
              "kind": "uint32",
              "display_label": "Do Not Acquire",
              "enum_ref": "bool_enum"
            }
          ]
        },
        {
          "id": "CSAD",
          "kind": "parsed",
          "display_label": "Advanced",
          "codec": "struct:f,f,f,f,f,f,f,f,f,f,f,f,f,f,f,f,f,f,f,f,f",
          "fields": [
            {
              "id": "dodge_fatigue_mod_mult",
              "kind": "float32",
              "display_label": "Dodge Fatigue Mod Mult"
            },
            {
              "id": "dodge_fatigue_mod_base",
              "kind": "float32",
              "display_label": "Dodge Fatigue Mod Base"
            },
            {
              "id": "encumbered_speed_mod_base",
              "kind": "float32",
              "display_label": "Encumbered Speed Mod Base"
            },
            {
              "id": "encumbered_speed_mod_mult",
              "kind": "float32",
              "display_label": "Encumbered Speed Mod Mult"
            },
            {
              "id": "dodge_while_under_attack_mult",
              "kind": "float32",
              "display_label": "Dodge While Under Attack Mult"
            },
            {
              "id": "dodge_not_under_attack_mult",
              "kind": "float32",
              "display_label": "Dodge Not Under Attack Mult"
            },
            {
              "id": "dodge_back_while_under_attack_mult",
              "kind": "float32",
              "display_label": "Dodge Back While Under Attack Mult"
            },
            {
              "id": "dodge_back_not_under_attack_mult",
              "kind": "float32",
              "display_label": "Dodge Back Not Under Attack Mult"
            },
            {
              "id": "dodge_forward_while_attacking_mult",
              "kind": "float32",
              "display_label": "Dodge Forward While Attacking Mult"
            },
            {
              "id": "dodge_forward_not_attacking_mult",
              "kind": "float32",
              "display_label": "Dodge Forward Not Attacking Mult"
            },
            {
              "id": "block_skill_modifier_mult",
              "kind": "float32",
              "display_label": "Block Skill Modifier Mult"
            },
            {
              "id": "block_skill_modifier_base",
              "kind": "float32",
              "display_label": "Block Skill Modifier Base"
            },
            {
              "id": "block_while_under_attack_mult",
              "kind": "float32",
              "display_label": "Block While Under Attack Mult"
            },
            {
              "id": "block_not_under_attack_mult",
              "kind": "float32",
              "display_label": "Block Not Under Attack Mult"
            },
            {
              "id": "attack_skill_modifier_mult",
              "kind": "float32",
              "display_label": "Attack Skill Modifier Mult"
            },
            {
              "id": "attack_skill_modifier_base",
              "kind": "float32",
              "display_label": "Attack Skill Modifier Base"
            },
            {
              "id": "attack_while_under_attack_mult",
              "kind": "float32",
              "display_label": "Attack While Under Attack Mult"
            },
            {
              "id": "attack_not_under_attack_mult",
              "kind": "float32",
              "display_label": "Attack Not Under Attack Mult"
            },
            {
              "id": "attack_during_block_mult",
              "kind": "float32",
              "display_label": "Attack During Block Mult"
            },
            {
              "id": "power_attack_fatigue_mod_base",
              "kind": "float32",
              "display_label": "Power Attack Fatigue Mod Base"
            },
            {
              "id": "power_attack_fatigue_mod_mult",
              "kind": "float32",
              "display_label": "Power Attack Fatigue Mod Mult"
            }
          ]
        }
      ],
      "display_label": "Combat Style",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "DIAL",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "QSTI",
          "kind": "parsed",
          "display_label": "Associated Quest",
          "codec": "formid",
          "fields": [
            {
              "id": "associated_quest",
              "kind": "formid",
              "display_label": "Associated Quest",
              "formlink_target": "QUST",
              "formlink_targets": [
                "QUST"
              ]
            }
          ],
          "repeatable": true,
          "formlink_target": "QUST",
          "formlink_targets": [
            "QUST"
          ]
        },
        {
          "id": "QSTR",
          "kind": "parsed",
          "display_label": "Removed Quest",
          "codec": "formid",
          "fields": [
            {
              "id": "removed_quest",
              "kind": "formid",
              "display_label": "Removed Quest",
              "formlink_target": "QUST",
              "formlink_targets": [
                "QUST"
              ]
            }
          ],
          "repeatable": true,
          "formlink_target": "QUST",
          "formlink_targets": [
            "QUST"
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "display_label": "Type",
          "codec": "uint8",
          "fields": [
            {
              "id": "type",
              "kind": "uint8",
              "display_label": "Type",
              "enum_ref": "dialogue_type_enum"
            }
          ],
          "required": true,
          "enum_ref": "dialogue_type_enum"
        },
        {
          "id": "INOM",
          "kind": "parsed",
          "display_label": "INFO Order (Masters only)",
          "codec": "formid_array",
          "fields": [
            {
              "id": "info_order_masters_only_info",
              "kind": "formid",
              "display_label": "INFO Order (Masters only) INFO",
              "formlink_target": "INFO",
              "formlink_targets": [
                "INFO"
              ]
            }
          ],
          "formlink_target": "INFO",
          "formlink_targets": [
            "INFO"
          ]
        },
        {
          "id": "INOA",
          "kind": "parsed",
          "display_label": "INFO Order (All previous modules)",
          "codec": "formid_array",
          "fields": [
            {
              "id": "info_order_all_previous_modules_info",
              "kind": "formid",
              "display_label": "INFO Order (All previous modules) INFO",
              "formlink_target": "INFO",
              "formlink_targets": [
                "INFO"
              ]
            }
          ],
          "formlink_target": "INFO",
          "formlink_targets": [
            "INFO"
          ]
        }
      ],
      "display_label": "Dialog Topic",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "DOOR",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "model_filename",
              "kind": "zstring",
              "display_label": "Model FileName"
            }
          ]
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ]
        },
        {
          "id": "SCRI",
          "kind": "parsed",
          "display_label": "Script",
          "codec": "formid",
          "fields": [
            {
              "id": "script",
              "kind": "formid",
              "display_label": "Script",
              "formlink_target": "SCPT",
              "formlink_targets": [
                "SCPT"
              ]
            }
          ],
          "formlink_target": "SCPT",
          "formlink_targets": [
            "SCPT"
          ]
        },
        {
          "id": "SNAM",
          "kind": "parsed",
          "display_label": "Open Sound",
          "codec": "formid",
          "fields": [
            {
              "id": "open_sound",
              "kind": "formid",
              "display_label": "Open Sound",
              "formlink_target": "SOUN",
              "formlink_targets": [
                "SOUN"
              ]
            }
          ],
          "formlink_target": "SOUN",
          "formlink_targets": [
            "SOUN"
          ]
        },
        {
          "id": "ANAM",
          "kind": "parsed",
          "display_label": "Close Sound",
          "codec": "formid",
          "fields": [
            {
              "id": "close_sound",
              "kind": "formid",
              "display_label": "Close Sound",
              "formlink_target": "SOUN",
              "formlink_targets": [
                "SOUN"
              ]
            }
          ],
          "formlink_target": "SOUN",
          "formlink_targets": [
            "SOUN"
          ]
        },
        {
          "id": "BNAM",
          "kind": "parsed",
          "display_label": "Loop Sound",
          "codec": "formid",
          "fields": [
            {
              "id": "loop_sound",
              "kind": "formid",
              "display_label": "Loop Sound",
              "formlink_target": "SOUN",
              "formlink_targets": [
                "SOUN"
              ]
            }
          ],
          "formlink_target": "SOUN",
          "formlink_targets": [
            "SOUN"
          ]
        },
        {
          "id": "FNAM",
          "kind": "parsed",
          "display_label": "Flags",
          "codec": "uint8",
          "fields": [
            {
              "id": "flags",
              "kind": "uint8",
              "display_label": "Flags",
              "enum_ref": "DOOR.FNAM.flags"
            }
          ],
          "required": true,
          "enum_ref": "DOOR.FNAM.flags"
        },
        {
          "id": "TNAM",
          "kind": "parsed",
          "display_label": "Destination",
          "codec": "formid",
          "fields": [
            {
              "id": "destination",
              "kind": "formid",
              "display_label": "Destination",
              "formlink_targets": [
                "CELL",
                "WRLD"
              ]
            }
          ],
          "repeatable": true,
          "formlink_targets": [
            "CELL",
            "WRLD"
          ]
        }
      ],
      "display_label": "Door",
      "record_flags": {
        "valid_mask": 5152,
        "bits": [
          {
            "bit": 10,
            "name": "Quest Item"
          }
        ]
      }
    },
    {
      "id": "EFSH",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "ICON",
          "kind": "parsed",
          "display_label": "Fill Texture",
          "codec": "zstring",
          "fields": [
            {
              "id": "fill_texture",
              "kind": "zstring",
              "display_label": "Fill Texture"
            }
          ],
          "required": true
        },
        {
          "id": "ICO2",
          "kind": "parsed",
          "display_label": "Particle Shader Texture",
          "codec": "zstring",
          "fields": [
            {
              "id": "particle_shader_texture",
              "kind": "zstring",
              "display_label": "Particle Shader Texture"
            }
          ],
          "required": true
        },
        {
          "id": "DATA",
          "kind": "parsed_with_raw_fallback",
          "display_label": "Data",
          "codec": "struct:B,B,B,B,I,I,I,B,B,B,B,f,f,f,f,f,f,f,f,f,B,B,B,B,f,f,f,f,f,f,f,f,I,I,I,I,I,f,f,f,f,f,f,f,f,f,f,f,f,f,f,f,f,f,f,f,B,B,B,B,B,B,B,B,B,B,B,B,f,f,f,f,f,f",
          "fields": [
            {
              "id": "flags",
              "kind": "uint8",
              "display_label": "Flags",
              "enum_ref": "EFSH.DATA.flags"
            },
            {
              "id": "unknown_u8_1",
              "kind": "uint8",
              "display_label": "Unknown Byte 2"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Unknown Byte 3"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Unknown Byte 4"
            },
            {
              "id": "membrane_shader_source_blend_mode",
              "kind": "uint32",
              "display_label": "Membrane Shader Source Blend Mode",
              "enum_ref": "blend_mode_enum"
            },
            {
              "id": "membrane_shader_blend_operation",
              "kind": "uint32",
              "display_label": "Membrane Shader Blend Operation",
              "enum_ref": "blend_op_enum"
            },
            {
              "id": "membrane_shader_z_test_function",
              "kind": "uint32",
              "display_label": "Membrane Shader Z Test Function",
              "enum_ref": "z_test_func_enum"
            },
            {
              "id": "fill_texture_effect_color_red",
              "kind": "uint8",
              "display_label": "Fill/Texture Effect Color Red"
            },
            {
              "id": "fill_texture_effect_color_green",
              "kind": "uint8",
              "display_label": "Fill/Texture Effect Color Green"
            },
            {
              "id": "fill_texture_effect_color_blue",
              "kind": "uint8",
              "display_label": "Fill/Texture Effect Color Blue"
            },
            {
              "id": "unknown_u8_10",
              "kind": "uint8",
              "display_label": "Fill/Texture Effect Color Unknown Byte 11"
            },
            {
              "id": "fill_texture_effect_alpha_fade_in_time",
              "kind": "float32",
              "display_label": "Fill/Texture Effect Alpha Fade In Time"
            },
            {
              "id": "fill_texture_effect_full_alpha_time",
              "kind": "float32",
              "display_label": "Fill/Texture Effect Full Alpha Time"
            },
            {
              "id": "fill_texture_effect_alpha_fade_out_time",
              "kind": "float32",
              "display_label": "Fill/Texture Effect Alpha Fade Out Time"
            },
            {
              "id": "fill_texture_effect_persistent_alpha_ratio",
              "kind": "float32",
              "display_label": "Fill/Texture Effect Persistent Alpha Ratio"
            },
            {
              "id": "fill_texture_effect_alpha_pulse_amplitude",
              "kind": "float32",
              "display_label": "Fill/Texture Effect Alpha Pulse Amplitude"
            },
            {
              "id": "fill_texture_effect_alpha_pulse_frequency",
              "kind": "float32",
              "display_label": "Fill/Texture Effect Alpha Pulse Frequency"
            },
            {
              "id": "fill_texture_effect_texture_animation_speed_u",
              "kind": "float32",
              "display_label": "Fill/Texture Effect Texture Animation Speed (U)"
            },
            {
              "id": "fill_texture_effect_texture_animation_speed_v",
              "kind": "float32",
              "display_label": "Fill/Texture Effect Texture Animation Speed (V)"
            },
            {
              "id": "edge_effect_fall_off",
              "kind": "float32",
              "display_label": "Edge Effect Fall Off"
            },
            {
              "id": "edge_effect_color_red",
              "kind": "uint8",
              "display_label": "Edge Effect Color Red"
            },
            {
              "id": "edge_effect_color_green",
              "kind": "uint8",
              "display_label": "Edge Effect Color Green"
            },
            {
              "id": "edge_effect_color_blue",
              "kind": "uint8",
              "display_label": "Edge Effect Color Blue"
            },
            {
              "id": "unknown_u8_23",
              "kind": "uint8",
              "display_label": "Edge Effect Color Unknown Byte 24"
            },
            {
              "id": "edge_effect_alpha_fade_in_time",
              "kind": "float32",
              "display_label": "Edge Effect Alpha Fade In Time"
            },
            {
              "id": "edge_effect_full_alpha_time",
              "kind": "float32",
              "display_label": "Edge Effect Full Alpha Time"
            },
            {
              "id": "edge_effect_alpha_fade_out_time",
              "kind": "float32",
              "display_label": "Edge Effect Alpha Fade Out Time"
            },
            {
              "id": "edge_effect_persistent_alpha_ratio",
              "kind": "float32",
              "display_label": "Edge Effect Persistent Alpha Ratio"
            },
            {
              "id": "edge_effect_alpha_pulse_amplitude",
              "kind": "float32",
              "display_label": "Edge Effect Alpha Pulse Amplitude"
            },
            {
              "id": "edge_effect_alpha_pusle_frequence",
              "kind": "float32",
              "display_label": "Edge Effect Alpha Pusle Frequence"
            },
            {
              "id": "fill_texture_effect_full_alpha_ratio",
              "kind": "float32",
              "display_label": "Fill/Texture Effect - Full Alpha Ratio"
            },
            {
              "id": "edge_effect_full_alpha_ratio",
              "kind": "float32",
              "display_label": "Edge Effect - Full Alpha Ratio"
            },
            {
              "id": "membrane_shader_dest_blend_mode",
              "kind": "uint32",
              "display_label": "Membrane Shader - Dest Blend Mode",
              "enum_ref": "blend_mode_enum"
            },
            {
              "id": "particle_shader_source_blend_mode",
              "kind": "uint32",
              "display_label": "Particle Shader Source Blend Mode",
              "enum_ref": "blend_mode_enum"
            },
            {
              "id": "particle_shader_blend_operation",
              "kind": "uint32",
              "display_label": "Particle Shader Blend Operation",
              "enum_ref": "blend_op_enum"
            },
            {
              "id": "particle_shader_z_test_function",
              "kind": "uint32",
              "display_label": "Particle Shader Z Test Function",
              "enum_ref": "z_test_func_enum"
            },
            {
              "id": "particle_shader_dest_blend_mode",
              "kind": "uint32",
              "display_label": "Particle Shader Dest Blend Mode",
              "enum_ref": "blend_mode_enum"
            },
            {
              "id": "particle_shader_particle_birth_ramp_up_time",
              "kind": "float32",
              "display_label": "Particle Shader Particle Birth Ramp Up Time"
            },
            {
              "id": "particle_shader_full_particle_birth_time",
              "kind": "float32",
              "display_label": "Particle Shader Full Particle Birth Time"
            },
            {
              "id": "particle_shader_particle_birth_ramp_down_time",
              "kind": "float32",
              "display_label": "Particle Shader Particle Birth Ramp Down Time"
            },
            {
              "id": "particle_shader_full_particle_birth_ratio",
              "kind": "float32",
              "display_label": "Particle Shader Full Particle Birth Ratio"
            },
            {
              "id": "particle_shader_persistant_particle_birth_ratio",
              "kind": "float32",
              "display_label": "Particle Shader Persistant Particle Birth Ratio"
            },
            {
              "id": "particle_shader_particle_lifetime",
              "kind": "float32",
              "display_label": "Particle Shader Particle Lifetime"
            },
            {
              "id": "particle_shader_particle_lifetime_1",
              "kind": "float32",
              "display_label": "Particle Shader Particle Lifetime +/-"
            },
            {
              "id": "particle_shader_initial_speed_along_normal",
              "kind": "float32",
              "display_label": "Particle Shader Initial Speed Along Normal"
            },
            {
              "id": "particle_shader_acceleration_along_normal",
              "kind": "float32",
              "display_label": "Particle Shader Acceleration Along Normal"
            },
            {
              "id": "particle_shader_initial_velocity_1",
              "kind": "float32",
              "display_label": "Particle Shader Initial Velocity #1"
            },
            {
              "id": "particle_shader_initial_velocity_2",
              "kind": "float32",
              "display_label": "Particle Shader Initial Velocity #2"
            },
            {
              "id": "particle_shader_initial_velocity_3",
              "kind": "float32",
              "display_label": "Particle Shader Initial Velocity #3"
            },
            {
              "id": "particle_shader_acceleration_1",
              "kind": "float32",
              "display_label": "Particle Shader Acceleration #1"
            },
            {
              "id": "particle_shader_acceleration_2",
              "kind": "float32",
              "display_label": "Particle Shader Acceleration #2"
            },
            {
              "id": "particle_shader_acceleration_3",
              "kind": "float32",
              "display_label": "Particle Shader Acceleration #3"
            },
            {
              "id": "particle_shader_scale_key_1",
              "kind": "float32",
              "display_label": "Particle Shader Scale Key 1"
            },
            {
              "id": "particle_shader_scale_key_2",
              "kind": "float32",
              "display_label": "Particle Shader Scale Key 2"
            },
            {
              "id": "particle_shader_scale_key_1_time",
              "kind": "float32",
              "display_label": "Particle Shader Scale Key 1 Time"
            },
            {
              "id": "particle_shader_scale_key_2_time",
              "kind": "float32",
              "display_label": "Particle Shader Scale Key 2 Time"
            },
            {
              "id": "color_key_1_color_red",
              "kind": "uint8",
              "display_label": "Color Key 1 - Color Red"
            },
            {
              "id": "color_key_1_color_green",
              "kind": "uint8",
              "display_label": "Color Key 1 - Color Green"
            },
            {
              "id": "color_key_1_color_blue",
              "kind": "uint8",
              "display_label": "Color Key 1 - Color Blue"
            },
            {
              "id": "unknown_u8_59",
              "kind": "uint8",
              "display_label": "Color Key 1 - Color Unknown Byte 60"
            },
            {
              "id": "color_key_2_color_red",
              "kind": "uint8",
              "display_label": "Color Key 2 - Color Red"
            },
            {
              "id": "color_key_2_color_green",
              "kind": "uint8",
              "display_label": "Color Key 2 - Color Green"
            },
            {
              "id": "color_key_2_color_blue",
              "kind": "uint8",
              "display_label": "Color Key 2 - Color Blue"
            },
            {
              "id": "unknown_u8_63",
              "kind": "uint8",
              "display_label": "Color Key 2 - Color Unknown Byte 64"
            },
            {
              "id": "color_key_3_color_red",
              "kind": "uint8",
              "display_label": "Color Key 3 - Color Red"
            },
            {
              "id": "color_key_3_color_green",
              "kind": "uint8",
              "display_label": "Color Key 3 - Color Green"
            },
            {
              "id": "color_key_3_color_blue",
              "kind": "uint8",
              "display_label": "Color Key 3 - Color Blue"
            },
            {
              "id": "unknown_u8_67",
              "kind": "uint8",
              "display_label": "Color Key 3 - Color Unknown Byte 68"
            },
            {
              "id": "color_key_1_color_alpha",
              "kind": "float32",
              "display_label": "Color Key 1 - Color Alpha"
            },
            {
              "id": "color_key_2_color_alpha",
              "kind": "float32",
              "display_label": "Color Key 2 - Color Alpha"
            },
            {
              "id": "color_key_3_color_alpha",
              "kind": "float32",
              "display_label": "Color Key 3 - Color Alpha"
            },
            {
              "id": "color_key_1_color_key_time",
              "kind": "float32",
              "display_label": "Color Key 1 - Color Key Time"
            },
            {
              "id": "color_key_2_color_key_time",
              "kind": "float32",
              "display_label": "Color Key 2 - Color Key Time"
            },
            {
              "id": "color_key_3_color_key_time",
              "kind": "float32",
              "display_label": "Color Key 3 - Color Key Time"
            }
          ]
        }
      ],
      "display_label": "Effect Shader",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "ENCH",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "ENIT",
          "kind": "parsed",
          "display_label": "Data",
          "codec": "struct:I,I,I,B,B,B,B",
          "fields": [
            {
              "id": "type",
              "kind": "uint32",
              "display_label": "Type",
              "enum_ref": "ENCH.ENIT.type"
            },
            {
              "id": "charge_amount",
              "kind": "uint32",
              "display_label": "Charge Amount"
            },
            {
              "id": "enchant_cost",
              "kind": "uint32",
              "display_label": "Enchant Cost"
            },
            {
              "id": "no_autocalc_cost",
              "kind": "uint8",
              "display_label": "No Autocalc Cost",
              "enum_ref": "bool_enum"
            },
            {
              "id": "unknown_u8_4",
              "kind": "uint8",
              "display_label": "Unknown Byte 5"
            },
            {
              "id": "unknown_u8_5",
              "kind": "uint8",
              "display_label": "Unknown Byte 6"
            },
            {
              "id": "unknown_u8_6",
              "kind": "uint8",
              "display_label": "Unknown Byte 7"
            }
          ],
          "required": true
        },
        {
          "id": "EFID",
          "kind": "parsed",
          "display_label": "Magic Effect Name",
          "codec": "uint32",
          "fields": [
            {
              "id": "magic_effect_name",
              "kind": "uint32",
              "display_label": "Magic Effect Name"
            }
          ],
          "repeatable": true,
          "required": true,
          "scope_id": "effects"
        },
        {
          "id": "EFIT",
          "kind": "parsed",
          "codec": "struct:I,I,I,I,I,i",
          "fields": [
            {
              "id": "magic_effect_name",
              "kind": "uint32",
              "display_label": "Magic Effect Name"
            },
            {
              "id": "magnitude",
              "kind": "uint32",
              "display_label": "Magnitude"
            },
            {
              "id": "area",
              "kind": "uint32",
              "display_label": "Area"
            },
            {
              "id": "duration",
              "kind": "uint32",
              "display_label": "Duration"
            },
            {
              "id": "type",
              "kind": "uint32",
              "display_label": "Type",
              "enum_ref": "effect_type_enum"
            },
            {
              "id": "actor_value",
              "kind": "int32",
              "display_label": "Actor Value",
              "enum_ref": "actor_value_enum"
            }
          ],
          "repeatable": true,
          "required": true,
          "scope_id": "effects"
        },
        {
          "id": "SCIT",
          "kind": "parsed_with_raw_fallback",
          "codec": "struct:I,I,I,B,B,B,B",
          "fields": [
            {
              "id": "script_effect",
              "kind": "formid",
              "display_label": "Script effect",
              "formlink_target": "SCPT",
              "formlink_targets": [
                "SCPT"
              ],
              "null_allowed": true
            },
            {
              "id": "magic_school",
              "kind": "uint32",
              "display_label": "Magic school",
              "enum_ref": "magic_school_enum"
            },
            {
              "id": "visual_effect_name",
              "kind": "uint32",
              "display_label": "Visual effect name"
            },
            {
              "id": "hostile",
              "kind": "uint8",
              "display_label": "Hostile",
              "enum_ref": "bool_enum"
            },
            {
              "id": "unknown_u8_4",
              "kind": "uint8",
              "display_label": "Unknown Byte 5"
            },
            {
              "id": "unknown_u8_5",
              "kind": "uint8",
              "display_label": "Unknown Byte 6"
            },
            {
              "id": "unknown_u8_6",
              "kind": "uint8",
              "display_label": "Unknown Byte 7"
            }
          ],
          "repeatable": true,
          "required": true,
          "scope_id": "effects"
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ],
          "repeatable": true,
          "required": true,
          "scope_id": "effects"
        }
      ],
      "display_label": "Enchantment",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "EYES",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "ICON",
          "kind": "parsed",
          "display_label": "Texture",
          "codec": "zstring",
          "fields": [
            {
              "id": "texture",
              "kind": "zstring",
              "display_label": "Texture"
            }
          ],
          "required": true
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "display_label": "Playable",
          "codec": "uint8",
          "fields": [
            {
              "id": "playable",
              "kind": "uint8",
              "display_label": "Playable",
              "enum_ref": "bool_enum"
            }
          ],
          "required": true,
          "enum_ref": "bool_enum"
        }
      ],
      "display_label": "Eyes",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "FACT",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "XNAM",
          "kind": "parsed",
          "display_label": "Relations",
          "codec": "struct:I,i",
          "fields": [
            {
              "id": "faction",
              "kind": "formid",
              "display_label": "Faction",
              "formlink_targets": [
                "FACT",
                "RACE"
              ]
            },
            {
              "id": "modifier",
              "kind": "int32",
              "display_label": "Modifier"
            }
          ],
          "repeatable": true
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "display_label": "Flags",
          "codec": "uint8",
          "fields": [
            {
              "id": "flags",
              "kind": "uint8",
              "display_label": "Flags",
              "enum_ref": "FACT.DATA.flags"
            }
          ],
          "required": true,
          "enum_ref": "FACT.DATA.flags"
        },
        {
          "id": "CNAM",
          "kind": "parsed",
          "display_label": "Crime Gold Multiplier",
          "codec": "float32",
          "fields": [
            {
              "id": "float32_0",
              "kind": "float32"
            }
          ],
          "required": true
        },
        {
          "id": "RNAM",
          "kind": "parsed",
          "display_label": "Rank#",
          "codec": "int32",
          "fields": [
            {
              "id": "rank",
              "kind": "int32",
              "display_label": "Rank#"
            }
          ],
          "repeatable": true,
          "scope_id": "ranks"
        },
        {
          "id": "MNAM",
          "kind": "parsed",
          "display_label": "Male",
          "codec": "zstring",
          "fields": [
            {
              "id": "male",
              "kind": "zstring",
              "display_label": "Male"
            }
          ],
          "repeatable": true,
          "scope_id": "ranks"
        },
        {
          "id": "FNAM",
          "kind": "parsed",
          "display_label": "Female",
          "codec": "zstring",
          "fields": [
            {
              "id": "female",
              "kind": "zstring",
              "display_label": "Female"
            }
          ],
          "repeatable": true,
          "scope_id": "ranks"
        },
        {
          "id": "INAM",
          "kind": "parsed",
          "display_label": "Insignia",
          "codec": "zstring",
          "fields": [
            {
              "id": "insignia",
              "kind": "zstring",
              "display_label": "Insignia"
            }
          ],
          "repeatable": true,
          "scope_id": "ranks"
        }
      ],
      "display_label": "Faction",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "FLOR",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "model_filename",
              "kind": "zstring",
              "display_label": "Model FileName"
            }
          ]
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ]
        },
        {
          "id": "SCRI",
          "kind": "parsed",
          "display_label": "Script",
          "codec": "formid",
          "fields": [
            {
              "id": "script",
              "kind": "formid",
              "display_label": "Script",
              "formlink_target": "SCPT",
              "formlink_targets": [
                "SCPT"
              ]
            }
          ],
          "formlink_target": "SCPT",
          "formlink_targets": [
            "SCPT"
          ]
        },
        {
          "id": "PFIG",
          "kind": "parsed",
          "display_label": "Ingredient",
          "codec": "formid",
          "fields": [
            {
              "id": "ingredient",
              "kind": "formid",
              "display_label": "Ingredient",
              "formlink_target": "INGR",
              "formlink_targets": [
                "INGR"
              ]
            }
          ],
          "formlink_target": "INGR",
          "formlink_targets": [
            "INGR"
          ]
        },
        {
          "id": "PFPC",
          "kind": "parsed",
          "display_label": "Seasonal ingredient production",
          "codec": "struct:B,B,B,B",
          "fields": [
            {
              "id": "spring",
              "kind": "uint8",
              "display_label": "Spring"
            },
            {
              "id": "summer",
              "kind": "uint8",
              "display_label": "Summer "
            },
            {
              "id": "fall",
              "kind": "uint8",
              "display_label": "Fall"
            },
            {
              "id": "winter",
              "kind": "uint8",
              "display_label": "Winter"
            }
          ],
          "required": true
        }
      ],
      "display_label": "Flora",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "FURN",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "model_filename",
              "kind": "zstring",
              "display_label": "Model FileName"
            }
          ]
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ]
        },
        {
          "id": "SCRI",
          "kind": "parsed",
          "display_label": "Script",
          "codec": "formid",
          "fields": [
            {
              "id": "script",
              "kind": "formid",
              "display_label": "Script",
              "formlink_target": "SCPT",
              "formlink_targets": [
                "SCPT"
              ]
            }
          ],
          "formlink_target": "SCPT",
          "formlink_targets": [
            "SCPT"
          ]
        },
        {
          "id": "MNAM",
          "kind": "parsed",
          "display_label": "Marker Flags",
          "codec": "bytes",
          "fields": [
            {
              "id": "marker_flags",
              "kind": "bytes",
              "display_label": "Marker Flags"
            }
          ],
          "required": true
        }
      ],
      "display_label": "Furniture",
      "record_flags": {
        "valid_mask": 5152,
        "bits": [
          {
            "bit": 10,
            "name": "Quest Item"
          }
        ]
      }
    },
    {
      "id": "GLOB",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FNAM",
          "kind": "parsed",
          "display_label": "Type",
          "codec": "uint8",
          "fields": [
            {
              "id": "type",
              "kind": "uint8",
              "display_label": "Type"
            }
          ],
          "required": true
        },
        {
          "id": "FLTV",
          "kind": "parsed",
          "display_label": "Value",
          "codec": "float32",
          "fields": [
            {
              "id": "float32_0",
              "kind": "float32"
            }
          ],
          "required": true
        }
      ],
      "display_label": "Global",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "GMST",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "display_label": "Value",
          "required": true,
          "union_selector": "editor_id_prefix",
          "union_variants": [
            {
              "id": "name",
              "codec": "zstring",
              "fields": [
                {
                  "id": "name_name",
                  "kind": "zstring",
                  "display_label": "Name Name"
                }
              ],
              "conditions": [
                {
                  "field": "editor_id_prefix",
                  "operator": "in",
                  "values": [
                    "s"
                  ]
                }
              ]
            },
            {
              "id": "int",
              "codec": "int32",
              "fields": [
                {
                  "id": "int_int",
                  "kind": "int32",
                  "display_label": "Int Int"
                }
              ],
              "conditions": [
                {
                  "field": "editor_id_prefix",
                  "operator": "not_in",
                  "values": [
                    "f",
                    "s"
                  ]
                }
              ]
            },
            {
              "id": "float",
              "codec": "float32",
              "fields": [
                {
                  "id": "float_float",
                  "kind": "float32",
                  "display_label": "Float Float"
                }
              ],
              "conditions": [
                {
                  "field": "editor_id_prefix",
                  "operator": "in",
                  "values": [
                    "f"
                  ]
                }
              ]
            }
          ]
        }
      ],
      "display_label": "Game Setting",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "GRAS",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "model_filename",
              "kind": "zstring",
              "display_label": "Model FileName"
            }
          ]
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ]
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "display_label": "Data",
          "codec": "struct:B,B,B,B,H,B,B,I,f,f,f,f,B,B,B,B",
          "fields": [
            {
              "id": "density",
              "kind": "uint8",
              "display_label": "Density"
            },
            {
              "id": "min_slope",
              "kind": "uint8",
              "display_label": "Min Slope"
            },
            {
              "id": "max_slope",
              "kind": "uint8",
              "display_label": "Max Slope"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Unknown Byte 4"
            },
            {
              "id": "unit_from_water_amount",
              "kind": "uint16",
              "display_label": "Unit from water amount"
            },
            {
              "id": "unknown_u8_5",
              "kind": "uint8",
              "display_label": "Unknown Byte 6"
            },
            {
              "id": "unknown_u8_6",
              "kind": "uint8",
              "display_label": "Unknown Byte 7"
            },
            {
              "id": "unit_from_water_type",
              "kind": "uint32",
              "display_label": "Unit from water type",
              "enum_ref": "GRAS.DATA.unit_from_water_type"
            },
            {
              "id": "position_range",
              "kind": "float32",
              "display_label": "Position Range"
            },
            {
              "id": "height_range",
              "kind": "float32",
              "display_label": "Height Range"
            },
            {
              "id": "color_range",
              "kind": "float32",
              "display_label": "Color Range"
            },
            {
              "id": "wave_period",
              "kind": "float32",
              "display_label": "Wave Period"
            },
            {
              "id": "flags",
              "kind": "uint8",
              "display_label": "Flags",
              "enum_ref": "GRAS.DATA.flags"
            },
            {
              "id": "unknown_u8_13",
              "kind": "uint8",
              "display_label": "Unknown Byte 14"
            },
            {
              "id": "unknown_u8_14",
              "kind": "uint8",
              "display_label": "Unknown Byte 15"
            },
            {
              "id": "unknown_u8_15",
              "kind": "uint8",
              "display_label": "Unknown Byte 16"
            }
          ],
          "required": true
        }
      ],
      "display_label": "Grass",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "HAIR",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "model_filename",
              "kind": "zstring",
              "display_label": "Model FileName"
            }
          ]
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ]
        },
        {
          "id": "ICON",
          "kind": "parsed",
          "display_label": "Texture",
          "codec": "zstring",
          "fields": [
            {
              "id": "texture",
              "kind": "zstring",
              "display_label": "Texture"
            }
          ],
          "required": true
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "display_label": "Flags",
          "codec": "uint8",
          "fields": [
            {
              "id": "flags",
              "kind": "uint8",
              "display_label": "Flags",
              "enum_ref": "HAIR.DATA.flags"
            }
          ],
          "required": true,
          "enum_ref": "HAIR.DATA.flags"
        }
      ],
      "display_label": "Hair",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "IDLE",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "model_filename",
              "kind": "zstring",
              "display_label": "Model FileName"
            }
          ]
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ]
        },
        {
          "id": "CTDA",
          "kind": "raw",
          "repeatable": true,
          "scope_id": "conditions"
        },
        {
          "id": "CTDT",
          "kind": "raw",
          "repeatable": true,
          "scope_id": "conditions"
        },
        {
          "id": "ANAM",
          "kind": "parsed",
          "display_label": "Animation Group Section",
          "codec": "uint8",
          "fields": [
            {
              "id": "animation_group_section",
              "kind": "uint8",
              "display_label": "Animation Group Section"
            }
          ],
          "required": true
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "display_label": "Animations",
          "codec": "struct:I,I",
          "fields": [
            {
              "id": "parent",
              "kind": "formid",
              "display_label": "Parent",
              "formlink_target": "IDLE",
              "formlink_targets": [
                "IDLE"
              ],
              "null_allowed": true
            },
            {
              "id": "previous",
              "kind": "formid",
              "display_label": "Previous",
              "formlink_target": "IDLE",
              "formlink_targets": [
                "IDLE"
              ],
              "null_allowed": true
            }
          ],
          "required": true
        }
      ],
      "display_label": "Idle Animation",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "INFO",
      "subrecords": [
        {
          "id": "DATA",
          "kind": "parsed_with_raw_fallback",
          "display_label": "Data",
          "codec": "struct:B,B,B",
          "fields": [
            {
              "id": "type",
              "kind": "uint8",
              "display_label": "Type",
              "enum_ref": "dialogue_type_enum"
            },
            {
              "id": "next_speaker",
              "kind": "uint8",
              "display_label": "Next Speaker",
              "enum_ref": "INFO.DATA.next_speaker"
            },
            {
              "id": "flags",
              "kind": "uint8",
              "display_label": "Flags",
              "enum_ref": "INFO.DATA.flags"
            }
          ]
        },
        {
          "id": "QSTI",
          "kind": "parsed",
          "display_label": "Quest",
          "codec": "formid",
          "fields": [
            {
              "id": "quest",
              "kind": "formid",
              "display_label": "Quest",
              "formlink_target": "QUST",
              "formlink_targets": [
                "QUST"
              ]
            }
          ],
          "required": true,
          "formlink_target": "QUST",
          "formlink_targets": [
            "QUST"
          ]
        },
        {
          "id": "TPIC",
          "kind": "parsed",
          "display_label": "Previous Topic",
          "codec": "formid",
          "fields": [
            {
              "id": "previous_topic",
              "kind": "formid",
              "display_label": "Previous Topic",
              "formlink_target": "DIAL",
              "formlink_targets": [
                "DIAL"
              ]
            }
          ],
          "formlink_target": "DIAL",
          "formlink_targets": [
            "DIAL"
          ]
        },
        {
          "id": "PNAM",
          "kind": "parsed",
          "display_label": "Previous Info",
          "codec": "formid",
          "fields": [
            {
              "id": "previous_info",
              "kind": "formid",
              "display_label": "Previous Info",
              "formlink_target": "INFO",
              "formlink_targets": [
                "INFO"
              ],
              "null_allowed": true
            }
          ],
          "formlink_target": "INFO",
          "formlink_targets": [
            "INFO"
          ],
          "null_allowed": true
        },
        {
          "id": "NAME",
          "kind": "parsed",
          "display_label": "Topic",
          "codec": "formid",
          "fields": [
            {
              "id": "topic",
              "kind": "formid",
              "display_label": "Topic",
              "formlink_target": "DIAL",
              "formlink_targets": [
                "DIAL"
              ]
            }
          ],
          "repeatable": true,
          "formlink_target": "DIAL",
          "formlink_targets": [
            "DIAL"
          ]
        },
        {
          "id": "TRDT",
          "kind": "parsed",
          "display_label": "Response Data",
          "codec": "struct:I,i,B,B,B,B,B,B,B,B",
          "fields": [
            {
              "id": "emotion_type",
              "kind": "uint32",
              "display_label": "Emotion Type",
              "enum_ref": "INFO.TRDT.emotion_type"
            },
            {
              "id": "emotion_value",
              "kind": "int32",
              "display_label": "Emotion Value"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Unknown Byte 3"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Unknown Byte 4"
            },
            {
              "id": "unknown_u8_4",
              "kind": "uint8",
              "display_label": "Unknown Byte 5"
            },
            {
              "id": "unknown_u8_5",
              "kind": "uint8",
              "display_label": "Unknown Byte 6"
            },
            {
              "id": "response_number",
              "kind": "uint8",
              "display_label": "Response Number"
            },
            {
              "id": "unknown_u8_7",
              "kind": "uint8",
              "display_label": "Unknown Byte 8"
            },
            {
              "id": "unknown_u8_8",
              "kind": "uint8",
              "display_label": "Unknown Byte 9"
            },
            {
              "id": "unknown_u8_9",
              "kind": "uint8",
              "display_label": "Unknown Byte 10"
            }
          ],
          "repeatable": true,
          "required": true,
          "scope_id": "responses"
        },
        {
          "id": "NAM1",
          "kind": "parsed",
          "display_label": "Response Text",
          "codec": "zstring",
          "fields": [
            {
              "id": "response_text",
              "kind": "zstring",
              "display_label": "Response Text"
            }
          ],
          "repeatable": true,
          "required": true,
          "scope_id": "responses"
        },
        {
          "id": "NAM2",
          "kind": "parsed",
          "display_label": "Actor Notes",
          "codec": "zstring",
          "fields": [
            {
              "id": "actor_notes",
              "kind": "zstring",
              "display_label": "Actor Notes"
            }
          ],
          "repeatable": true,
          "required": true,
          "scope_id": "responses"
        },
        {
          "id": "CTDA",
          "kind": "raw",
          "repeatable": true,
          "scope_id": "conditions"
        },
        {
          "id": "CTDT",
          "kind": "raw",
          "repeatable": true,
          "scope_id": "conditions"
        },
        {
          "id": "TCLT",
          "kind": "parsed",
          "display_label": "Choice",
          "codec": "formid",
          "fields": [
            {
              "id": "choice",
              "kind": "formid",
              "display_label": "Choice",
              "formlink_target": "DIAL",
              "formlink_targets": [
                "DIAL"
              ]
            }
          ],
          "repeatable": true,
          "formlink_target": "DIAL",
          "formlink_targets": [
            "DIAL"
          ]
        },
        {
          "id": "TCLF",
          "kind": "parsed",
          "display_label": "Topic",
          "codec": "formid",
          "fields": [
            {
              "id": "topic",
              "kind": "formid",
              "display_label": "Topic",
              "formlink_target": "DIAL",
              "formlink_targets": [
                "DIAL"
              ]
            }
          ],
          "repeatable": true,
          "formlink_target": "DIAL",
          "formlink_targets": [
            "DIAL"
          ]
        },
        {
          "id": "SCHR",
          "kind": "parsed",
          "display_label": "Basic Script Data",
          "codec": "struct:B,B,B,B,I,I,I,I",
          "fields": [
            {
              "id": "unknown_u8_0",
              "kind": "uint8",
              "display_label": "Unknown Byte 1"
            },
            {
              "id": "unknown_u8_1",
              "kind": "uint8",
              "display_label": "Unknown Byte 2"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Unknown Byte 3"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Unknown Byte 4"
            },
            {
              "id": "refcount",
              "kind": "uint32",
              "display_label": "RefCount"
            },
            {
              "id": "compiledsize",
              "kind": "uint32",
              "display_label": "CompiledSize"
            },
            {
              "id": "variablecount",
              "kind": "uint32",
              "display_label": "VariableCount"
            },
            {
              "id": "type",
              "kind": "uint32",
              "display_label": "Type",
              "enum_ref": "INFO.SCHR.type"
            }
          ],
          "repeatable": true,
          "authoring_layout": "row_group",
          "authoring_key": "group_result_script",
          "scope_id": "result_script"
        },
        {
          "id": "SCHD",
          "kind": "parsed",
          "display_label": "Basic Script Data",
          "codec": "struct:B,B,B,B,I,I,I,I",
          "fields": [
            {
              "id": "unknown_u8_0",
              "kind": "uint8",
              "display_label": "Unknown Byte 1"
            },
            {
              "id": "unknown_u8_1",
              "kind": "uint8",
              "display_label": "Unknown Byte 2"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Unknown Byte 3"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Unknown Byte 4"
            },
            {
              "id": "refcount",
              "kind": "uint32",
              "display_label": "RefCount"
            },
            {
              "id": "compiledsize",
              "kind": "uint32",
              "display_label": "CompiledSize"
            },
            {
              "id": "variablecount",
              "kind": "uint32",
              "display_label": "VariableCount"
            },
            {
              "id": "type",
              "kind": "uint32",
              "display_label": "Type",
              "enum_ref": "INFO.SCHD.type"
            },
            {
              "id": "unknown",
              "kind": "bytes",
              "display_label": "Unknown"
            }
          ],
          "repeatable": true,
          "authoring_layout": "row_group",
          "authoring_key": "group_result_script",
          "scope_id": "result_script"
        },
        {
          "id": "SCDA",
          "kind": "parsed",
          "display_label": "Compiled result script",
          "codec": "bytes",
          "fields": [
            {
              "id": "compiled_result_script",
              "kind": "bytes",
              "display_label": "Compiled result script"
            }
          ],
          "repeatable": true,
          "authoring_layout": "row_group",
          "authoring_key": "group_result_script",
          "scope_id": "result_script"
        },
        {
          "id": "SCTX",
          "kind": "raw",
          "display_label": "Result script source",
          "repeatable": true,
          "authoring_layout": "row_group",
          "authoring_key": "group_result_script",
          "scope_id": "result_script"
        },
        {
          "id": "SCRO",
          "kind": "parsed",
          "display_label": "Global Reference",
          "codec": "formid",
          "fields": [
            {
              "id": "formid_0",
              "kind": "formid"
            }
          ],
          "repeatable": true,
          "authoring_layout": "row_group",
          "authoring_key": "group_result_script",
          "scope_id": "result_script"
        },
        {
          "id": "SCRV",
          "kind": "parsed",
          "display_label": "Local Variable",
          "codec": "uint32",
          "fields": [
            {
              "id": "local_variable",
              "kind": "uint32",
              "display_label": "Local Variable"
            }
          ],
          "repeatable": true,
          "authoring_layout": "row_group",
          "authoring_key": "group_result_script",
          "scope_id": "result_script"
        }
      ],
      "display_label": "Dialog response",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "INGR",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "model_filename",
              "kind": "zstring",
              "display_label": "Model FileName"
            }
          ]
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ]
        },
        {
          "id": "ICON",
          "kind": "parsed",
          "display_label": "Icon FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "icon_filename",
              "kind": "zstring",
              "display_label": "Icon FileName"
            }
          ],
          "required": true
        },
        {
          "id": "SCRI",
          "kind": "parsed",
          "display_label": "Script",
          "codec": "formid",
          "fields": [
            {
              "id": "script",
              "kind": "formid",
              "display_label": "Script",
              "formlink_target": "SCPT",
              "formlink_targets": [
                "SCPT"
              ]
            }
          ],
          "formlink_target": "SCPT",
          "formlink_targets": [
            "SCPT"
          ]
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "display_label": "Weight",
          "codec": "float32",
          "fields": [
            {
              "id": "float32_0",
              "kind": "float32"
            }
          ],
          "required": true
        },
        {
          "id": "ENIT",
          "kind": "parsed",
          "display_label": "Data",
          "codec": "struct:i,B,B,B,B",
          "fields": [
            {
              "id": "value",
              "kind": "int32",
              "display_label": "Value"
            },
            {
              "id": "flags",
              "kind": "uint8",
              "display_label": "Flags",
              "enum_ref": "INGR.ENIT.flags"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Unknown Byte 3"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Unknown Byte 4"
            },
            {
              "id": "unknown_u8_4",
              "kind": "uint8",
              "display_label": "Unknown Byte 5"
            }
          ],
          "required": true
        },
        {
          "id": "EFID",
          "kind": "parsed",
          "display_label": "Magic Effect Name",
          "codec": "uint32",
          "fields": [
            {
              "id": "magic_effect_name",
              "kind": "uint32",
              "display_label": "Magic Effect Name"
            }
          ],
          "repeatable": true,
          "required": true,
          "scope_id": "effects"
        },
        {
          "id": "EFIT",
          "kind": "parsed",
          "codec": "struct:I,I,I,I,I,i",
          "fields": [
            {
              "id": "magic_effect_name",
              "kind": "uint32",
              "display_label": "Magic Effect Name"
            },
            {
              "id": "magnitude",
              "kind": "uint32",
              "display_label": "Magnitude"
            },
            {
              "id": "area",
              "kind": "uint32",
              "display_label": "Area"
            },
            {
              "id": "duration",
              "kind": "uint32",
              "display_label": "Duration"
            },
            {
              "id": "type",
              "kind": "uint32",
              "display_label": "Type",
              "enum_ref": "effect_type_enum"
            },
            {
              "id": "actor_value",
              "kind": "int32",
              "display_label": "Actor Value",
              "enum_ref": "actor_value_enum"
            }
          ],
          "repeatable": true,
          "required": true,
          "scope_id": "effects"
        },
        {
          "id": "SCIT",
          "kind": "parsed_with_raw_fallback",
          "codec": "struct:I,I,I,B,B,B,B",
          "fields": [
            {
              "id": "script_effect",
              "kind": "formid",
              "display_label": "Script effect",
              "formlink_target": "SCPT",
              "formlink_targets": [
                "SCPT"
              ],
              "null_allowed": true
            },
            {
              "id": "magic_school",
              "kind": "uint32",
              "display_label": "Magic school",
              "enum_ref": "magic_school_enum"
            },
            {
              "id": "visual_effect_name",
              "kind": "uint32",
              "display_label": "Visual effect name"
            },
            {
              "id": "hostile",
              "kind": "uint8",
              "display_label": "Hostile",
              "enum_ref": "bool_enum"
            },
            {
              "id": "unknown_u8_4",
              "kind": "uint8",
              "display_label": "Unknown Byte 5"
            },
            {
              "id": "unknown_u8_5",
              "kind": "uint8",
              "display_label": "Unknown Byte 6"
            },
            {
              "id": "unknown_u8_6",
              "kind": "uint8",
              "display_label": "Unknown Byte 7"
            }
          ],
          "repeatable": true,
          "required": true,
          "scope_id": "effects"
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ],
          "repeatable": true,
          "required": true,
          "scope_id": "effects"
        }
      ],
      "display_label": "Ingredient",
      "record_flags": {
        "valid_mask": 5152,
        "bits": [
          {
            "bit": 10,
            "name": "Quest Item"
          }
        ]
      }
    },
    {
      "id": "KEYM",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "model_filename",
              "kind": "zstring",
              "display_label": "Model FileName"
            }
          ]
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ]
        },
        {
          "id": "ICON",
          "kind": "parsed",
          "display_label": "Icon FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "icon_filename",
              "kind": "zstring",
              "display_label": "Icon FileName"
            }
          ],
          "required": true
        },
        {
          "id": "SCRI",
          "kind": "parsed",
          "display_label": "Script",
          "codec": "formid",
          "fields": [
            {
              "id": "script",
              "kind": "formid",
              "display_label": "Script",
              "formlink_target": "SCPT",
              "formlink_targets": [
                "SCPT"
              ]
            }
          ],
          "formlink_target": "SCPT",
          "formlink_targets": [
            "SCPT"
          ]
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "display_label": "Data",
          "codec": "struct:i,f",
          "fields": [
            {
              "id": "value",
              "kind": "int32",
              "display_label": "Value"
            },
            {
              "id": "weight",
              "kind": "float32",
              "display_label": "Weight"
            }
          ],
          "required": true
        }
      ],
      "display_label": "Key",
      "record_flags": {
        "valid_mask": 5152,
        "bits": [
          {
            "bit": 10,
            "name": "Quest Item"
          }
        ]
      }
    },
    {
      "id": "LAND",
      "subrecords": [
        {
          "id": "DATA",
          "kind": "parsed",
          "display_label": "Flags",
          "codec": "uint32",
          "fields": [
            {
              "id": "flags",
              "kind": "uint32",
              "display_label": "Flags",
              "enum_ref": "LAND.DATA.flags"
            }
          ],
          "enum_ref": "LAND.DATA.flags"
        },
        {
          "id": "VNML",
          "kind": "custom_codec",
          "display_label": "Vertex Normals",
          "codec": "esp_authoring_core::land::heightmap"
        },
        {
          "id": "VHGT",
          "kind": "custom_codec",
          "display_label": "Vertex Height Map",
          "codec": "esp_authoring_core::land::heightmap"
        },
        {
          "id": "VCLR",
          "kind": "parsed",
          "display_label": "Vertex Colors",
          "codec": "array_struct:",
          "fields": [
            {
              "id": "row",
              "kind": "struct",
              "display_label": "Row",
              "fields": [
                {
                  "id": "red",
                  "kind": "uint8",
                  "display_label": "Red"
                },
                {
                  "id": "green",
                  "kind": "uint8",
                  "display_label": "Green"
                },
                {
                  "id": "blue",
                  "kind": "uint8",
                  "display_label": "Blue"
                }
              ],
              "array": {
                "layout": "row_array",
                "element_codec": "B,B,B"
              }
            }
          ],
          "array": {
            "layout": "row_array"
          },
          "row_label": "Vertex Colors"
        },
        {
          "id": "BTXT",
          "kind": "parsed",
          "codec": "struct:I,B,B,h",
          "fields": [
            {
              "id": "texture",
              "kind": "formid",
              "display_label": "Texture",
              "formlink_target": "LTEX",
              "formlink_targets": [
                "LTEX"
              ],
              "null_allowed": true
            },
            {
              "id": "quadrant",
              "kind": "uint8",
              "display_label": "Quadrant",
              "enum_ref": "quadrant_enum"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Unknown Byte 3"
            },
            {
              "id": "layer",
              "kind": "int16",
              "display_label": "Layer"
            }
          ],
          "repeatable": true,
          "scope_id": "layers"
        },
        {
          "id": "ATXT",
          "kind": "parsed",
          "codec": "struct:I,B,B,h",
          "fields": [
            {
              "id": "texture",
              "kind": "formid",
              "display_label": "Texture",
              "formlink_target": "LTEX",
              "formlink_targets": [
                "LTEX"
              ],
              "null_allowed": true
            },
            {
              "id": "quadrant",
              "kind": "uint8",
              "display_label": "Quadrant",
              "enum_ref": "quadrant_enum"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Unknown Byte 3"
            },
            {
              "id": "layer",
              "kind": "int16",
              "display_label": "Layer"
            }
          ],
          "repeatable": true,
          "scope_id": "layers"
        },
        {
          "id": "VTXT",
          "kind": "parsed",
          "display_label": "Alpha Layer Data",
          "codec": "array_struct:H,B,B,f",
          "fields": [
            {
              "id": "alpha_layer_data_position",
              "kind": "uint16",
              "display_label": "Alpha Layer Data Position"
            },
            {
              "id": "unknown_u8_1",
              "kind": "uint8",
              "display_label": "Alpha Layer Data Unknown Byte 2"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Alpha Layer Data Unknown Byte 3"
            },
            {
              "id": "alpha_layer_data_opacity",
              "kind": "float32",
              "display_label": "Alpha Layer Data Opacity"
            }
          ],
          "repeatable": true,
          "array": {
            "layout": "row_array",
            "element_codec": "H,B,B,f"
          },
          "row_label": "Alpha Layer Data",
          "scope_id": "layers"
        },
        {
          "id": "VTEX",
          "kind": "parsed",
          "display_label": "Landscape Textures",
          "codec": "formid_array",
          "fields": [
            {
              "id": "landscape_textures_texture",
              "kind": "formid",
              "display_label": "Landscape Textures Texture",
              "formlink_target": "LTEX",
              "formlink_targets": [
                "LTEX"
              ],
              "null_allowed": true
            }
          ],
          "formlink_target": "LTEX",
          "formlink_targets": [
            "LTEX"
          ],
          "null_allowed": true
        }
      ],
      "display_label": "Landscape",
      "record_flags": {
        "valid_mask": 266272,
        "bits": [
          {
            "bit": 18,
            "name": "Compressed"
          }
        ]
      }
    },
    {
      "id": "LIGH",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "model_filename",
              "kind": "zstring",
              "display_label": "Model FileName"
            }
          ]
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ]
        },
        {
          "id": "SCRI",
          "kind": "parsed",
          "display_label": "Script",
          "codec": "formid",
          "fields": [
            {
              "id": "script",
              "kind": "formid",
              "display_label": "Script",
              "formlink_target": "SCPT",
              "formlink_targets": [
                "SCPT"
              ]
            }
          ],
          "formlink_target": "SCPT",
          "formlink_targets": [
            "SCPT"
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "ICON",
          "kind": "parsed",
          "display_label": "Icon FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "icon_filename",
              "kind": "zstring",
              "display_label": "Icon FileName"
            }
          ],
          "required": true
        },
        {
          "id": "DATA",
          "kind": "parsed_with_raw_fallback",
          "display_label": "Data",
          "codec": "struct:i,I,B,B,B,B,I,f,f,I,f",
          "fields": [
            {
              "id": "time",
              "kind": "int32",
              "display_label": "Time"
            },
            {
              "id": "radius",
              "kind": "uint32",
              "display_label": "Radius"
            },
            {
              "id": "color_red",
              "kind": "uint8",
              "display_label": "Color Red"
            },
            {
              "id": "color_green",
              "kind": "uint8",
              "display_label": "Color Green"
            },
            {
              "id": "color_blue",
              "kind": "uint8",
              "display_label": "Color Blue"
            },
            {
              "id": "unknown_u8_5",
              "kind": "uint8",
              "display_label": "Color Unknown Byte 6"
            },
            {
              "id": "flags",
              "kind": "uint32",
              "display_label": "Flags",
              "enum_ref": "LIGH.DATA.flags"
            },
            {
              "id": "falloff_exponent",
              "kind": "float32",
              "display_label": "Falloff Exponent"
            },
            {
              "id": "fov",
              "kind": "float32",
              "display_label": "FOV"
            },
            {
              "id": "value",
              "kind": "uint32",
              "display_label": "Value"
            },
            {
              "id": "weight",
              "kind": "float32",
              "display_label": "Weight"
            }
          ]
        },
        {
          "id": "FNAM",
          "kind": "parsed",
          "display_label": "Fade value",
          "codec": "float32",
          "fields": [
            {
              "id": "float32_0",
              "kind": "float32"
            }
          ],
          "required": true
        },
        {
          "id": "SNAM",
          "kind": "parsed",
          "display_label": "Sound",
          "codec": "formid",
          "fields": [
            {
              "id": "sound",
              "kind": "formid",
              "display_label": "Sound",
              "formlink_target": "SOUN",
              "formlink_targets": [
                "SOUN"
              ]
            }
          ],
          "formlink_target": "SOUN",
          "formlink_targets": [
            "SOUN"
          ]
        }
      ],
      "display_label": "Light",
      "record_flags": {
        "valid_mask": 5152,
        "bits": [
          {
            "bit": 10,
            "name": "Quest"
          }
        ]
      }
    },
    {
      "id": "LSCR",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "ICON",
          "kind": "parsed",
          "display_label": "Icon FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "icon_filename",
              "kind": "zstring",
              "display_label": "Icon FileName"
            }
          ],
          "required": true
        },
        {
          "id": "DESC",
          "kind": "parsed",
          "display_label": "Description",
          "codec": "zstring",
          "fields": [
            {
              "id": "description",
              "kind": "zstring",
              "display_label": "Description"
            }
          ]
        },
        {
          "id": "LNAM",
          "kind": "parsed",
          "display_label": "Locations",
          "codec": "struct:I,I,h,h",
          "fields": [
            {
              "id": "direct",
              "kind": "formid",
              "display_label": "Direct",
              "formlink_targets": [
                "CELL",
                "WRLD"
              ],
              "null_allowed": true
            },
            {
              "id": "indirect_world",
              "kind": "formid",
              "display_label": "Indirect World",
              "formlink_target": "WRLD",
              "formlink_targets": [
                "WRLD"
              ],
              "null_allowed": true
            },
            {
              "id": "indirect_grid_y",
              "kind": "int16",
              "display_label": "Indirect Grid Y"
            },
            {
              "id": "indirect_grid_x",
              "kind": "int16",
              "display_label": "Indirect Grid X"
            }
          ],
          "repeatable": true
        }
      ],
      "display_label": "Load Screen",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "LTEX",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "ICON",
          "kind": "parsed",
          "display_label": "Icon FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "icon_filename",
              "kind": "zstring",
              "display_label": "Icon FileName"
            }
          ],
          "required": true
        },
        {
          "id": "HNAM",
          "kind": "parsed",
          "display_label": "Havok Data",
          "codec": "struct:B,B,B",
          "fields": [
            {
              "id": "material_type",
              "kind": "uint8",
              "display_label": "Material Type",
              "enum_ref": "LTEX.HNAM.material_type"
            },
            {
              "id": "friction",
              "kind": "uint8",
              "display_label": "Friction"
            },
            {
              "id": "restitution",
              "kind": "uint8",
              "display_label": "Restitution"
            }
          ],
          "required": true
        },
        {
          "id": "SNAM",
          "kind": "parsed",
          "display_label": "Texture Specular Exponent",
          "codec": "uint8",
          "fields": [
            {
              "id": "texture_specular_exponent",
              "kind": "uint8",
              "display_label": "Texture Specular Exponent"
            }
          ],
          "required": true
        },
        {
          "id": "GNAM",
          "kind": "parsed",
          "display_label": "Grass",
          "codec": "formid",
          "fields": [
            {
              "id": "grass",
              "kind": "formid",
              "display_label": "Grass",
              "formlink_target": "GRAS",
              "formlink_targets": [
                "GRAS"
              ]
            }
          ],
          "repeatable": true,
          "formlink_target": "GRAS",
          "formlink_targets": [
            "GRAS"
          ]
        }
      ],
      "display_label": "Landscape Texture",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "LVLC",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "LVLD",
          "kind": "parsed",
          "display_label": "Chance none",
          "codec": "uint8",
          "fields": [
            {
              "id": "chance_none",
              "kind": "uint8",
              "display_label": "Chance none"
            }
          ],
          "required": true
        },
        {
          "id": "LVLF",
          "kind": "parsed",
          "display_label": "Flags",
          "codec": "uint8",
          "fields": [
            {
              "id": "flags",
              "kind": "uint8",
              "display_label": "Flags",
              "enum_ref": "LVLC.LVLF.flags"
            }
          ],
          "required": true,
          "enum_ref": "LVLC.LVLF.flags"
        },
        {
          "id": "LVLO",
          "kind": "parsed_with_raw_fallback",
          "codec": "struct:H,B,B,I,H,B,B",
          "fields": [
            {
              "id": "level",
              "kind": "uint16",
              "display_label": "Level"
            },
            {
              "id": "unknown_u8_1",
              "kind": "uint8",
              "display_label": "Unknown Byte 2"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Unknown Byte 3"
            },
            {
              "id": "creature",
              "kind": "formid",
              "display_label": "Creature",
              "formlink_targets": [
                "CREA",
                "LVLC",
                "NPC_"
              ]
            },
            {
              "id": "count",
              "kind": "uint16",
              "display_label": "Count"
            },
            {
              "id": "unknown_u8_5",
              "kind": "uint8",
              "display_label": "Unknown Byte 6"
            },
            {
              "id": "unknown_u8_6",
              "kind": "uint8",
              "display_label": "Unknown Byte 7"
            }
          ],
          "repeatable": true,
          "scope_id": "leveled_list_entries"
        },
        {
          "id": "SCRI",
          "kind": "parsed",
          "display_label": "Script",
          "codec": "formid",
          "fields": [
            {
              "id": "script",
              "kind": "formid",
              "display_label": "Script",
              "formlink_target": "SCPT",
              "formlink_targets": [
                "SCPT"
              ]
            }
          ],
          "formlink_target": "SCPT",
          "formlink_targets": [
            "SCPT"
          ]
        },
        {
          "id": "TNAM",
          "kind": "parsed",
          "display_label": "Creature template",
          "codec": "formid",
          "fields": [
            {
              "id": "creature_template",
              "kind": "formid",
              "display_label": "Creature template",
              "formlink_targets": [
                "CREA",
                "NPC_"
              ]
            }
          ],
          "formlink_targets": [
            "CREA",
            "NPC_"
          ]
        }
      ],
      "display_label": "Leveled Creature",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "LVLI",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "LVLD",
          "kind": "parsed",
          "display_label": "Chance none",
          "codec": "uint8",
          "fields": [
            {
              "id": "chance_none",
              "kind": "uint8",
              "display_label": "Chance none"
            }
          ],
          "required": true
        },
        {
          "id": "LVLF",
          "kind": "parsed",
          "display_label": "Flags",
          "codec": "uint8",
          "fields": [
            {
              "id": "flags",
              "kind": "uint8",
              "display_label": "Flags",
              "enum_ref": "LVLI.LVLF.flags"
            }
          ],
          "required": true,
          "enum_ref": "LVLI.LVLF.flags"
        },
        {
          "id": "LVLO",
          "kind": "parsed_with_raw_fallback",
          "codec": "struct:H,B,B,I,H,B,B",
          "fields": [
            {
              "id": "level",
              "kind": "uint16",
              "display_label": "Level"
            },
            {
              "id": "unknown_u8_1",
              "kind": "uint8",
              "display_label": "Unknown Byte 2"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Unknown Byte 3"
            },
            {
              "id": "item",
              "kind": "formid",
              "display_label": "Item",
              "formlink_targets": [
                "ALCH",
                "AMMO",
                "APPA",
                "ARMO",
                "BOOK",
                "CLOT",
                "INGR",
                "KEYM",
                "LIGH",
                "LVLI",
                "MISC",
                "SGST",
                "SLGM",
                "WEAP"
              ]
            },
            {
              "id": "count",
              "kind": "uint16",
              "display_label": "Count"
            },
            {
              "id": "unknown_u8_5",
              "kind": "uint8",
              "display_label": "Unknown Byte 6"
            },
            {
              "id": "unknown_u8_6",
              "kind": "uint8",
              "display_label": "Unknown Byte 7"
            }
          ],
          "repeatable": true,
          "scope_id": "leveled_list_entries"
        },
        {
          "id": "DATA",
          "kind": "raw"
        }
      ],
      "display_label": "Leveled Item",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "LVSP",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "LVLD",
          "kind": "parsed",
          "display_label": "Chance none",
          "codec": "uint8",
          "fields": [
            {
              "id": "chance_none",
              "kind": "uint8",
              "display_label": "Chance none"
            }
          ],
          "required": true
        },
        {
          "id": "LVLF",
          "kind": "parsed",
          "display_label": "Flags",
          "codec": "uint8",
          "fields": [
            {
              "id": "flags",
              "kind": "uint8",
              "display_label": "Flags",
              "enum_ref": "LVSP.LVLF.flags"
            }
          ],
          "required": true,
          "enum_ref": "LVSP.LVLF.flags"
        },
        {
          "id": "LVLO",
          "kind": "parsed_with_raw_fallback",
          "codec": "struct:H,B,B,I,H,B,B",
          "fields": [
            {
              "id": "level",
              "kind": "uint16",
              "display_label": "Level"
            },
            {
              "id": "unknown_u8_1",
              "kind": "uint8",
              "display_label": "Unknown Byte 2"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Unknown Byte 3"
            },
            {
              "id": "spell",
              "kind": "formid",
              "display_label": "Spell",
              "formlink_targets": [
                "LVSP",
                "SPEL"
              ]
            },
            {
              "id": "count",
              "kind": "uint16",
              "display_label": "Count"
            },
            {
              "id": "unknown_u8_5",
              "kind": "uint8",
              "display_label": "Unknown Byte 6"
            },
            {
              "id": "unknown_u8_6",
              "kind": "uint8",
              "display_label": "Unknown Byte 7"
            }
          ],
          "repeatable": true,
          "scope_id": "leveled_list_entries"
        }
      ],
      "display_label": "Leveled Spell",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "MGEF",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "raw",
          "display_label": "Magic Effect Code",
          "required": true
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "DESC",
          "kind": "parsed",
          "display_label": "Description",
          "codec": "zstring",
          "fields": [
            {
              "id": "description",
              "kind": "zstring",
              "display_label": "Description"
            }
          ]
        },
        {
          "id": "ICON",
          "kind": "parsed",
          "display_label": "Icon FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "icon_filename",
              "kind": "zstring",
              "display_label": "Icon FileName"
            }
          ],
          "required": true
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "model_filename",
              "kind": "zstring",
              "display_label": "Model FileName"
            }
          ]
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ]
        },
        {
          "id": "DATA",
          "kind": "parsed_with_raw_fallback",
          "display_label": "Data",
          "codec": "struct:I,f,I,I,i,H,B,B,I,f,I,I,I,I,I,I,f,f",
          "fields": [
            {
              "id": "flags",
              "kind": "uint32",
              "display_label": "Flags",
              "enum_ref": "MGEF.DATA.flags"
            },
            {
              "id": "base_cost",
              "kind": "float32",
              "display_label": "Base cost"
            },
            {
              "id": "assoc_item",
              "kind": "uint32",
              "display_label": "Assoc. Item"
            },
            {
              "id": "magic_school",
              "kind": "uint32",
              "display_label": "Magic School",
              "enum_ref": "magic_school_enum"
            },
            {
              "id": "resist_value",
              "kind": "int32",
              "display_label": "Resist value",
              "enum_ref": "MGEF.DATA.resist_value"
            },
            {
              "id": "counter_effect_count",
              "kind": "uint16",
              "display_label": "Counter Effect Count"
            },
            {
              "id": "unknown_u8_6",
              "kind": "uint8",
              "display_label": "Unknown Byte 7"
            },
            {
              "id": "unknown_u8_7",
              "kind": "uint8",
              "display_label": "Unknown Byte 8"
            },
            {
              "id": "light",
              "kind": "formid",
              "display_label": "Light",
              "formlink_target": "LIGH",
              "formlink_targets": [
                "LIGH"
              ],
              "null_allowed": true
            },
            {
              "id": "projectile_speed",
              "kind": "float32",
              "display_label": "Projectile speed"
            },
            {
              "id": "effect_shader",
              "kind": "formid",
              "display_label": "Effect Shader",
              "formlink_target": "EFSH",
              "formlink_targets": [
                "EFSH"
              ],
              "null_allowed": true
            },
            {
              "id": "enchant_effect",
              "kind": "formid",
              "display_label": "Enchant effect",
              "formlink_target": "EFSH",
              "formlink_targets": [
                "EFSH"
              ],
              "null_allowed": true
            },
            {
              "id": "casting_sound",
              "kind": "formid",
              "display_label": "Casting sound",
              "formlink_target": "SOUN",
              "formlink_targets": [
                "SOUN"
              ],
              "null_allowed": true
            },
            {
              "id": "bolt_sound",
              "kind": "formid",
              "display_label": "Bolt sound",
              "formlink_target": "SOUN",
              "formlink_targets": [
                "SOUN"
              ],
              "null_allowed": true
            },
            {
              "id": "hit_sound",
              "kind": "formid",
              "display_label": "Hit sound",
              "formlink_target": "SOUN",
              "formlink_targets": [
                "SOUN"
              ],
              "null_allowed": true
            },
            {
              "id": "area_sound",
              "kind": "formid",
              "display_label": "Area sound",
              "formlink_target": "SOUN",
              "formlink_targets": [
                "SOUN"
              ],
              "null_allowed": true
            },
            {
              "id": "constant_effect_enchantment_factor",
              "kind": "float32",
              "display_label": "Constant Effect enchantment factor"
            },
            {
              "id": "constant_effect_barter_factor",
              "kind": "float32",
              "display_label": "Constant Effect barter factor"
            }
          ],
          "required": true
        },
        {
          "id": "ESCE",
          "kind": "parsed",
          "display_label": "Counter Effects",
          "codec": "array_struct:I",
          "fields": [
            {
              "id": "counter_effects_counter_effect_code",
              "kind": "uint32",
              "display_label": "Counter Effects Counter Effect Code"
            }
          ],
          "array": {
            "layout": "row_array",
            "element_codec": "I"
          },
          "row_label": "Counter Effects"
        }
      ],
      "display_label": "Magic Effect",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "MISC",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "model_filename",
              "kind": "zstring",
              "display_label": "Model FileName"
            }
          ]
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ]
        },
        {
          "id": "ICON",
          "kind": "parsed",
          "display_label": "Icon FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "icon_filename",
              "kind": "zstring",
              "display_label": "Icon FileName"
            }
          ],
          "required": true
        },
        {
          "id": "SCRI",
          "kind": "parsed",
          "display_label": "Script",
          "codec": "formid",
          "fields": [
            {
              "id": "script",
              "kind": "formid",
              "display_label": "Script",
              "formlink_target": "SCPT",
              "formlink_targets": [
                "SCPT"
              ]
            }
          ],
          "formlink_target": "SCPT",
          "formlink_targets": [
            "SCPT"
          ]
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "codec": "struct:I,I",
          "fields": [
            {
              "id": "union_0",
              "kind": "uint32"
            },
            {
              "id": "union_1",
              "kind": "uint32"
            }
          ],
          "required": true
        }
      ],
      "display_label": "Misc. Item",
      "record_flags": {
        "valid_mask": 5152,
        "bits": [
          {
            "bit": 10,
            "name": "Quest Item"
          }
        ]
      }
    },
    {
      "id": "NPC_",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "model_filename",
              "kind": "zstring",
              "display_label": "Model FileName"
            }
          ]
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ]
        },
        {
          "id": "ACBS",
          "kind": "parsed",
          "display_label": "Configuration",
          "codec": "struct:I,H,H,H,h,H,H",
          "fields": [
            {
              "id": "flags",
              "kind": "uint32",
              "display_label": "Flags",
              "enum_ref": "NPC_.ACBS.flags"
            },
            {
              "id": "base_spell_points",
              "kind": "uint16",
              "display_label": "Base spell points"
            },
            {
              "id": "fatigue",
              "kind": "uint16",
              "display_label": "Fatigue"
            },
            {
              "id": "barter_gold",
              "kind": "uint16",
              "display_label": "Barter gold"
            },
            {
              "id": "level_offset",
              "kind": "int16",
              "display_label": "Level (offset)"
            },
            {
              "id": "calc_min",
              "kind": "uint16",
              "display_label": "Calc min"
            },
            {
              "id": "calc_max",
              "kind": "uint16",
              "display_label": "Calc max"
            }
          ],
          "required": true
        },
        {
          "id": "SNAM",
          "kind": "parsed",
          "display_label": "Factions",
          "codec": "struct:I,b,B,B,B",
          "fields": [
            {
              "id": "faction",
              "kind": "formid",
              "display_label": "Faction",
              "formlink_target": "FACT",
              "formlink_targets": [
                "FACT"
              ]
            },
            {
              "id": "rank",
              "kind": "int8",
              "display_label": "Rank"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Unknown Byte 3"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Unknown Byte 4"
            },
            {
              "id": "unknown_u8_4",
              "kind": "uint8",
              "display_label": "Unknown Byte 5"
            }
          ],
          "repeatable": true
        },
        {
          "id": "INAM",
          "kind": "parsed",
          "display_label": "Death item",
          "codec": "formid",
          "fields": [
            {
              "id": "death_item",
              "kind": "formid",
              "display_label": "Death item",
              "formlink_target": "LVLI",
              "formlink_targets": [
                "LVLI"
              ]
            }
          ],
          "formlink_target": "LVLI",
          "formlink_targets": [
            "LVLI"
          ]
        },
        {
          "id": "RNAM",
          "kind": "parsed",
          "display_label": "Race",
          "codec": "formid",
          "fields": [
            {
              "id": "race",
              "kind": "formid",
              "display_label": "Race",
              "formlink_target": "RACE",
              "formlink_targets": [
                "RACE"
              ]
            }
          ],
          "required": true,
          "formlink_target": "RACE",
          "formlink_targets": [
            "RACE"
          ]
        },
        {
          "id": "SPLO",
          "kind": "raw",
          "display_label": "Spell",
          "repeatable": true
        },
        {
          "id": "SCRI",
          "kind": "parsed",
          "display_label": "Script",
          "codec": "formid",
          "fields": [
            {
              "id": "script",
              "kind": "formid",
              "display_label": "Script",
              "formlink_target": "SCPT",
              "formlink_targets": [
                "SCPT"
              ]
            }
          ],
          "formlink_target": "SCPT",
          "formlink_targets": [
            "SCPT"
          ]
        },
        {
          "id": "CNTO",
          "kind": "parsed",
          "display_label": "Items",
          "codec": "struct:I,i",
          "fields": [
            {
              "id": "item",
              "kind": "formid",
              "display_label": "Item",
              "formlink_targets": [
                "ALCH",
                "AMMO",
                "APPA",
                "ARMO",
                "BOOK",
                "CLOT",
                "INGR",
                "KEYM",
                "LIGH",
                "LVLI",
                "MISC",
                "SGST",
                "SLGM",
                "WEAP"
              ]
            },
            {
              "id": "count",
              "kind": "int32",
              "display_label": "Count"
            }
          ],
          "repeatable": true
        },
        {
          "id": "AIDT",
          "kind": "parsed",
          "display_label": "AI Data",
          "codec": "struct:B,B,B,B,I,b,B,B,B",
          "fields": [
            {
              "id": "aggression",
              "kind": "uint8",
              "display_label": "Aggression"
            },
            {
              "id": "confidence",
              "kind": "uint8",
              "display_label": "Confidence"
            },
            {
              "id": "energy_level",
              "kind": "uint8",
              "display_label": "Energy Level"
            },
            {
              "id": "responsibility",
              "kind": "uint8",
              "display_label": "Responsibility"
            },
            {
              "id": "buys_sells_and_services",
              "kind": "uint32",
              "display_label": "Buys/Sells and Services",
              "enum_ref": "service_flags"
            },
            {
              "id": "teaches",
              "kind": "int8",
              "display_label": "Teaches",
              "enum_ref": "skill_enum"
            },
            {
              "id": "maximum_training_level",
              "kind": "uint8",
              "display_label": "Maximum training level"
            },
            {
              "id": "unknown_u8_7",
              "kind": "uint8",
              "display_label": "Unknown Byte 8"
            },
            {
              "id": "unknown_u8_8",
              "kind": "uint8",
              "display_label": "Unknown Byte 9"
            }
          ],
          "required": true
        },
        {
          "id": "PKID",
          "kind": "parsed",
          "display_label": "AI Package",
          "codec": "formid",
          "fields": [
            {
              "id": "ai_package",
              "kind": "formid",
              "display_label": "AI Package",
              "formlink_target": "PACK",
              "formlink_targets": [
                "PACK"
              ]
            }
          ],
          "repeatable": true,
          "formlink_target": "PACK",
          "formlink_targets": [
            "PACK"
          ]
        },
        {
          "id": "KFFZ",
          "kind": "raw",
          "display_label": "Animations"
        },
        {
          "id": "CNAM",
          "kind": "parsed",
          "display_label": "Class",
          "codec": "formid",
          "fields": [
            {
              "id": "class",
              "kind": "formid",
              "display_label": "Class",
              "formlink_target": "CLAS",
              "formlink_targets": [
                "CLAS"
              ]
            }
          ],
          "required": true,
          "formlink_target": "CLAS",
          "formlink_targets": [
            "CLAS"
          ]
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "display_label": "Stats",
          "codec": "struct:B,B,B,B,B,B,B,B,B,B,B,B,B,B,B,B,B,B,B,B,B,H,B,B,B,B,B,B,B,B,B,B",
          "fields": [
            {
              "id": "armorer",
              "kind": "uint8",
              "display_label": "Armorer"
            },
            {
              "id": "athletics",
              "kind": "uint8",
              "display_label": "Athletics"
            },
            {
              "id": "blade",
              "kind": "uint8",
              "display_label": "Blade"
            },
            {
              "id": "block",
              "kind": "uint8",
              "display_label": "Block"
            },
            {
              "id": "blunt",
              "kind": "uint8",
              "display_label": "Blunt"
            },
            {
              "id": "hand_to_hand",
              "kind": "uint8",
              "display_label": "Hand to Hand"
            },
            {
              "id": "heavy_armor",
              "kind": "uint8",
              "display_label": "Heavy Armor"
            },
            {
              "id": "alchemy",
              "kind": "uint8",
              "display_label": "Alchemy"
            },
            {
              "id": "alteration",
              "kind": "uint8",
              "display_label": "Alteration"
            },
            {
              "id": "conjuration",
              "kind": "uint8",
              "display_label": "Conjuration"
            },
            {
              "id": "destruction",
              "kind": "uint8",
              "display_label": "Destruction"
            },
            {
              "id": "illusion",
              "kind": "uint8",
              "display_label": "Illusion"
            },
            {
              "id": "mysticism",
              "kind": "uint8",
              "display_label": "Mysticism"
            },
            {
              "id": "restoration",
              "kind": "uint8",
              "display_label": "Restoration"
            },
            {
              "id": "acrobatics",
              "kind": "uint8",
              "display_label": "Acrobatics"
            },
            {
              "id": "light_armor",
              "kind": "uint8",
              "display_label": "Light Armor"
            },
            {
              "id": "marksman",
              "kind": "uint8",
              "display_label": "Marksman"
            },
            {
              "id": "mercantile",
              "kind": "uint8",
              "display_label": "Mercantile"
            },
            {
              "id": "security",
              "kind": "uint8",
              "display_label": "Security"
            },
            {
              "id": "sneak",
              "kind": "uint8",
              "display_label": "Sneak"
            },
            {
              "id": "speechcraft",
              "kind": "uint8",
              "display_label": "Speechcraft"
            },
            {
              "id": "health",
              "kind": "uint16",
              "display_label": "Health"
            },
            {
              "id": "unknown_u8_22",
              "kind": "uint8",
              "display_label": "Unknown Byte 23"
            },
            {
              "id": "unknown_u8_23",
              "kind": "uint8",
              "display_label": "Unknown Byte 24"
            },
            {
              "id": "strength",
              "kind": "uint8",
              "display_label": "Strength"
            },
            {
              "id": "intelligence",
              "kind": "uint8",
              "display_label": "Intelligence"
            },
            {
              "id": "willpower",
              "kind": "uint8",
              "display_label": "Willpower"
            },
            {
              "id": "agility",
              "kind": "uint8",
              "display_label": "Agility"
            },
            {
              "id": "speed",
              "kind": "uint8",
              "display_label": "Speed"
            },
            {
              "id": "endurance",
              "kind": "uint8",
              "display_label": "Endurance"
            },
            {
              "id": "personality",
              "kind": "uint8",
              "display_label": "Personality"
            },
            {
              "id": "luck",
              "kind": "uint8",
              "display_label": "Luck"
            }
          ],
          "required": true
        },
        {
          "id": "HNAM",
          "kind": "parsed",
          "display_label": "Hair",
          "codec": "formid",
          "fields": [
            {
              "id": "hair",
              "kind": "formid",
              "display_label": "Hair",
              "formlink_target": "HAIR",
              "formlink_targets": [
                "HAIR"
              ]
            }
          ],
          "formlink_target": "HAIR",
          "formlink_targets": [
            "HAIR"
          ]
        },
        {
          "id": "LNAM",
          "kind": "parsed",
          "display_label": "Hair length",
          "codec": "float32",
          "fields": [
            {
              "id": "float32_0",
              "kind": "float32"
            }
          ]
        },
        {
          "id": "ENAM",
          "kind": "parsed",
          "display_label": "Eyes",
          "codec": "formid_array",
          "fields": [
            {
              "id": "eyes_eyes",
              "kind": "formid",
              "display_label": "Eyes Eyes",
              "formlink_target": "EYES",
              "formlink_targets": [
                "EYES"
              ]
            }
          ],
          "formlink_target": "EYES",
          "formlink_targets": [
            "EYES"
          ]
        },
        {
          "id": "HCLR",
          "kind": "parsed",
          "display_label": "Hair color",
          "codec": "struct:B,B,B,B",
          "fields": [
            {
              "id": "hair_color_red",
              "kind": "uint8",
              "display_label": "Hair color Red"
            },
            {
              "id": "hair_color_green",
              "kind": "uint8",
              "display_label": "Hair color Green"
            },
            {
              "id": "hair_color_blue",
              "kind": "uint8",
              "display_label": "Hair color Blue"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Hair color Unknown Byte 4"
            }
          ],
          "required": true
        },
        {
          "id": "ZNAM",
          "kind": "parsed",
          "display_label": "Combat Style",
          "codec": "formid",
          "fields": [
            {
              "id": "combat_style",
              "kind": "formid",
              "display_label": "Combat Style",
              "formlink_target": "CSTY",
              "formlink_targets": [
                "CSTY"
              ]
            }
          ],
          "formlink_target": "CSTY",
          "formlink_targets": [
            "CSTY"
          ]
        },
        {
          "id": "FGGS",
          "kind": "parsed",
          "display_label": "Facegen Symmetric Geometry",
          "codec": "array_struct:f",
          "fields": [
            {
              "id": "facegen_symmetric_geometry_bone_morph_key",
              "kind": "float32",
              "display_label": "Facegen Symmetric Geometry Bone Morph Key"
            }
          ],
          "repeatable": true,
          "required": true,
          "array": {
            "layout": "row_array",
            "element_codec": "f"
          },
          "row_label": "Facegen Symmetric Geometry",
          "scope_id": "facegen_data"
        },
        {
          "id": "FGGA",
          "kind": "parsed",
          "display_label": "Facegen Asymmetric Geometry",
          "codec": "array_struct:f",
          "fields": [
            {
              "id": "facegen_asymmetric_geometry_bone_morph_key",
              "kind": "float32",
              "display_label": "Facegen Asymmetric Geometry Bone Morph Key"
            }
          ],
          "repeatable": true,
          "required": true,
          "array": {
            "layout": "row_array",
            "element_codec": "f"
          },
          "row_label": "Facegen Asymmetric Geometry",
          "scope_id": "facegen_data"
        },
        {
          "id": "FGTS",
          "kind": "parsed",
          "display_label": "Facegen Symmetric Texture",
          "codec": "array_struct:f",
          "fields": [
            {
              "id": "facegen_symmetric_texture_color_morph_key",
              "kind": "float32",
              "display_label": "Facegen Symmetric Texture Color Morph Key"
            }
          ],
          "repeatable": true,
          "required": true,
          "array": {
            "layout": "row_array",
            "element_codec": "f"
          },
          "row_label": "Facegen Symmetric Texture",
          "scope_id": "facegen_data"
        },
        {
          "id": "FNAM",
          "kind": "parsed",
          "display_label": "Unknown",
          "codec": "bytes",
          "fields": [
            {
              "id": "unknown",
              "kind": "bytes",
              "display_label": "Unknown"
            }
          ],
          "required": true
        }
      ],
      "display_label": "Non-Player Character",
      "record_flags": {
        "valid_mask": 791584,
        "bits": [
          {
            "bit": 10,
            "name": "Quest Item"
          },
          {
            "bit": 18,
            "name": "Compressed"
          },
          {
            "bit": 19,
            "name": "Starts Dead"
          }
        ]
      }
    },
    {
      "id": "PACK",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "PKDT",
          "kind": "raw",
          "display_label": "General",
          "required": true
        },
        {
          "id": "PLDT",
          "kind": "parsed",
          "display_label": "Location",
          "codec": "struct:I,I,i",
          "fields": [
            {
              "id": "type",
              "kind": "uint32",
              "display_label": "Type",
              "enum_ref": "PACK.PLDT.type"
            },
            {
              "id": "location",
              "kind": "uint32",
              "display_label": "Location"
            },
            {
              "id": "radius",
              "kind": "int32",
              "display_label": "Radius"
            }
          ]
        },
        {
          "id": "PSDT",
          "kind": "parsed",
          "display_label": "Schedule",
          "codec": "struct:b,b,b,b,I",
          "fields": [
            {
              "id": "month",
              "kind": "int8",
              "display_label": "Month"
            },
            {
              "id": "day_of_week",
              "kind": "int8",
              "display_label": "Day Of Week",
              "enum_ref": "package_schedule_day_of_week_enum"
            },
            {
              "id": "date",
              "kind": "int8",
              "display_label": "Date",
              "enum_ref": "package_schedule_day_of_month_enum"
            },
            {
              "id": "time",
              "kind": "int8",
              "display_label": "Time",
              "enum_ref": "package_schedule_hours_enum"
            },
            {
              "id": "duration_hours",
              "kind": "uint32",
              "display_label": "Duration (Hours)"
            }
          ]
        },
        {
          "id": "PTDT",
          "kind": "parsed",
          "display_label": "Target",
          "codec": "struct:I,I,i",
          "fields": [
            {
              "id": "type",
              "kind": "uint32",
              "display_label": "Type",
              "enum_ref": "PACK.PTDT.type"
            },
            {
              "id": "target",
              "kind": "uint32",
              "display_label": "Target"
            },
            {
              "id": "count",
              "kind": "int32",
              "display_label": "Count"
            }
          ]
        },
        {
          "id": "CTDA",
          "kind": "raw",
          "repeatable": true,
          "scope_id": "conditions"
        },
        {
          "id": "CTDT",
          "kind": "raw",
          "repeatable": true,
          "scope_id": "conditions"
        }
      ],
      "display_label": "Package",
      "record_flags": {
        "valid_mask": 53280,
        "bits": [
          {
            "bit": 14,
            "name": "Unknown 14"
          },
          {
            "bit": 15,
            "name": "Unknown 15"
          }
        ]
      }
    },
    {
      "id": "PGRD",
      "subrecords": [
        {
          "id": "DATA",
          "kind": "parsed",
          "display_label": "Point Count",
          "codec": "uint16",
          "fields": [
            {
              "id": "point_count",
              "kind": "uint16",
              "display_label": "Point Count"
            }
          ],
          "required": true
        },
        {
          "id": "PGRP",
          "kind": "parsed",
          "display_label": "Points",
          "codec": "array_struct:f,f,f,B,B,B,B",
          "fields": [
            {
              "id": "points_x",
              "kind": "float32",
              "display_label": "Points X"
            },
            {
              "id": "points_y",
              "kind": "float32",
              "display_label": "Points Y"
            },
            {
              "id": "points_z_even_red_orange_odd_blue",
              "kind": "float32",
              "display_label": "Points Z (Even = Red/Orange, Odd = Blue)"
            },
            {
              "id": "points_connections",
              "kind": "uint8",
              "display_label": "Points Connections"
            },
            {
              "id": "unknown_u8_4",
              "kind": "uint8",
              "display_label": "Points Unknown Byte 5"
            },
            {
              "id": "unknown_u8_5",
              "kind": "uint8",
              "display_label": "Points Unknown Byte 6"
            },
            {
              "id": "unknown_u8_6",
              "kind": "uint8",
              "display_label": "Points Unknown Byte 7"
            }
          ],
          "required": true,
          "array": {
            "layout": "row_array",
            "element_codec": "f,f,f,B,B,B,B"
          },
          "row_label": "Points"
        },
        {
          "id": "PGAG",
          "kind": "parsed",
          "display_label": "Auto-Generated Point Sets",
          "codec": "array_struct:B",
          "fields": [
            {
              "id": "auto_generated_point_sets_set",
              "kind": "uint8",
              "display_label": "Auto-Generated Point Sets Set",
              "enum_ref": "pgag_flags"
            }
          ],
          "array": {
            "layout": "row_array",
            "element_codec": "B"
          },
          "row_label": "Auto-Generated Point Sets"
        },
        {
          "id": "PGRR",
          "kind": "parsed",
          "display_label": "Point-to-Point Connections",
          "codec": "array_struct:",
          "fields": [
            {
              "id": "point",
              "kind": "int16",
              "display_label": "Point",
              "array": {
                "layout": "row_array",
                "element_codec": "h"
              }
            }
          ],
          "array": {
            "layout": "row_array"
          },
          "row_label": "Point-to-Point Connections"
        },
        {
          "id": "PGRI",
          "kind": "parsed",
          "display_label": "Inter-Cell Connections",
          "codec": "array_struct:H,B,B,f,f,f",
          "fields": [
            {
              "id": "inter_cell_connections_point",
              "kind": "uint16",
              "display_label": "Inter-Cell Connections Point"
            },
            {
              "id": "unknown_u8_1",
              "kind": "uint8",
              "display_label": "Inter-Cell Connections Unknown Byte 2"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Inter-Cell Connections Unknown Byte 3"
            },
            {
              "id": "inter_cell_connections_x",
              "kind": "float32",
              "display_label": "Inter-Cell Connections X"
            },
            {
              "id": "inter_cell_connections_y",
              "kind": "float32",
              "display_label": "Inter-Cell Connections Y"
            },
            {
              "id": "inter_cell_connections_z",
              "kind": "float32",
              "display_label": "Inter-Cell Connections Z"
            }
          ],
          "array": {
            "layout": "row_array",
            "element_codec": "H,B,B,f,f,f"
          },
          "row_label": "Inter-Cell Connections"
        },
        {
          "id": "PGRL",
          "kind": "parsed",
          "display_label": "Point-to-Reference Mappings",
          "codec": "struct:I",
          "fields": [
            {
              "id": "reference",
              "kind": "formid",
              "display_label": "Reference",
              "formlink_target": "REFR",
              "formlink_targets": [
                "REFR"
              ]
            },
            {
              "id": "points",
              "kind": "uint32",
              "display_label": "Points",
              "array": {
                "layout": "row_array",
                "element_codec": "I"
              }
            }
          ],
          "repeatable": true
        }
      ],
      "display_label": "Path Grid",
      "record_flags": {
        "valid_mask": 266272,
        "bits": [
          {
            "bit": 18,
            "name": "Compressed"
          }
        ]
      }
    },
    {
      "id": "PLYR",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "PLYR",
          "kind": "parsed",
          "display_label": "Player",
          "codec": "formid",
          "fields": [
            {
              "id": "formid_0",
              "kind": "formid"
            }
          ],
          "required": true
        }
      ],
      "display_label": "Player Reference",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "QUST",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "SCRI",
          "kind": "parsed",
          "display_label": "Script",
          "codec": "formid",
          "fields": [
            {
              "id": "script",
              "kind": "formid",
              "display_label": "Script",
              "formlink_target": "SCPT",
              "formlink_targets": [
                "SCPT"
              ]
            }
          ],
          "formlink_target": "SCPT",
          "formlink_targets": [
            "SCPT"
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "ICON",
          "kind": "parsed",
          "display_label": "Icon FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "icon_filename",
              "kind": "zstring",
              "display_label": "Icon FileName"
            }
          ],
          "required": true
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "display_label": "General",
          "codec": "struct:B,B",
          "fields": [
            {
              "id": "flags",
              "kind": "uint8",
              "display_label": "Flags",
              "enum_ref": "QUST.DATA.flags"
            },
            {
              "id": "priority",
              "kind": "uint8",
              "display_label": "Priority"
            }
          ],
          "required": true
        },
        {
          "id": "CTDA",
          "kind": "raw",
          "repeatable": true,
          "scope_id": "conditions"
        },
        {
          "id": "CTDT",
          "kind": "raw",
          "repeatable": true,
          "scope_id": "conditions"
        },
        {
          "id": "INDX",
          "kind": "parsed",
          "display_label": "Stage index",
          "codec": "int16",
          "fields": [
            {
              "id": "stage_index",
              "kind": "int16",
              "display_label": "Stage index"
            }
          ],
          "repeatable": true,
          "scope_id": "stages"
        },
        {
          "id": "QSDT",
          "kind": "parsed",
          "display_label": "Complete Quest",
          "codec": "uint8",
          "fields": [
            {
              "id": "complete_quest",
              "kind": "uint8",
              "display_label": "Complete Quest",
              "enum_ref": "bool_enum"
            }
          ],
          "repeatable": true,
          "enum_ref": "bool_enum",
          "scope_id": "stages"
        },
        {
          "id": "CTDA",
          "kind": "raw",
          "repeatable": true,
          "scope_id": "stages"
        },
        {
          "id": "CTDT",
          "kind": "raw",
          "repeatable": true,
          "scope_id": "stages"
        },
        {
          "id": "CNAM",
          "kind": "parsed",
          "display_label": "Log Entry",
          "codec": "zstring",
          "fields": [
            {
              "id": "log_entry",
              "kind": "zstring",
              "display_label": "Log Entry"
            }
          ],
          "repeatable": true,
          "scope_id": "stages"
        },
        {
          "id": "SCHR",
          "kind": "parsed",
          "display_label": "Basic Script Data",
          "codec": "struct:B,B,B,B,I,I,I,I",
          "fields": [
            {
              "id": "unknown_u8_0",
              "kind": "uint8",
              "display_label": "Unknown Byte 1"
            },
            {
              "id": "unknown_u8_1",
              "kind": "uint8",
              "display_label": "Unknown Byte 2"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Unknown Byte 3"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Unknown Byte 4"
            },
            {
              "id": "refcount",
              "kind": "uint32",
              "display_label": "RefCount"
            },
            {
              "id": "compiledsize",
              "kind": "uint32",
              "display_label": "CompiledSize"
            },
            {
              "id": "variablecount",
              "kind": "uint32",
              "display_label": "VariableCount"
            },
            {
              "id": "type",
              "kind": "uint32",
              "display_label": "Type",
              "enum_ref": "QUST.SCHR.type"
            }
          ],
          "repeatable": true,
          "scope_id": "stages"
        },
        {
          "id": "SCHD",
          "kind": "parsed",
          "display_label": "Basic Script Data",
          "codec": "struct:B,B,B,B,I,I,I,I",
          "fields": [
            {
              "id": "unknown_u8_0",
              "kind": "uint8",
              "display_label": "Unknown Byte 1"
            },
            {
              "id": "unknown_u8_1",
              "kind": "uint8",
              "display_label": "Unknown Byte 2"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Unknown Byte 3"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Unknown Byte 4"
            },
            {
              "id": "refcount",
              "kind": "uint32",
              "display_label": "RefCount"
            },
            {
              "id": "compiledsize",
              "kind": "uint32",
              "display_label": "CompiledSize"
            },
            {
              "id": "variablecount",
              "kind": "uint32",
              "display_label": "VariableCount"
            },
            {
              "id": "type",
              "kind": "uint32",
              "display_label": "Type",
              "enum_ref": "QUST.SCHD.type"
            },
            {
              "id": "unknown",
              "kind": "bytes",
              "display_label": "Unknown"
            }
          ],
          "repeatable": true,
          "scope_id": "stages"
        },
        {
          "id": "SCDA",
          "kind": "parsed",
          "display_label": "Compiled result script",
          "codec": "bytes",
          "fields": [
            {
              "id": "compiled_result_script",
              "kind": "bytes",
              "display_label": "Compiled result script"
            }
          ],
          "repeatable": true,
          "scope_id": "stages"
        },
        {
          "id": "SCTX",
          "kind": "raw",
          "display_label": "Result script source",
          "repeatable": true,
          "scope_id": "stages"
        },
        {
          "id": "SCRO",
          "kind": "parsed",
          "display_label": "Global Reference",
          "codec": "formid",
          "fields": [
            {
              "id": "formid_0",
              "kind": "formid"
            }
          ],
          "repeatable": true,
          "scope_id": "stages"
        },
        {
          "id": "SCRV",
          "kind": "parsed",
          "display_label": "Local Variable",
          "codec": "uint32",
          "fields": [
            {
              "id": "local_variable",
              "kind": "uint32",
              "display_label": "Local Variable"
            }
          ],
          "repeatable": true,
          "scope_id": "stages"
        },
        {
          "id": "QSTA",
          "kind": "parsed",
          "codec": "struct:I,B,B,B,B",
          "fields": [
            {
              "id": "target",
              "kind": "formid",
              "display_label": "Target",
              "formlink_targets": [
                "ACHR",
                "ACRE",
                "REFR"
              ]
            },
            {
              "id": "compass_marker_ignores_locks",
              "kind": "uint8",
              "display_label": "Compass Marker Ignores Locks",
              "enum_ref": "bool_enum"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Unknown Byte 3"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Unknown Byte 4"
            },
            {
              "id": "unknown_u8_4",
              "kind": "uint8",
              "display_label": "Unknown Byte 5"
            }
          ],
          "repeatable": true,
          "scope_id": "targets"
        },
        {
          "id": "CTDA",
          "kind": "raw",
          "repeatable": true,
          "scope_id": "targets"
        },
        {
          "id": "CTDT",
          "kind": "raw",
          "repeatable": true,
          "scope_id": "targets"
        }
      ],
      "display_label": "Quest",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "RACE",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "DESC",
          "kind": "parsed",
          "display_label": "Description",
          "codec": "zstring",
          "fields": [
            {
              "id": "description",
              "kind": "zstring",
              "display_label": "Description"
            }
          ]
        },
        {
          "id": "SPLO",
          "kind": "raw",
          "display_label": "Spell",
          "repeatable": true
        },
        {
          "id": "XNAM",
          "kind": "parsed",
          "display_label": "Relations",
          "codec": "struct:I,i",
          "fields": [
            {
              "id": "faction",
              "kind": "formid",
              "display_label": "Faction",
              "formlink_targets": [
                "FACT",
                "RACE"
              ]
            },
            {
              "id": "modifier",
              "kind": "int32",
              "display_label": "Modifier"
            }
          ],
          "repeatable": true
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "codec": "struct:B,B,f,f,f,f,I",
          "fields": [
            {
              "id": "skill_boosts",
              "kind": "struct",
              "display_label": "Skill Boosts",
              "fields": [
                {
                  "id": "skill",
                  "kind": "int8",
                  "display_label": "Skill",
                  "enum_ref": "major_skill_enum"
                },
                {
                  "id": "boost",
                  "kind": "int8",
                  "display_label": "Boost"
                }
              ],
              "array": {
                "layout": "row_array",
                "element_codec": "b,b"
              }
            },
            {
              "id": "unknown_u8_1",
              "kind": "uint8",
              "display_label": "Unknown Byte 2"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Unknown Byte 3"
            },
            {
              "id": "male_height",
              "kind": "float32",
              "display_label": "Male Height"
            },
            {
              "id": "female_height",
              "kind": "float32",
              "display_label": "Female Height"
            },
            {
              "id": "male_weight",
              "kind": "float32",
              "display_label": "Male Weight"
            },
            {
              "id": "female_weight",
              "kind": "float32",
              "display_label": "Female Weight"
            },
            {
              "id": "playable",
              "kind": "uint32",
              "display_label": "Playable",
              "enum_ref": "bool_enum"
            }
          ],
          "required": true
        },
        {
          "id": "VNAM",
          "kind": "parsed",
          "display_label": "Voice",
          "codec": "struct:I,I",
          "fields": [
            {
              "id": "male",
              "kind": "formid",
              "display_label": "Male",
              "formlink_target": "RACE",
              "formlink_targets": [
                "RACE"
              ],
              "null_allowed": true
            },
            {
              "id": "female",
              "kind": "formid",
              "display_label": "Female",
              "formlink_target": "RACE",
              "formlink_targets": [
                "RACE"
              ],
              "null_allowed": true
            }
          ]
        },
        {
          "id": "DNAM",
          "kind": "parsed",
          "display_label": "Default Hair",
          "codec": "struct:I,I",
          "fields": [
            {
              "id": "male",
              "kind": "formid",
              "display_label": "Male",
              "formlink_target": "HAIR",
              "formlink_targets": [
                "HAIR"
              ]
            },
            {
              "id": "female",
              "kind": "formid",
              "display_label": "Female",
              "formlink_target": "HAIR",
              "formlink_targets": [
                "HAIR"
              ]
            }
          ]
        },
        {
          "id": "CNAM",
          "kind": "parsed",
          "display_label": "Default Hair Color",
          "codec": "uint8",
          "fields": [
            {
              "id": "default_hair_color",
              "kind": "uint8",
              "display_label": "Default Hair Color"
            }
          ],
          "required": true
        },
        {
          "id": "PNAM",
          "kind": "parsed",
          "display_label": "FaceGen - Main clamp",
          "codec": "float32",
          "fields": [
            {
              "id": "float32_0",
              "kind": "float32"
            }
          ],
          "required": true
        },
        {
          "id": "UNAM",
          "kind": "parsed",
          "display_label": "FaceGen - Face clamp",
          "codec": "float32",
          "fields": [
            {
              "id": "float32_0",
              "kind": "float32"
            }
          ],
          "required": true
        },
        {
          "id": "ATTR",
          "kind": "parsed",
          "display_label": "Base Attributes",
          "codec": "struct:B,B,B,B,B,B,B,B,B,B,B,B,B,B,B,B",
          "fields": [
            {
              "id": "male_strength",
              "kind": "uint8",
              "display_label": "Male Strength"
            },
            {
              "id": "male_intelligence",
              "kind": "uint8",
              "display_label": "Male Intelligence"
            },
            {
              "id": "male_willpower",
              "kind": "uint8",
              "display_label": "Male Willpower"
            },
            {
              "id": "male_agility",
              "kind": "uint8",
              "display_label": "Male Agility"
            },
            {
              "id": "male_speed",
              "kind": "uint8",
              "display_label": "Male Speed"
            },
            {
              "id": "male_endurance",
              "kind": "uint8",
              "display_label": "Male Endurance"
            },
            {
              "id": "male_personality",
              "kind": "uint8",
              "display_label": "Male Personality"
            },
            {
              "id": "male_luck",
              "kind": "uint8",
              "display_label": "Male Luck"
            },
            {
              "id": "female_strength",
              "kind": "uint8",
              "display_label": "Female Strength"
            },
            {
              "id": "female_intelligence",
              "kind": "uint8",
              "display_label": "Female Intelligence"
            },
            {
              "id": "female_willpower",
              "kind": "uint8",
              "display_label": "Female Willpower"
            },
            {
              "id": "female_agility",
              "kind": "uint8",
              "display_label": "Female Agility"
            },
            {
              "id": "female_speed",
              "kind": "uint8",
              "display_label": "Female Speed"
            },
            {
              "id": "female_endurance",
              "kind": "uint8",
              "display_label": "Female Endurance"
            },
            {
              "id": "female_personality",
              "kind": "uint8",
              "display_label": "Female Personality"
            },
            {
              "id": "female_luck",
              "kind": "uint8",
              "display_label": "Female Luck"
            }
          ],
          "required": true
        },
        {
          "id": "NAM0",
          "kind": "parsed",
          "display_label": "Face Data Marker",
          "codec": "empty",
          "fields": [
            {
              "id": "face_data_marker",
              "kind": "empty",
              "display_label": "Face Data Marker"
            }
          ],
          "repeatable": true,
          "required": true,
          "scope_id": "face_data"
        },
        {
          "id": "NAM1",
          "kind": "parsed",
          "display_label": "Body Data Marker",
          "codec": "empty",
          "fields": [
            {
              "id": "body_data_marker",
              "kind": "empty",
              "display_label": "Body Data Marker"
            }
          ],
          "required": true
        },
        {
          "id": "MNAM",
          "kind": "parsed",
          "display_label": "Male Body Data Marker",
          "codec": "empty",
          "fields": [
            {
              "id": "male_body_data_marker",
              "kind": "empty",
              "display_label": "Male Body Data Marker"
            }
          ],
          "repeatable": true,
          "required": true,
          "authoring_layout": "row_group",
          "authoring_key": "group_male_body_data",
          "scope_id": "male_body_data"
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "model_filename",
              "kind": "zstring",
              "display_label": "Model FileName"
            }
          ],
          "repeatable": true,
          "required": true,
          "authoring_layout": "row_group",
          "authoring_key": "group_male_body_data",
          "scope_id": "male_body_data"
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ],
          "repeatable": true,
          "required": true,
          "authoring_layout": "row_group",
          "authoring_key": "group_male_body_data",
          "scope_id": "male_body_data"
        },
        {
          "id": "INDX",
          "kind": "parsed",
          "display_label": "Index",
          "codec": "uint32",
          "fields": [
            {
              "id": "index",
              "kind": "uint32",
              "display_label": "Index",
              "enum_ref": "body_part_index_enum"
            }
          ],
          "repeatable": true,
          "required": true,
          "enum_ref": "body_part_index_enum",
          "authoring_layout": "row_group",
          "authoring_key": "group_male_body_data",
          "scope_id": "male_body_data"
        },
        {
          "id": "ICON",
          "kind": "parsed",
          "display_label": "Icon FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "icon_filename",
              "kind": "zstring",
              "display_label": "Icon FileName"
            }
          ],
          "repeatable": true,
          "required": true,
          "authoring_layout": "row_group",
          "authoring_key": "group_male_body_data",
          "scope_id": "male_body_data"
        },
        {
          "id": "FNAM",
          "kind": "parsed",
          "display_label": "Female Body Data Marker",
          "codec": "empty",
          "fields": [
            {
              "id": "female_body_data_marker",
              "kind": "empty",
              "display_label": "Female Body Data Marker"
            }
          ],
          "repeatable": true,
          "required": true,
          "authoring_layout": "row_group",
          "authoring_key": "group_female_body_data",
          "scope_id": "female_body_data"
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "model_filename",
              "kind": "zstring",
              "display_label": "Model FileName"
            }
          ],
          "repeatable": true,
          "required": true,
          "authoring_layout": "row_group",
          "authoring_key": "group_female_body_data",
          "scope_id": "female_body_data"
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ],
          "repeatable": true,
          "required": true,
          "authoring_layout": "row_group",
          "authoring_key": "group_female_body_data",
          "scope_id": "female_body_data"
        },
        {
          "id": "INDX",
          "kind": "parsed",
          "display_label": "Index",
          "codec": "uint32",
          "fields": [
            {
              "id": "index",
              "kind": "uint32",
              "display_label": "Index",
              "enum_ref": "body_part_index_enum"
            }
          ],
          "repeatable": true,
          "required": true,
          "enum_ref": "body_part_index_enum",
          "authoring_layout": "row_group",
          "authoring_key": "group_female_body_data",
          "scope_id": "female_body_data"
        },
        {
          "id": "ICON",
          "kind": "parsed",
          "display_label": "Icon FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "icon_filename",
              "kind": "zstring",
              "display_label": "Icon FileName"
            }
          ],
          "repeatable": true,
          "required": true,
          "authoring_layout": "row_group",
          "authoring_key": "group_female_body_data",
          "scope_id": "female_body_data"
        },
        {
          "id": "HNAM",
          "kind": "parsed",
          "display_label": "Hairs",
          "codec": "formid_array",
          "fields": [
            {
              "id": "hairs_hair",
              "kind": "formid",
              "display_label": "Hairs Hair",
              "formlink_target": "HAIR",
              "formlink_targets": [
                "HAIR"
              ]
            }
          ],
          "required": true,
          "formlink_target": "HAIR",
          "formlink_targets": [
            "HAIR"
          ]
        },
        {
          "id": "ENAM",
          "kind": "parsed",
          "display_label": "Eyes",
          "codec": "formid_array",
          "fields": [
            {
              "id": "eyes_eye",
              "kind": "formid",
              "display_label": "Eyes Eye",
              "formlink_target": "EYES",
              "formlink_targets": [
                "EYES"
              ]
            }
          ],
          "required": true,
          "formlink_target": "EYES",
          "formlink_targets": [
            "EYES"
          ]
        },
        {
          "id": "FGGS",
          "kind": "parsed",
          "display_label": "Facegen Symmetric Geometry",
          "codec": "array_struct:f",
          "fields": [
            {
              "id": "facegen_symmetric_geometry_bone_morph_key",
              "kind": "float32",
              "display_label": "Facegen Symmetric Geometry Bone Morph Key"
            }
          ],
          "repeatable": true,
          "required": true,
          "array": {
            "layout": "row_array",
            "element_codec": "f"
          },
          "row_label": "Facegen Symmetric Geometry",
          "scope_id": "facegen_data"
        },
        {
          "id": "FGGA",
          "kind": "parsed",
          "display_label": "Facegen Asymmetric Geometry",
          "codec": "array_struct:f",
          "fields": [
            {
              "id": "facegen_asymmetric_geometry_bone_morph_key",
              "kind": "float32",
              "display_label": "Facegen Asymmetric Geometry Bone Morph Key"
            }
          ],
          "repeatable": true,
          "required": true,
          "array": {
            "layout": "row_array",
            "element_codec": "f"
          },
          "row_label": "Facegen Asymmetric Geometry",
          "scope_id": "facegen_data"
        },
        {
          "id": "FGTS",
          "kind": "parsed",
          "display_label": "Facegen Symmetric Texture",
          "codec": "array_struct:f",
          "fields": [
            {
              "id": "facegen_symmetric_texture_color_morph_key",
              "kind": "float32",
              "display_label": "Facegen Symmetric Texture Color Morph Key"
            }
          ],
          "repeatable": true,
          "required": true,
          "array": {
            "layout": "row_array",
            "element_codec": "f"
          },
          "row_label": "Facegen Symmetric Texture",
          "scope_id": "facegen_data"
        },
        {
          "id": "SNAM",
          "kind": "parsed",
          "display_label": "Unknown",
          "codec": "bytes",
          "fields": [
            {
              "id": "unknown",
              "kind": "bytes",
              "display_label": "Unknown"
            }
          ],
          "required": true
        },
        {
          "id": "HEAD",
          "kind": "parsed",
          "display_label": "Head",
          "codec": "formid",
          "fields": [
            {
              "id": "head",
              "kind": "formid",
              "display_label": "Head"
            }
          ],
          "repeatable": true
        }
      ],
      "display_label": "Race",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "REFR",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "NAME",
          "kind": "parsed",
          "display_label": "Base",
          "codec": "formid",
          "fields": [
            {
              "id": "base",
              "kind": "formid",
              "display_label": "Base",
              "formlink_targets": [
                "ACTI",
                "ALCH",
                "AMMO",
                "APPA",
                "ARMO",
                "BOOK",
                "CLOT",
                "CONT",
                "DOOR",
                "FLOR",
                "FURN",
                "GRAS",
                "INGR",
                "KEYM",
                "LIGH",
                "LVLC",
                "MISC",
                "SBSP",
                "SGST",
                "SLGM",
                "SOUN",
                "STAT",
                "TREE",
                "WEAP"
              ]
            }
          ],
          "formlink_targets": [
            "ACTI",
            "ALCH",
            "AMMO",
            "APPA",
            "ARMO",
            "BOOK",
            "CLOT",
            "CONT",
            "DOOR",
            "FLOR",
            "FURN",
            "GRAS",
            "INGR",
            "KEYM",
            "LIGH",
            "LVLC",
            "MISC",
            "SBSP",
            "SGST",
            "SLGM",
            "SOUN",
            "STAT",
            "TREE",
            "WEAP"
          ]
        },
        {
          "id": "XTEL",
          "kind": "parsed",
          "display_label": "Teleport Destination",
          "codec": "struct:I,f,f,f,f,f,f",
          "fields": [
            {
              "id": "door",
              "kind": "formid",
              "display_label": "Door",
              "formlink_target": "REFR",
              "formlink_targets": [
                "REFR"
              ]
            },
            {
              "id": "position_x",
              "kind": "float32",
              "display_label": "Position X"
            },
            {
              "id": "position_y",
              "kind": "float32",
              "display_label": "Position Y"
            },
            {
              "id": "position_z",
              "kind": "float32",
              "display_label": "Position Z"
            },
            {
              "id": "rotation_x",
              "kind": "float32",
              "display_label": "Rotation X"
            },
            {
              "id": "rotation_y",
              "kind": "float32",
              "display_label": "Rotation Y"
            },
            {
              "id": "rotation_z",
              "kind": "float32",
              "display_label": "Rotation Z"
            }
          ]
        },
        {
          "id": "XLOC",
          "kind": "raw",
          "display_label": "Lock information"
        },
        {
          "id": "XESP",
          "kind": "parsed",
          "display_label": "Enable Parent",
          "codec": "struct:I,B,B,B,B",
          "fields": [
            {
              "id": "reference",
              "kind": "formid",
              "display_label": "Reference",
              "formlink_targets": [
                "ACHR",
                "ACRE",
                "PLYR",
                "REFR"
              ]
            },
            {
              "id": "set_enable_state_to_opposite_of_parent",
              "kind": "uint8",
              "display_label": "Set Enable State To Opposite Of Parent",
              "enum_ref": "bool_enum"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Unknown Byte 3"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Unknown Byte 4"
            },
            {
              "id": "unknown_u8_4",
              "kind": "uint8",
              "display_label": "Unknown Byte 5"
            }
          ]
        },
        {
          "id": "XTRG",
          "kind": "parsed",
          "display_label": "Target",
          "codec": "formid",
          "fields": [
            {
              "id": "target",
              "kind": "formid",
              "display_label": "Target",
              "formlink_targets": [
                "ACHR",
                "ACRE",
                "REFR"
              ]
            }
          ],
          "formlink_targets": [
            "ACHR",
            "ACRE",
            "REFR"
          ]
        },
        {
          "id": "XSED",
          "kind": "parsed",
          "display_label": "Speed Tree",
          "codec": "struct:B",
          "fields": [
            {
              "id": "seed",
              "kind": "uint8",
              "display_label": "Seed"
            }
          ]
        },
        {
          "id": "XLOD",
          "kind": "parsed",
          "display_label": "Distant LOD Data",
          "codec": "array_struct:f",
          "fields": [
            {
              "id": "distant_lod_data_unknown",
              "kind": "float32",
              "display_label": "Distant LOD Data Unknown"
            }
          ],
          "array": {
            "layout": "row_array",
            "element_codec": "f"
          },
          "row_label": "Distant LOD Data"
        },
        {
          "id": "XCHG",
          "kind": "parsed",
          "display_label": "Charge",
          "codec": "float32",
          "fields": [
            {
              "id": "float32_0",
              "kind": "float32"
            }
          ]
        },
        {
          "id": "XHLT",
          "kind": "parsed",
          "display_label": "Health",
          "codec": "int32",
          "fields": [
            {
              "id": "health",
              "kind": "int32",
              "display_label": "Health"
            }
          ]
        },
        {
          "id": "XPCI",
          "kind": "parsed",
          "display_label": "Unused",
          "codec": "formid",
          "fields": [
            {
              "id": "unused",
              "kind": "formid",
              "display_label": "Unused",
              "formlink_target": "CELL",
              "formlink_targets": [
                "CELL"
              ]
            }
          ],
          "repeatable": true,
          "formlink_target": "CELL",
          "formlink_targets": [
            "CELL"
          ],
          "authoring_layout": "row_group",
          "authoring_key": "group_unused",
          "scope_id": "unused"
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Unused",
          "codec": "zstring",
          "fields": [
            {
              "id": "unused",
              "kind": "zstring",
              "display_label": "Unused"
            }
          ],
          "repeatable": true,
          "authoring_layout": "row_group",
          "authoring_key": "group_unused",
          "scope_id": "unused"
        },
        {
          "id": "XLCM",
          "kind": "parsed",
          "display_label": "Level Modifier",
          "codec": "int32",
          "fields": [
            {
              "id": "level_modifier",
              "kind": "int32",
              "display_label": "Level Modifier"
            }
          ]
        },
        {
          "id": "XRTM",
          "kind": "parsed",
          "display_label": "Reference Teleport Marker",
          "codec": "formid",
          "fields": [
            {
              "id": "reference_teleport_marker",
              "kind": "formid",
              "display_label": "Reference Teleport Marker",
              "formlink_target": "REFR",
              "formlink_targets": [
                "REFR"
              ]
            }
          ],
          "formlink_target": "REFR",
          "formlink_targets": [
            "REFR"
          ]
        },
        {
          "id": "XACT",
          "kind": "parsed",
          "display_label": "Action Flag",
          "codec": "uint32",
          "fields": [
            {
              "id": "action_flag",
              "kind": "uint32",
              "display_label": "Action Flag",
              "enum_ref": "REFR.XACT.action_flag"
            }
          ],
          "enum_ref": "REFR.XACT.action_flag"
        },
        {
          "id": "XCNT",
          "kind": "parsed",
          "display_label": "Count",
          "codec": "uint32",
          "fields": [
            {
              "id": "count",
              "kind": "uint32",
              "display_label": "Count"
            }
          ]
        },
        {
          "id": "XMRK",
          "kind": "parsed",
          "display_label": "Map Marker Data",
          "codec": "empty",
          "fields": [
            {
              "id": "map_marker_data",
              "kind": "empty",
              "display_label": "Map Marker Data"
            }
          ],
          "repeatable": true,
          "required": true,
          "authoring_layout": "row_group",
          "authoring_key": "group_map_marker",
          "scope_id": "map_marker"
        },
        {
          "id": "FNAM",
          "kind": "parsed",
          "display_label": "Map Flags",
          "codec": "uint8",
          "fields": [
            {
              "id": "map_flags",
              "kind": "uint8",
              "display_label": "Map Flags",
              "enum_ref": "REFR.FNAM.map_flags"
            }
          ],
          "repeatable": true,
          "required": true,
          "enum_ref": "REFR.FNAM.map_flags",
          "authoring_layout": "row_group",
          "authoring_key": "group_map_marker",
          "scope_id": "map_marker"
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ],
          "repeatable": true,
          "required": true,
          "authoring_layout": "row_group",
          "authoring_key": "group_map_marker",
          "scope_id": "map_marker"
        },
        {
          "id": "TNAM",
          "kind": "parsed",
          "codec": "struct:B,B",
          "fields": [
            {
              "id": "type",
              "kind": "uint8",
              "display_label": "Type",
              "enum_ref": "REFR.TNAM.type"
            },
            {
              "id": "unknown_u8_1",
              "kind": "uint8",
              "display_label": "Unknown Byte 2"
            }
          ],
          "repeatable": true,
          "required": true,
          "authoring_layout": "row_group",
          "authoring_key": "group_map_marker",
          "scope_id": "map_marker"
        },
        {
          "id": "ONAM",
          "kind": "parsed",
          "display_label": "Open by Default",
          "codec": "empty",
          "fields": [
            {
              "id": "open_by_default",
              "kind": "empty",
              "display_label": "Open by Default"
            }
          ]
        },
        {
          "id": "XRGD",
          "kind": "parsed",
          "display_label": "Bones",
          "codec": "array_struct:B,B,B,B,f,f,f,f,f,f",
          "fields": [
            {
              "id": "bones_bone_id",
              "kind": "uint8",
              "display_label": "Bones Bone Id"
            },
            {
              "id": "unknown_u8_1",
              "kind": "uint8",
              "display_label": "Bones Unknown Byte 2"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Bones Unknown Byte 3"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Bones Unknown Byte 4"
            },
            {
              "id": "bones_position_x",
              "kind": "float32",
              "display_label": "Bones Position X"
            },
            {
              "id": "bones_position_y",
              "kind": "float32",
              "display_label": "Bones Position Y"
            },
            {
              "id": "bones_position_z",
              "kind": "float32",
              "display_label": "Bones Position Z"
            },
            {
              "id": "bones_rotation_x",
              "kind": "float32",
              "display_label": "Bones Rotation X"
            },
            {
              "id": "bones_rotation_y",
              "kind": "float32",
              "display_label": "Bones Rotation Y"
            },
            {
              "id": "bones_rotation_z",
              "kind": "float32",
              "display_label": "Bones Rotation Z"
            }
          ],
          "repeatable": true,
          "array": {
            "layout": "row_array",
            "element_codec": "B,B,B,B,f,f,f,f,f,f"
          },
          "row_label": "Bones",
          "scope_id": "ragdoll_data"
        },
        {
          "id": "XSCL",
          "kind": "parsed",
          "display_label": "Scale",
          "codec": "float32",
          "fields": [
            {
              "id": "float32_0",
              "kind": "float32"
            }
          ]
        },
        {
          "id": "XSOL",
          "kind": "parsed",
          "display_label": "Contained Soul",
          "codec": "uint8",
          "fields": [
            {
              "id": "contained_soul",
              "kind": "uint8",
              "display_label": "Contained Soul",
              "enum_ref": "soul_gem_enum"
            }
          ],
          "enum_ref": "soul_gem_enum"
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "codec": "struct:f,f,f,f,f,f",
          "fields": [
            {
              "id": "position_rotation_position_x",
              "kind": "float32",
              "display_label": "Position Rotation Position X"
            },
            {
              "id": "position_rotation_position_y",
              "kind": "float32",
              "display_label": "Position Rotation Position Y"
            },
            {
              "id": "position_rotation_position_z",
              "kind": "float32",
              "display_label": "Position Rotation Position Z"
            },
            {
              "id": "position_rotation_rotation_x",
              "kind": "float32",
              "display_label": "Position Rotation Rotation X"
            },
            {
              "id": "position_rotation_rotation_y",
              "kind": "float32",
              "display_label": "Position Rotation Rotation Y"
            },
            {
              "id": "position_rotation_rotation_z",
              "kind": "float32",
              "display_label": "Position Rotation Rotation Z"
            }
          ],
          "required": true
        },
        {
          "id": "XOWN",
          "kind": "parsed",
          "display_label": "Owner",
          "codec": "formid",
          "fields": [
            {
              "id": "owner",
              "kind": "formid",
              "display_label": "Owner"
            }
          ]
        },
        {
          "id": "XRNK",
          "kind": "parsed",
          "display_label": "Faction rank",
          "codec": "int32",
          "fields": [
            {
              "id": "faction_rank",
              "kind": "int32",
              "display_label": "Faction rank"
            }
          ]
        },
        {
          "id": "XGLB",
          "kind": "parsed",
          "display_label": "Global",
          "codec": "formid",
          "fields": [
            {
              "id": "global",
              "kind": "formid",
              "display_label": "Global"
            }
          ]
        }
      ],
      "display_label": "Placed Object",
      "record_flags": {
        "valid_mask": 40608,
        "bits": [
          {
            "bit": 7,
            "name": "Turn Off Fire"
          },
          {
            "bit": 9,
            "name": "Cast Shadows"
          },
          {
            "bit": 10,
            "name": "Persistent"
          },
          {
            "bit": 11,
            "name": "Initially Disabled"
          },
          {
            "bit": 15,
            "name": "Visible When Distant"
          }
        ]
      }
    },
    {
      "id": "REGN",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "ICON",
          "kind": "parsed",
          "display_label": "Icon FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "icon_filename",
              "kind": "zstring",
              "display_label": "Icon FileName"
            }
          ],
          "required": true
        },
        {
          "id": "RCLR",
          "kind": "parsed",
          "display_label": "Map Color",
          "codec": "struct:B,B,B,B",
          "fields": [
            {
              "id": "map_color_red",
              "kind": "uint8",
              "display_label": "Map Color Red"
            },
            {
              "id": "map_color_green",
              "kind": "uint8",
              "display_label": "Map Color Green"
            },
            {
              "id": "map_color_blue",
              "kind": "uint8",
              "display_label": "Map Color Blue"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Map Color Unknown Byte 4"
            }
          ],
          "required": true
        },
        {
          "id": "WNAM",
          "kind": "parsed",
          "display_label": "Worldspace",
          "codec": "formid",
          "fields": [
            {
              "id": "worldspace",
              "kind": "formid",
              "display_label": "Worldspace",
              "formlink_target": "WRLD",
              "formlink_targets": [
                "WRLD"
              ]
            }
          ],
          "formlink_target": "WRLD",
          "formlink_targets": [
            "WRLD"
          ]
        },
        {
          "id": "RPLI",
          "kind": "parsed",
          "display_label": "Edge Fall-off",
          "codec": "uint32",
          "fields": [
            {
              "id": "edge_fall_off",
              "kind": "uint32",
              "display_label": "Edge Fall-off"
            }
          ],
          "repeatable": true,
          "scope_id": "region_areas"
        },
        {
          "id": "RPLD",
          "kind": "parsed",
          "display_label": "Points",
          "codec": "array_struct:f,f",
          "fields": [
            {
              "id": "points_x",
              "kind": "float32",
              "display_label": "Points X"
            },
            {
              "id": "points_y",
              "kind": "float32",
              "display_label": "Points Y"
            }
          ],
          "repeatable": true,
          "array": {
            "layout": "row_array",
            "element_codec": "f,f"
          },
          "row_label": "Points",
          "scope_id": "region_areas"
        },
        {
          "id": "ANAM",
          "kind": "raw",
          "repeatable": true,
          "scope_id": "region_areas"
        },
        {
          "id": "RDAT",
          "kind": "parsed_with_raw_fallback",
          "codec": "struct:I,B,B,B,B",
          "fields": [
            {
              "id": "type",
              "kind": "uint32",
              "display_label": "Type",
              "enum_ref": "REGN.RDAT.type"
            },
            {
              "id": "override",
              "kind": "uint8",
              "display_label": "Override",
              "enum_ref": "bool_enum"
            },
            {
              "id": "priority",
              "kind": "uint8",
              "display_label": "Priority"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Unknown Byte 4"
            },
            {
              "id": "unknown_u8_4",
              "kind": "uint8",
              "display_label": "Unknown Byte 5"
            }
          ],
          "repeatable": true,
          "scope_id": "region_data_entries"
        },
        {
          "id": "RDOT",
          "kind": "parsed",
          "display_label": "Objects",
          "codec": "array_struct:I,H,B,B,f,B,B,B,B,H,H,f,f,f,f,f,H,H,H,B,B,B,B,B,B",
          "fields": [
            {
              "id": "objects_object",
              "kind": "formid",
              "display_label": "Objects Object",
              "formlink_targets": [
                "FLOR",
                "LTEX",
                "STAT",
                "TREE"
              ]
            },
            {
              "id": "objects_parent_index",
              "kind": "uint16",
              "display_label": "Objects Parent Index"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Objects Unknown Byte 3"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Objects Unknown Byte 4"
            },
            {
              "id": "objects_density",
              "kind": "float32",
              "display_label": "Objects Density"
            },
            {
              "id": "objects_clustering",
              "kind": "uint8",
              "display_label": "Objects Clustering"
            },
            {
              "id": "objects_min_slope",
              "kind": "uint8",
              "display_label": "Objects Min Slope"
            },
            {
              "id": "objects_max_slope",
              "kind": "uint8",
              "display_label": "Objects Max Slope"
            },
            {
              "id": "objects_flags",
              "kind": "uint8",
              "display_label": "Objects Flags",
              "enum_ref": "REGN.RDOT.objects_flags"
            },
            {
              "id": "objects_radius_wrt_parent",
              "kind": "uint16",
              "display_label": "Objects Radius wrt Parent"
            },
            {
              "id": "objects_radius",
              "kind": "uint16",
              "display_label": "Objects Radius"
            },
            {
              "id": "objects_min_height",
              "kind": "float32",
              "display_label": "Objects Min Height"
            },
            {
              "id": "objects_max_height",
              "kind": "float32",
              "display_label": "Objects Max Height"
            },
            {
              "id": "objects_sink",
              "kind": "float32",
              "display_label": "Objects Sink"
            },
            {
              "id": "objects_sink_variance",
              "kind": "float32",
              "display_label": "Objects Sink Variance"
            },
            {
              "id": "objects_size_variance",
              "kind": "float32",
              "display_label": "Objects Size Variance"
            },
            {
              "id": "objects_angle_variance_x",
              "kind": "uint16",
              "display_label": "Objects Angle Variance X"
            },
            {
              "id": "objects_angle_variance_y",
              "kind": "uint16",
              "display_label": "Objects Angle Variance Y"
            },
            {
              "id": "objects_angle_variance_z",
              "kind": "uint16",
              "display_label": "Objects Angle Variance Z"
            },
            {
              "id": "unknown_u8_19",
              "kind": "uint8",
              "display_label": "Objects Unknown Byte 20"
            },
            {
              "id": "unknown_u8_20",
              "kind": "uint8",
              "display_label": "Objects Unknown Byte 21"
            },
            {
              "id": "unknown_u8_21",
              "kind": "uint8",
              "display_label": "Objects Unknown Byte 22"
            },
            {
              "id": "unknown_u8_22",
              "kind": "uint8",
              "display_label": "Objects Unknown Byte 23"
            },
            {
              "id": "unknown_u8_23",
              "kind": "uint8",
              "display_label": "Objects Unknown Byte 24"
            },
            {
              "id": "unknown_u8_24",
              "kind": "uint8",
              "display_label": "Objects Unknown Byte 25"
            }
          ],
          "repeatable": true,
          "array": {
            "layout": "row_array",
            "element_codec": "I,H,B,B,f,B,B,B,B,H,H,f,f,f,f,f,H,H,H,B,B,B,B,B,B"
          },
          "row_label": "Objects",
          "scope_id": "region_data_entries"
        },
        {
          "id": "RDMP",
          "kind": "parsed",
          "display_label": "Map Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "map_name",
              "kind": "zstring",
              "display_label": "Map Name"
            }
          ],
          "repeatable": true,
          "scope_id": "region_data_entries"
        },
        {
          "id": "RDGS",
          "kind": "parsed",
          "display_label": "Grasses",
          "codec": "array_struct:I,B,B,B,B",
          "fields": [
            {
              "id": "grasses_grass",
              "kind": "formid",
              "display_label": "Grasses Grass",
              "formlink_target": "GRAS",
              "formlink_targets": [
                "GRAS"
              ]
            },
            {
              "id": "unknown_u8_1",
              "kind": "uint8",
              "display_label": "Grasses Unknown Byte 2"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Grasses Unknown Byte 3"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Grasses Unknown Byte 4"
            },
            {
              "id": "unknown_u8_4",
              "kind": "uint8",
              "display_label": "Grasses Unknown Byte 5"
            }
          ],
          "repeatable": true,
          "array": {
            "layout": "row_array",
            "element_codec": "I,B,B,B,B"
          },
          "row_label": "Grasses",
          "scope_id": "region_data_entries"
        },
        {
          "id": "RDMD",
          "kind": "parsed",
          "display_label": "Music Type",
          "codec": "uint32",
          "fields": [
            {
              "id": "music_type",
              "kind": "uint32",
              "display_label": "Music Type",
              "enum_ref": "music_enum"
            }
          ],
          "repeatable": true,
          "enum_ref": "music_enum",
          "scope_id": "region_data_entries"
        },
        {
          "id": "RDWT",
          "kind": "parsed",
          "display_label": "Weather Types",
          "codec": "array_struct:I,I",
          "fields": [
            {
              "id": "weather_types_weather",
              "kind": "formid",
              "display_label": "Weather Types Weather",
              "formlink_target": "WTHR",
              "formlink_targets": [
                "WTHR"
              ]
            },
            {
              "id": "weather_types_chance",
              "kind": "uint32",
              "display_label": "Weather Types Chance"
            }
          ],
          "repeatable": true,
          "array": {
            "layout": "row_array",
            "element_codec": "I,I"
          },
          "row_label": "Weather Types",
          "scope_id": "region_data_entries"
        }
      ],
      "display_label": "Region",
      "record_flags": {
        "valid_mask": 4192,
        "bits": [
          {
            "bit": 6,
            "name": "Border Region"
          }
        ]
      }
    },
    {
      "id": "ROAD",
      "subrecords": [
        {
          "id": "PGRP",
          "kind": "parsed",
          "display_label": "Points",
          "codec": "array_struct:f,f,f,B,B,B,B",
          "fields": [
            {
              "id": "points_x",
              "kind": "float32",
              "display_label": "Points X"
            },
            {
              "id": "points_y",
              "kind": "float32",
              "display_label": "Points Y"
            },
            {
              "id": "points_z_even_red_orange_odd_blue",
              "kind": "float32",
              "display_label": "Points Z (Even = Red/Orange, Odd = Blue)"
            },
            {
              "id": "points_connections",
              "kind": "uint8",
              "display_label": "Points Connections"
            },
            {
              "id": "unknown_u8_4",
              "kind": "uint8",
              "display_label": "Points Unknown Byte 5"
            },
            {
              "id": "unknown_u8_5",
              "kind": "uint8",
              "display_label": "Points Unknown Byte 6"
            },
            {
              "id": "unknown_u8_6",
              "kind": "uint8",
              "display_label": "Points Unknown Byte 7"
            }
          ],
          "required": true,
          "array": {
            "layout": "row_array",
            "element_codec": "f,f,f,B,B,B,B"
          },
          "row_label": "Points"
        },
        {
          "id": "PGRR",
          "kind": "parsed",
          "display_label": "Point-to-Point Connections",
          "codec": "array_struct:",
          "fields": [
            {
              "id": "point",
              "kind": "struct",
              "display_label": "Point",
              "fields": [
                {
                  "id": "x",
                  "kind": "float32",
                  "display_label": "X"
                },
                {
                  "id": "y",
                  "kind": "float32",
                  "display_label": "Y"
                },
                {
                  "id": "z",
                  "kind": "float32",
                  "display_label": "Z"
                }
              ],
              "array": {
                "layout": "row_array",
                "element_codec": "f,f,f"
              }
            }
          ],
          "required": true,
          "array": {
            "layout": "row_array"
          },
          "row_label": "Point-to-Point Connections"
        }
      ],
      "display_label": "Road",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "SBSP",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "DNAM",
          "kind": "parsed",
          "display_label": "Bounds",
          "codec": "struct:f,f,f",
          "fields": [
            {
              "id": "x",
              "kind": "float32",
              "display_label": "X"
            },
            {
              "id": "y",
              "kind": "float32",
              "display_label": "Y"
            },
            {
              "id": "z",
              "kind": "float32",
              "display_label": "Z"
            }
          ],
          "required": true
        }
      ],
      "display_label": "Subspace",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "SCPT",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "SCHD",
          "kind": "parsed",
          "display_label": "Unknown (Script Header?)",
          "codec": "bytes",
          "fields": [
            {
              "id": "unknown_script_header",
              "kind": "bytes",
              "display_label": "Unknown (Script Header?)"
            }
          ]
        },
        {
          "id": "SCHR",
          "kind": "parsed",
          "display_label": "Basic Script Data",
          "codec": "struct:B,B,B,B,I,I,I,I",
          "fields": [
            {
              "id": "unknown_u8_0",
              "kind": "uint8",
              "display_label": "Unknown Byte 1"
            },
            {
              "id": "unknown_u8_1",
              "kind": "uint8",
              "display_label": "Unknown Byte 2"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Unknown Byte 3"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Unknown Byte 4"
            },
            {
              "id": "refcount",
              "kind": "uint32",
              "display_label": "RefCount"
            },
            {
              "id": "compiledsize",
              "kind": "uint32",
              "display_label": "CompiledSize"
            },
            {
              "id": "variablecount",
              "kind": "uint32",
              "display_label": "VariableCount"
            },
            {
              "id": "type",
              "kind": "uint32",
              "display_label": "Type",
              "enum_ref": "SCPT.SCHR.type"
            }
          ],
          "repeatable": true
        },
        {
          "id": "SCHD",
          "kind": "parsed",
          "display_label": "Basic Script Data",
          "codec": "struct:B,B,B,B,I,I,I,I",
          "fields": [
            {
              "id": "unknown_u8_0",
              "kind": "uint8",
              "display_label": "Unknown Byte 1"
            },
            {
              "id": "unknown_u8_1",
              "kind": "uint8",
              "display_label": "Unknown Byte 2"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Unknown Byte 3"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Unknown Byte 4"
            },
            {
              "id": "refcount",
              "kind": "uint32",
              "display_label": "RefCount"
            },
            {
              "id": "compiledsize",
              "kind": "uint32",
              "display_label": "CompiledSize"
            },
            {
              "id": "variablecount",
              "kind": "uint32",
              "display_label": "VariableCount"
            },
            {
              "id": "type",
              "kind": "uint32",
              "display_label": "Type",
              "enum_ref": "SCPT.SCHD.type"
            },
            {
              "id": "unknown",
              "kind": "bytes",
              "display_label": "Unknown"
            }
          ],
          "repeatable": true
        },
        {
          "id": "SCDA",
          "kind": "parsed",
          "display_label": "Compiled Script",
          "codec": "bytes",
          "fields": [
            {
              "id": "compiled_script",
              "kind": "bytes",
              "display_label": "Compiled Script"
            }
          ],
          "required": true
        },
        {
          "id": "SCTX",
          "kind": "raw",
          "display_label": "Script Source",
          "required": true
        },
        {
          "id": "SLSD",
          "kind": "parsed",
          "codec": "struct:I,B,B,B,B,B,B,B,B,B,B,B,B,B",
          "fields": [
            {
              "id": "index",
              "kind": "uint32",
              "display_label": "Index"
            },
            {
              "id": "unknown_u8_1",
              "kind": "uint8",
              "display_label": "Unknown Byte 2"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Unknown Byte 3"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Unknown Byte 4"
            },
            {
              "id": "unknown_u8_4",
              "kind": "uint8",
              "display_label": "Unknown Byte 5"
            },
            {
              "id": "unknown_u8_5",
              "kind": "uint8",
              "display_label": "Unknown Byte 6"
            },
            {
              "id": "unknown_u8_6",
              "kind": "uint8",
              "display_label": "Unknown Byte 7"
            },
            {
              "id": "unknown_u8_7",
              "kind": "uint8",
              "display_label": "Unknown Byte 8"
            },
            {
              "id": "unknown_u8_8",
              "kind": "uint8",
              "display_label": "Unknown Byte 9"
            },
            {
              "id": "unknown_u8_9",
              "kind": "uint8",
              "display_label": "Unknown Byte 10"
            },
            {
              "id": "unknown_u8_10",
              "kind": "uint8",
              "display_label": "Unknown Byte 11"
            },
            {
              "id": "unknown_u8_11",
              "kind": "uint8",
              "display_label": "Unknown Byte 12"
            },
            {
              "id": "unknown_u8_12",
              "kind": "uint8",
              "display_label": "Unknown Byte 13"
            },
            {
              "id": "islongorshort",
              "kind": "uint8",
              "display_label": "IsLongOrShort",
              "enum_ref": "bool_enum"
            },
            {
              "id": "unused",
              "kind": "bytes",
              "display_label": "Unused"
            }
          ],
          "repeatable": true,
          "scope_id": "local_variables"
        },
        {
          "id": "SCVR",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ],
          "repeatable": true,
          "scope_id": "local_variables"
        },
        {
          "id": "SCRO",
          "kind": "parsed",
          "display_label": "Global Reference",
          "codec": "formid",
          "fields": [
            {
              "id": "formid_0",
              "kind": "formid"
            }
          ],
          "repeatable": true,
          "scope_id": "references"
        },
        {
          "id": "SCRV",
          "kind": "parsed",
          "display_label": "Local Variable",
          "codec": "uint32",
          "fields": [
            {
              "id": "local_variable",
              "kind": "uint32",
              "display_label": "Local Variable"
            }
          ],
          "repeatable": true,
          "scope_id": "references"
        }
      ],
      "display_label": "Script",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "SGST",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "model_filename",
              "kind": "zstring",
              "display_label": "Model FileName"
            }
          ]
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ]
        },
        {
          "id": "ICON",
          "kind": "parsed",
          "display_label": "Icon FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "icon_filename",
              "kind": "zstring",
              "display_label": "Icon FileName"
            }
          ],
          "required": true
        },
        {
          "id": "SCRI",
          "kind": "parsed",
          "display_label": "Script",
          "codec": "formid",
          "fields": [
            {
              "id": "script",
              "kind": "formid",
              "display_label": "Script",
              "formlink_target": "SCPT",
              "formlink_targets": [
                "SCPT"
              ]
            }
          ],
          "formlink_target": "SCPT",
          "formlink_targets": [
            "SCPT"
          ]
        },
        {
          "id": "EFID",
          "kind": "parsed",
          "display_label": "Magic Effect Name",
          "codec": "uint32",
          "fields": [
            {
              "id": "magic_effect_name",
              "kind": "uint32",
              "display_label": "Magic Effect Name"
            }
          ],
          "repeatable": true,
          "required": true,
          "scope_id": "effects"
        },
        {
          "id": "EFIT",
          "kind": "parsed",
          "codec": "struct:I,I,I,I,I,i",
          "fields": [
            {
              "id": "magic_effect_name",
              "kind": "uint32",
              "display_label": "Magic Effect Name"
            },
            {
              "id": "magnitude",
              "kind": "uint32",
              "display_label": "Magnitude"
            },
            {
              "id": "area",
              "kind": "uint32",
              "display_label": "Area"
            },
            {
              "id": "duration",
              "kind": "uint32",
              "display_label": "Duration"
            },
            {
              "id": "type",
              "kind": "uint32",
              "display_label": "Type",
              "enum_ref": "effect_type_enum"
            },
            {
              "id": "actor_value",
              "kind": "int32",
              "display_label": "Actor Value",
              "enum_ref": "actor_value_enum"
            }
          ],
          "repeatable": true,
          "required": true,
          "scope_id": "effects"
        },
        {
          "id": "SCIT",
          "kind": "parsed_with_raw_fallback",
          "codec": "struct:I,I,I,B,B,B,B",
          "fields": [
            {
              "id": "script_effect",
              "kind": "formid",
              "display_label": "Script effect",
              "formlink_target": "SCPT",
              "formlink_targets": [
                "SCPT"
              ],
              "null_allowed": true
            },
            {
              "id": "magic_school",
              "kind": "uint32",
              "display_label": "Magic school",
              "enum_ref": "magic_school_enum"
            },
            {
              "id": "visual_effect_name",
              "kind": "uint32",
              "display_label": "Visual effect name"
            },
            {
              "id": "hostile",
              "kind": "uint8",
              "display_label": "Hostile",
              "enum_ref": "bool_enum"
            },
            {
              "id": "unknown_u8_4",
              "kind": "uint8",
              "display_label": "Unknown Byte 5"
            },
            {
              "id": "unknown_u8_5",
              "kind": "uint8",
              "display_label": "Unknown Byte 6"
            },
            {
              "id": "unknown_u8_6",
              "kind": "uint8",
              "display_label": "Unknown Byte 7"
            }
          ],
          "repeatable": true,
          "required": true,
          "scope_id": "effects"
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ],
          "repeatable": true,
          "required": true,
          "scope_id": "effects"
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "display_label": "Data",
          "codec": "struct:B,I,f",
          "fields": [
            {
              "id": "uses",
              "kind": "uint8",
              "display_label": "Uses "
            },
            {
              "id": "value",
              "kind": "uint32",
              "display_label": "Value"
            },
            {
              "id": "weight",
              "kind": "float32",
              "display_label": "Weight"
            }
          ],
          "required": true
        }
      ],
      "display_label": "Sigil Stone",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "SKIL",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "INDX",
          "kind": "parsed",
          "display_label": "Skill",
          "codec": "int32",
          "fields": [
            {
              "id": "skill",
              "kind": "int32",
              "display_label": "Skill",
              "enum_ref": "major_skill_enum"
            }
          ],
          "required": true,
          "enum_ref": "major_skill_enum"
        },
        {
          "id": "DESC",
          "kind": "parsed",
          "display_label": "Description",
          "codec": "zstring",
          "fields": [
            {
              "id": "description",
              "kind": "zstring",
              "display_label": "Description"
            }
          ]
        },
        {
          "id": "ICON",
          "kind": "parsed",
          "display_label": "Icon FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "icon_filename",
              "kind": "zstring",
              "display_label": "Icon FileName"
            }
          ],
          "required": true
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "display_label": "Skill Data",
          "codec": "struct:i,I,I",
          "fields": [
            {
              "id": "action",
              "kind": "int32",
              "display_label": "Action",
              "enum_ref": "major_skill_enum"
            },
            {
              "id": "attribute",
              "kind": "uint32",
              "display_label": "Attribute",
              "enum_ref": "attribute_enum"
            },
            {
              "id": "specialization",
              "kind": "uint32",
              "display_label": "Specialization",
              "enum_ref": "specialization_enum"
            },
            {
              "id": "use_values",
              "kind": "float32",
              "display_label": "Use Values",
              "array": {
                "layout": "row_array",
                "element_codec": "f"
              }
            }
          ],
          "required": true
        },
        {
          "id": "ANAM",
          "kind": "parsed",
          "display_label": "Apprentice Text",
          "codec": "zstring",
          "fields": [
            {
              "id": "apprentice_text",
              "kind": "zstring",
              "display_label": "Apprentice Text"
            }
          ],
          "required": true
        },
        {
          "id": "JNAM",
          "kind": "parsed",
          "display_label": "Journeyman Text",
          "codec": "zstring",
          "fields": [
            {
              "id": "journeyman_text",
              "kind": "zstring",
              "display_label": "Journeyman Text"
            }
          ],
          "required": true
        },
        {
          "id": "ENAM",
          "kind": "parsed",
          "display_label": "Expert Text",
          "codec": "zstring",
          "fields": [
            {
              "id": "expert_text",
              "kind": "zstring",
              "display_label": "Expert Text"
            }
          ],
          "required": true
        },
        {
          "id": "MNAM",
          "kind": "parsed",
          "display_label": "Master Text",
          "codec": "zstring",
          "fields": [
            {
              "id": "master_text",
              "kind": "zstring",
              "display_label": "Master Text"
            }
          ],
          "required": true
        }
      ],
      "display_label": "Skill",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "SLGM",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "model_filename",
              "kind": "zstring",
              "display_label": "Model FileName"
            }
          ]
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ]
        },
        {
          "id": "ICON",
          "kind": "parsed",
          "display_label": "Icon FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "icon_filename",
              "kind": "zstring",
              "display_label": "Icon FileName"
            }
          ],
          "required": true
        },
        {
          "id": "SCRI",
          "kind": "parsed",
          "display_label": "Script",
          "codec": "formid",
          "fields": [
            {
              "id": "script",
              "kind": "formid",
              "display_label": "Script",
              "formlink_target": "SCPT",
              "formlink_targets": [
                "SCPT"
              ]
            }
          ],
          "formlink_target": "SCPT",
          "formlink_targets": [
            "SCPT"
          ]
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "display_label": "Data",
          "codec": "struct:I,f",
          "fields": [
            {
              "id": "value",
              "kind": "uint32",
              "display_label": "Value"
            },
            {
              "id": "weight",
              "kind": "float32",
              "display_label": "Weight"
            }
          ],
          "required": true
        },
        {
          "id": "SOUL",
          "kind": "parsed",
          "display_label": "Contained Soul",
          "codec": "uint8",
          "fields": [
            {
              "id": "contained_soul",
              "kind": "uint8",
              "display_label": "Contained Soul",
              "enum_ref": "soul_gem_enum"
            }
          ],
          "required": true,
          "enum_ref": "soul_gem_enum"
        },
        {
          "id": "SLCP",
          "kind": "parsed",
          "display_label": "Maximum Capacity",
          "codec": "uint8",
          "fields": [
            {
              "id": "maximum_capacity",
              "kind": "uint8",
              "display_label": "Maximum Capacity",
              "enum_ref": "soul_gem_enum"
            }
          ],
          "required": true,
          "enum_ref": "soul_gem_enum"
        }
      ],
      "display_label": "Soul Gem",
      "record_flags": {
        "valid_mask": 5152,
        "bits": [
          {
            "bit": 10,
            "name": "Quest Item"
          }
        ]
      }
    },
    {
      "id": "SOUN",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FNAM",
          "kind": "parsed",
          "display_label": "Sound Filename",
          "codec": "zstring",
          "fields": [
            {
              "id": "sound_filename",
              "kind": "zstring",
              "display_label": "Sound Filename"
            }
          ]
        },
        {
          "id": "SNDX",
          "kind": "raw",
          "display_label": "Sound Data",
          "required": true
        },
        {
          "id": "SNDD",
          "kind": "raw",
          "display_label": "Sound Data"
        }
      ],
      "display_label": "Sound",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "SPEL",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "SPIT",
          "kind": "parsed",
          "display_label": "Data",
          "codec": "struct:B,B,B,B,I,B,B,B,B,B,B,B,B",
          "fields": [
            {
              "id": "type",
              "kind": "uint8",
              "display_label": "Type",
              "enum_ref": "SPEL.SPIT.type"
            },
            {
              "id": "unknown_u8_1",
              "kind": "uint8",
              "display_label": "Unknown Byte 2"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Unknown Byte 3"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Unknown Byte 4"
            },
            {
              "id": "cost",
              "kind": "uint32",
              "display_label": "Cost"
            },
            {
              "id": "level",
              "kind": "uint8",
              "display_label": "Level",
              "enum_ref": "SPEL.SPIT.level"
            },
            {
              "id": "unknown_u8_6",
              "kind": "uint8",
              "display_label": "Unknown Byte 7"
            },
            {
              "id": "unknown_u8_7",
              "kind": "uint8",
              "display_label": "Unknown Byte 8"
            },
            {
              "id": "unknown_u8_8",
              "kind": "uint8",
              "display_label": "Unknown Byte 9"
            },
            {
              "id": "flags",
              "kind": "uint8",
              "display_label": "Flags",
              "enum_ref": "SPEL.SPIT.flags"
            },
            {
              "id": "unknown_u8_10",
              "kind": "uint8",
              "display_label": "Unknown Byte 11"
            },
            {
              "id": "unknown_u8_11",
              "kind": "uint8",
              "display_label": "Unknown Byte 12"
            },
            {
              "id": "unknown_u8_12",
              "kind": "uint8",
              "display_label": "Unknown Byte 13"
            }
          ],
          "required": true
        },
        {
          "id": "EFID",
          "kind": "parsed",
          "display_label": "Magic Effect Name",
          "codec": "uint32",
          "fields": [
            {
              "id": "magic_effect_name",
              "kind": "uint32",
              "display_label": "Magic Effect Name"
            }
          ],
          "repeatable": true,
          "required": true,
          "scope_id": "effects"
        },
        {
          "id": "EFIT",
          "kind": "parsed",
          "codec": "struct:I,I,I,I,I,i",
          "fields": [
            {
              "id": "magic_effect_name",
              "kind": "uint32",
              "display_label": "Magic Effect Name"
            },
            {
              "id": "magnitude",
              "kind": "uint32",
              "display_label": "Magnitude"
            },
            {
              "id": "area",
              "kind": "uint32",
              "display_label": "Area"
            },
            {
              "id": "duration",
              "kind": "uint32",
              "display_label": "Duration"
            },
            {
              "id": "type",
              "kind": "uint32",
              "display_label": "Type",
              "enum_ref": "effect_type_enum"
            },
            {
              "id": "actor_value",
              "kind": "int32",
              "display_label": "Actor Value",
              "enum_ref": "actor_value_enum"
            }
          ],
          "repeatable": true,
          "required": true,
          "scope_id": "effects"
        },
        {
          "id": "SCIT",
          "kind": "parsed_with_raw_fallback",
          "codec": "struct:I,I,I,B,B,B,B",
          "fields": [
            {
              "id": "script_effect",
              "kind": "formid",
              "display_label": "Script effect",
              "formlink_target": "SCPT",
              "formlink_targets": [
                "SCPT"
              ],
              "null_allowed": true
            },
            {
              "id": "magic_school",
              "kind": "uint32",
              "display_label": "Magic school",
              "enum_ref": "magic_school_enum"
            },
            {
              "id": "visual_effect_name",
              "kind": "uint32",
              "display_label": "Visual effect name"
            },
            {
              "id": "hostile",
              "kind": "uint8",
              "display_label": "Hostile",
              "enum_ref": "bool_enum"
            },
            {
              "id": "unknown_u8_4",
              "kind": "uint8",
              "display_label": "Unknown Byte 5"
            },
            {
              "id": "unknown_u8_5",
              "kind": "uint8",
              "display_label": "Unknown Byte 6"
            },
            {
              "id": "unknown_u8_6",
              "kind": "uint8",
              "display_label": "Unknown Byte 7"
            }
          ],
          "repeatable": true,
          "required": true,
          "scope_id": "effects"
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ],
          "repeatable": true,
          "required": true,
          "scope_id": "effects"
        }
      ],
      "display_label": "Spell",
      "record_flags": {
        "valid_mask": 4128
      }
    },
    {
      "id": "STAT",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "model_filename",
              "kind": "zstring",
              "display_label": "Model FileName"
            }
          ]
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ]
        },
        {
          "id": "DMTL",
          "kind": "parsed",
          "display_label": "Distant Model Texture List",
          "codec": "array_struct:Q,Q,Q",
          "fields": [
            {
              "id": "distant_model_texture_list_file_hash_pc",
              "kind": "uint64",
              "display_label": "Distant Model Texture List File Hash (PC)"
            },
            {
              "id": "distant_model_texture_list_file_hash_console",
              "kind": "uint64",
              "display_label": "Distant Model Texture List File Hash (Console)"
            },
            {
              "id": "distant_model_texture_list_folder_hash",
              "kind": "uint64",
              "display_label": "Distant Model Texture List Folder Hash"
            }
          ],
          "array": {
            "layout": "row_array",
            "element_codec": "Q,Q,Q"
          },
          "row_label": "Distant Model Texture List"
        }
      ],
      "display_label": "Static",
      "record_flags": {
        "valid_mask": 5152,
        "bits": [
          {
            "bit": 10,
            "name": "Quest Item"
          }
        ]
      }
    },
    {
      "id": "TES4",
      "subrecords": [
        {
          "id": "HEDR",
          "kind": "parsed",
          "codec": "struct:f,i,i",
          "required": true
        },
        {
          "id": "CNAM",
          "kind": "parsed",
          "codec": "zstring"
        },
        {
          "id": "SNAM",
          "kind": "parsed",
          "codec": "zstring"
        },
        {
          "id": "MAST",
          "kind": "parsed",
          "codec": "zstring",
          "repeatable": true
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "codec": "int64",
          "repeatable": true
        },
        {
          "id": "ONAM",
          "kind": "parsed",
          "codec": "formid",
          "repeatable": true
        },
        {
          "id": "INTV",
          "kind": "parsed",
          "codec": "uint32"
        },
        {
          "id": "INCC",
          "kind": "parsed",
          "codec": "uint32"
        }
      ],
      "order_hint": [
        "HEDR",
        "CNAM",
        "SNAM",
        "MAST",
        "DATA",
        "ONAM",
        "INTV",
        "INCC"
      ]
    },
    {
      "id": "TREE",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "SPT File FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "spt_file_filename",
              "kind": "zstring",
              "display_label": "SPT File FileName"
            }
          ]
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ]
        },
        {
          "id": "ICON",
          "kind": "parsed",
          "display_label": "Leaf Texture",
          "codec": "zstring",
          "fields": [
            {
              "id": "leaf_texture",
              "kind": "zstring",
              "display_label": "Leaf Texture"
            }
          ]
        },
        {
          "id": "SNAM",
          "kind": "parsed",
          "display_label": "SpeedTree Seeds",
          "codec": "array_struct:I",
          "fields": [
            {
              "id": "speedtree_seeds_speedtree_seed",
              "kind": "uint32",
              "display_label": "SpeedTree Seeds SpeedTree Seed"
            }
          ],
          "array": {
            "layout": "row_array",
            "element_codec": "I"
          },
          "row_label": "SpeedTree Seeds"
        },
        {
          "id": "CNAM",
          "kind": "parsed",
          "display_label": "Tree Data",
          "codec": "struct:f,f,f,f,f,i,f,f",
          "fields": [
            {
              "id": "leaf_curvature",
              "kind": "float32",
              "display_label": "Leaf Curvature"
            },
            {
              "id": "minimum_leaf_angle",
              "kind": "float32",
              "display_label": "Minimum Leaf Angle"
            },
            {
              "id": "maximum_leaf_angle",
              "kind": "float32",
              "display_label": "Maximum Leaf Angle"
            },
            {
              "id": "branch_dimming_value",
              "kind": "float32",
              "display_label": "Branch Dimming Value"
            },
            {
              "id": "leaf_dimming_value",
              "kind": "float32",
              "display_label": "Leaf Dimming Value"
            },
            {
              "id": "shadow_radius",
              "kind": "int32",
              "display_label": "Shadow Radius"
            },
            {
              "id": "rock_speed",
              "kind": "float32",
              "display_label": "Rock Speed"
            },
            {
              "id": "rustle_speed",
              "kind": "float32",
              "display_label": "Rustle Speed"
            }
          ],
          "required": true
        },
        {
          "id": "BNAM",
          "kind": "parsed",
          "display_label": "Billboard Dimensions",
          "codec": "struct:f,f",
          "fields": [
            {
              "id": "width",
              "kind": "float32",
              "display_label": "Width"
            },
            {
              "id": "height",
              "kind": "float32",
              "display_label": "Height"
            }
          ],
          "required": true
        }
      ],
      "display_label": "Tree",
      "record_flags": {
        "valid_mask": 5152,
        "bits": [
          {
            "bit": 10,
            "name": "Quest Item"
          }
        ]
      }
    },
    {
      "id": "WATR",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "TNAM",
          "kind": "parsed",
          "display_label": "Texture",
          "codec": "zstring",
          "fields": [
            {
              "id": "texture",
              "kind": "zstring",
              "display_label": "Texture"
            }
          ],
          "required": true
        },
        {
          "id": "ANAM",
          "kind": "parsed",
          "display_label": "Opacity",
          "codec": "uint8",
          "fields": [
            {
              "id": "opacity",
              "kind": "uint8",
              "display_label": "Opacity"
            }
          ],
          "required": true
        },
        {
          "id": "FNAM",
          "kind": "parsed",
          "display_label": "Flags",
          "codec": "uint8",
          "fields": [
            {
              "id": "flags",
              "kind": "uint8",
              "display_label": "Flags",
              "enum_ref": "WATR.FNAM.flags"
            }
          ],
          "required": true,
          "enum_ref": "WATR.FNAM.flags"
        },
        {
          "id": "MNAM",
          "kind": "parsed",
          "display_label": "Material ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "material_id",
              "kind": "zstring",
              "display_label": "Material ID"
            }
          ],
          "required": true
        },
        {
          "id": "SNAM",
          "kind": "parsed",
          "display_label": "Sound",
          "codec": "formid",
          "fields": [
            {
              "id": "sound",
              "kind": "formid",
              "display_label": "Sound",
              "formlink_target": "SOUN",
              "formlink_targets": [
                "SOUN"
              ]
            }
          ],
          "formlink_target": "SOUN",
          "formlink_targets": [
            "SOUN"
          ]
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "display_label": "Data",
          "codec": "struct:f,f,f,f,f,f,f,f,f,f,f,B,B,B,B,B,B,B,B,B,B,B,B,B,B,B,B,f,f,f,f,f,f,f,f,f,f,H",
          "fields": [
            {
              "id": "wind_velocity",
              "kind": "float32",
              "display_label": "Wind Velocity"
            },
            {
              "id": "wind_direction",
              "kind": "float32",
              "display_label": "Wind Direction"
            },
            {
              "id": "wave_amplitude",
              "kind": "float32",
              "display_label": "Wave Amplitude"
            },
            {
              "id": "wave_frequency",
              "kind": "float32",
              "display_label": "Wave Frequency"
            },
            {
              "id": "sun_power",
              "kind": "float32",
              "display_label": "Sun Power"
            },
            {
              "id": "reflectivity_amount",
              "kind": "float32",
              "display_label": "Reflectivity Amount"
            },
            {
              "id": "fresnel_amount",
              "kind": "float32",
              "display_label": "Fresnel Amount"
            },
            {
              "id": "scroll_x_speed",
              "kind": "float32",
              "display_label": "Scroll X Speed"
            },
            {
              "id": "scroll_y_speed",
              "kind": "float32",
              "display_label": "Scroll Y Speed"
            },
            {
              "id": "fog_distance_near",
              "kind": "float32",
              "display_label": "Fog Distance Near"
            },
            {
              "id": "fog_distance_far",
              "kind": "float32",
              "display_label": "Fog Distance Far"
            },
            {
              "id": "shallow_color_red",
              "kind": "uint8",
              "display_label": "Shallow Color Red"
            },
            {
              "id": "shallow_color_green",
              "kind": "uint8",
              "display_label": "Shallow Color Green"
            },
            {
              "id": "shallow_color_blue",
              "kind": "uint8",
              "display_label": "Shallow Color Blue"
            },
            {
              "id": "unknown_u8_14",
              "kind": "uint8",
              "display_label": "Shallow Color Unknown Byte 15"
            },
            {
              "id": "deep_color_red",
              "kind": "uint8",
              "display_label": "Deep Color Red"
            },
            {
              "id": "deep_color_green",
              "kind": "uint8",
              "display_label": "Deep Color Green"
            },
            {
              "id": "deep_color_blue",
              "kind": "uint8",
              "display_label": "Deep Color Blue"
            },
            {
              "id": "unknown_u8_18",
              "kind": "uint8",
              "display_label": "Deep Color Unknown Byte 19"
            },
            {
              "id": "reflection_color_red",
              "kind": "uint8",
              "display_label": "Reflection Color Red"
            },
            {
              "id": "reflection_color_green",
              "kind": "uint8",
              "display_label": "Reflection Color Green"
            },
            {
              "id": "reflection_color_blue",
              "kind": "uint8",
              "display_label": "Reflection Color Blue"
            },
            {
              "id": "unknown_u8_22",
              "kind": "uint8",
              "display_label": "Reflection Color Unknown Byte 23"
            },
            {
              "id": "texture_blend",
              "kind": "uint8",
              "display_label": "Texture Blend"
            },
            {
              "id": "unknown_u8_24",
              "kind": "uint8",
              "display_label": "Unknown Byte 25"
            },
            {
              "id": "unknown_u8_25",
              "kind": "uint8",
              "display_label": "Unknown Byte 26"
            },
            {
              "id": "unknown_u8_26",
              "kind": "uint8",
              "display_label": "Unknown Byte 27"
            },
            {
              "id": "rain_simulator_force",
              "kind": "float32",
              "display_label": "Rain Simulator Force"
            },
            {
              "id": "rain_simulator_velocity",
              "kind": "float32",
              "display_label": "Rain Simulator Velocity"
            },
            {
              "id": "rain_simulator_falloff",
              "kind": "float32",
              "display_label": "Rain Simulator Falloff"
            },
            {
              "id": "rain_simulator_dampner",
              "kind": "float32",
              "display_label": "Rain Simulator Dampner"
            },
            {
              "id": "rain_simulator_starting_size",
              "kind": "float32",
              "display_label": "Rain Simulator Starting Size"
            },
            {
              "id": "displacement_simulator_force",
              "kind": "float32",
              "display_label": "Displacement Simulator Force"
            },
            {
              "id": "displacement_simulator_velocity",
              "kind": "float32",
              "display_label": "Displacement Simulator Velocity"
            },
            {
              "id": "displacement_simulator_falloff",
              "kind": "float32",
              "display_label": "Displacement Simulator Falloff"
            },
            {
              "id": "displacement_simulator_dampner",
              "kind": "float32",
              "display_label": "Displacement Simulator Dampner"
            },
            {
              "id": "displacement_simulator_starting_size",
              "kind": "float32",
              "display_label": "Displacement Simulator Starting Size"
            },
            {
              "id": "damage",
              "kind": "uint16",
              "display_label": "Damage"
            }
          ]
        },
        {
          "id": "GNAM",
          "kind": "parsed",
          "display_label": "Related Waters",
          "codec": "struct:I,I,I",
          "fields": [
            {
              "id": "daytime",
              "kind": "formid",
              "display_label": "Daytime",
              "formlink_target": "WATR",
              "formlink_targets": [
                "WATR"
              ],
              "null_allowed": true
            },
            {
              "id": "nighttime",
              "kind": "formid",
              "display_label": "Nighttime",
              "formlink_target": "WATR",
              "formlink_targets": [
                "WATR"
              ],
              "null_allowed": true
            },
            {
              "id": "underwater",
              "kind": "formid",
              "display_label": "Underwater",
              "formlink_target": "WATR",
              "formlink_targets": [
                "WATR"
              ],
              "null_allowed": true
            }
          ],
          "required": true
        }
      ],
      "display_label": "Water",
      "record_flags": {
        "valid_mask": 5152,
        "bits": [
          {
            "bit": 10,
            "name": "Quest Item"
          }
        ]
      }
    },
    {
      "id": "WEAP",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "model_filename",
              "kind": "zstring",
              "display_label": "Model FileName"
            }
          ]
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ]
        },
        {
          "id": "ICON",
          "kind": "parsed",
          "display_label": "Icon FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "icon_filename",
              "kind": "zstring",
              "display_label": "Icon FileName"
            }
          ],
          "required": true
        },
        {
          "id": "SCRI",
          "kind": "parsed",
          "display_label": "Script",
          "codec": "formid",
          "fields": [
            {
              "id": "script",
              "kind": "formid",
              "display_label": "Script",
              "formlink_target": "SCPT",
              "formlink_targets": [
                "SCPT"
              ]
            }
          ],
          "formlink_target": "SCPT",
          "formlink_targets": [
            "SCPT"
          ]
        },
        {
          "id": "EITM",
          "kind": "parsed",
          "display_label": "Effect",
          "codec": "formid",
          "fields": [
            {
              "id": "effect",
              "kind": "formid",
              "display_label": "Effect",
              "formlink_target": "ENCH",
              "formlink_targets": [
                "ENCH"
              ]
            }
          ],
          "repeatable": true,
          "formlink_target": "ENCH",
          "formlink_targets": [
            "ENCH"
          ],
          "scope_id": "enchantment"
        },
        {
          "id": "EAMT",
          "kind": "parsed",
          "display_label": "Capacity",
          "codec": "uint16",
          "fields": [
            {
              "id": "capacity",
              "kind": "uint16",
              "display_label": "Capacity"
            }
          ],
          "repeatable": true,
          "scope_id": "enchantment"
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "display_label": "Data",
          "codec": "struct:B,B,B,B,f,f,I,I,I,f,H",
          "fields": [
            {
              "id": "type",
              "kind": "uint8",
              "display_label": "Type",
              "enum_ref": "WEAP.DATA.type"
            },
            {
              "id": "unknown_u8_1",
              "kind": "uint8",
              "display_label": "Unknown Byte 2"
            },
            {
              "id": "unknown_u8_2",
              "kind": "uint8",
              "display_label": "Unknown Byte 3"
            },
            {
              "id": "unknown_u8_3",
              "kind": "uint8",
              "display_label": "Unknown Byte 4"
            },
            {
              "id": "speed",
              "kind": "float32",
              "display_label": "Speed"
            },
            {
              "id": "reach",
              "kind": "float32",
              "display_label": "Reach"
            },
            {
              "id": "ignores_normal_weapon_resistance",
              "kind": "uint32",
              "display_label": "Ignores Normal Weapon Resistance",
              "enum_ref": "bool_enum"
            },
            {
              "id": "value",
              "kind": "uint32",
              "display_label": "Value"
            },
            {
              "id": "health",
              "kind": "uint32",
              "display_label": "Health"
            },
            {
              "id": "weight",
              "kind": "float32",
              "display_label": "Weight"
            },
            {
              "id": "damage",
              "kind": "uint16",
              "display_label": "Damage"
            }
          ],
          "required": true
        }
      ],
      "display_label": "Weapon",
      "record_flags": {
        "valid_mask": 5152,
        "bits": [
          {
            "bit": 10,
            "name": "Quest Item"
          }
        ]
      }
    },
    {
      "id": "WRLD",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "FULL",
          "kind": "parsed",
          "display_label": "Name",
          "codec": "zstring",
          "fields": [
            {
              "id": "name",
              "kind": "zstring",
              "display_label": "Name"
            }
          ]
        },
        {
          "id": "WNAM",
          "kind": "parsed",
          "display_label": "Parent Worldspace",
          "codec": "formid",
          "fields": [
            {
              "id": "parent_worldspace",
              "kind": "formid",
              "display_label": "Parent Worldspace",
              "formlink_target": "WRLD",
              "formlink_targets": [
                "WRLD"
              ]
            }
          ],
          "formlink_target": "WRLD",
          "formlink_targets": [
            "WRLD"
          ]
        },
        {
          "id": "CNAM",
          "kind": "parsed",
          "display_label": "Climate",
          "codec": "formid",
          "fields": [
            {
              "id": "climate",
              "kind": "formid",
              "display_label": "Climate",
              "formlink_target": "CLMT",
              "formlink_targets": [
                "CLMT"
              ]
            }
          ],
          "formlink_target": "CLMT",
          "formlink_targets": [
            "CLMT"
          ]
        },
        {
          "id": "NAM2",
          "kind": "parsed",
          "display_label": "Water",
          "codec": "formid",
          "fields": [
            {
              "id": "water",
              "kind": "formid",
              "display_label": "Water",
              "formlink_target": "WATR",
              "formlink_targets": [
                "WATR"
              ]
            }
          ],
          "formlink_target": "WATR",
          "formlink_targets": [
            "WATR"
          ]
        },
        {
          "id": "ICON",
          "kind": "parsed",
          "display_label": "Map Image",
          "codec": "zstring",
          "fields": [
            {
              "id": "map_image",
              "kind": "zstring",
              "display_label": "Map Image"
            }
          ]
        },
        {
          "id": "MNAM",
          "kind": "parsed",
          "display_label": "World Map Data",
          "codec": "struct:i,i,h,h,h,h",
          "fields": [
            {
              "id": "usable_dimensions_x",
              "kind": "int32",
              "display_label": "Usable Dimensions X"
            },
            {
              "id": "usable_dimensions_y",
              "kind": "int32",
              "display_label": "Usable Dimensions Y"
            },
            {
              "id": "cell_coordinates_nw_cell_x",
              "kind": "int16",
              "display_label": "Cell Coordinates NW Cell X"
            },
            {
              "id": "cell_coordinates_nw_cell_y",
              "kind": "int16",
              "display_label": "Cell Coordinates NW Cell Y"
            },
            {
              "id": "cell_coordinates_se_cell_x",
              "kind": "int16",
              "display_label": "Cell Coordinates SE Cell X"
            },
            {
              "id": "cell_coordinates_se_cell_y",
              "kind": "int16",
              "display_label": "Cell Coordinates SE Cell Y"
            }
          ]
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "display_label": "Flags",
          "codec": "uint8",
          "fields": [
            {
              "id": "flags",
              "kind": "uint8",
              "display_label": "Flags",
              "enum_ref": "WRLD.DATA.flags"
            }
          ],
          "required": true,
          "enum_ref": "WRLD.DATA.flags"
        },
        {
          "id": "NAM0",
          "kind": "parsed",
          "display_label": "Min",
          "codec": "struct:f,f",
          "fields": [
            {
              "id": "x",
              "kind": "float32",
              "display_label": "X"
            },
            {
              "id": "y",
              "kind": "float32",
              "display_label": "Y"
            }
          ],
          "repeatable": true,
          "required": true,
          "scope_id": "worldspace_bounds"
        },
        {
          "id": "NAM9",
          "kind": "parsed",
          "display_label": "Max",
          "codec": "struct:f,f",
          "fields": [
            {
              "id": "x",
              "kind": "float32",
              "display_label": "X"
            },
            {
              "id": "y",
              "kind": "float32",
              "display_label": "Y"
            }
          ],
          "repeatable": true,
          "required": true,
          "scope_id": "worldspace_bounds"
        },
        {
          "id": "SNAM",
          "kind": "parsed",
          "display_label": "Music",
          "codec": "uint32",
          "fields": [
            {
              "id": "music",
              "kind": "uint32",
              "display_label": "Music",
              "enum_ref": "music_enum"
            }
          ],
          "enum_ref": "music_enum"
        },
        {
          "id": "OFST",
          "kind": "parsed",
          "display_label": "Offsets",
          "codec": "array_struct:",
          "fields": [
            {
              "id": "row",
              "kind": "uint32",
              "display_label": "Row",
              "array": {
                "layout": "row_array",
                "element_codec": "I"
              }
            }
          ],
          "array": {
            "layout": "row_array"
          },
          "row_label": "Offsets"
        }
      ],
      "display_label": "Worldspace",
      "record_flags": {
        "valid_mask": 528416,
        "bits": [
          {
            "bit": 19,
            "name": "Can't Wait"
          }
        ]
      }
    },
    {
      "id": "WTHR",
      "subrecords": [
        {
          "id": "EDID",
          "kind": "parsed",
          "display_label": "Editor ID",
          "codec": "zstring",
          "fields": [
            {
              "id": "editor_id",
              "kind": "zstring",
              "display_label": "Editor ID"
            }
          ]
        },
        {
          "id": "CNAM",
          "kind": "parsed",
          "display_label": "Cloud Texture Lower Layer",
          "codec": "zstring",
          "fields": [
            {
              "id": "cloud_texture_lower_layer",
              "kind": "zstring",
              "display_label": "Cloud Texture Lower Layer"
            }
          ]
        },
        {
          "id": "DNAM",
          "kind": "parsed",
          "display_label": "Cloud Texture Upper Layer",
          "codec": "zstring",
          "fields": [
            {
              "id": "cloud_texture_upper_layer",
              "kind": "zstring",
              "display_label": "Cloud Texture Upper Layer"
            }
          ]
        },
        {
          "id": "MODL",
          "kind": "parsed",
          "display_label": "Precipitation Model FileName",
          "codec": "zstring",
          "fields": [
            {
              "id": "precipitation_model_filename",
              "kind": "zstring",
              "display_label": "Precipitation Model FileName"
            }
          ]
        },
        {
          "id": "MODB",
          "kind": "parsed",
          "codec": "bytes",
          "fields": [
            {
              "id": "model_information",
              "kind": "bytes",
              "display_label": "Model Information"
            }
          ]
        },
        {
          "id": "NAM0",
          "kind": "raw",
          "display_label": "Weather Colors",
          "required": true
        },
        {
          "id": "FNAM",
          "kind": "raw",
          "display_label": "Fog Distance",
          "required": true
        },
        {
          "id": "HNAM",
          "kind": "parsed",
          "display_label": "HDR Data",
          "codec": "struct:f,f,f,f,f,f,f,f,f,f,f,f,f,f",
          "fields": [
            {
              "id": "eye_adapt_speed",
              "kind": "float32",
              "display_label": "Eye Adapt Speed"
            },
            {
              "id": "blur_radius",
              "kind": "float32",
              "display_label": "Blur Radius"
            },
            {
              "id": "blur_passes",
              "kind": "float32",
              "display_label": "Blur Passes"
            },
            {
              "id": "emissive_mult",
              "kind": "float32",
              "display_label": "Emissive Mult"
            },
            {
              "id": "target_lum",
              "kind": "float32",
              "display_label": "Target LUM"
            },
            {
              "id": "upper_lum_clamp",
              "kind": "float32",
              "display_label": "Upper LUM Clamp"
            },
            {
              "id": "bright_scale",
              "kind": "float32",
              "display_label": "Bright Scale"
            },
            {
              "id": "bright_clamp",
              "kind": "float32",
              "display_label": "Bright Clamp"
            },
            {
              "id": "lum_ramp_no_tex",
              "kind": "float32",
              "display_label": "LUM Ramp No Tex"
            },
            {
              "id": "lum_ramp_min",
              "kind": "float32",
              "display_label": "LUM Ramp Min"
            },
            {
              "id": "lum_ramp_max",
              "kind": "float32",
              "display_label": "LUM Ramp Max"
            },
            {
              "id": "sunlight_dimmer",
              "kind": "float32",
              "display_label": "Sunlight Dimmer"
            },
            {
              "id": "grass_dimmer",
              "kind": "float32",
              "display_label": "Grass Dimmer"
            },
            {
              "id": "tree_dimmer",
              "kind": "float32",
              "display_label": "Tree Dimmer"
            }
          ],
          "required": true
        },
        {
          "id": "DATA",
          "kind": "parsed",
          "display_label": "Data",
          "codec": "struct:B,B,B,B,B,B,B,B,B,B,B,B,B,B,B",
          "fields": [
            {
              "id": "wind_speed",
              "kind": "uint8",
              "display_label": "Wind Speed"
            },
            {
              "id": "cloud_speed_lower",
              "kind": "uint8",
              "display_label": "Cloud Speed (Lower)"
            },
            {
              "id": "cloud_speed_upper",
              "kind": "uint8",
              "display_label": "Cloud Speed (Upper)"
            },
            {
              "id": "trans_delta",
              "kind": "uint8",
              "display_label": "Trans Delta"
            },
            {
              "id": "sun_glare",
              "kind": "uint8",
              "display_label": "Sun Glare"
            },
            {
              "id": "sun_damage",
              "kind": "uint8",
              "display_label": "Sun Damage"
            },
            {
              "id": "precipitation_begin_fade_in",
              "kind": "uint8",
              "display_label": "Precipitation - Begin Fade In"
            },
            {
              "id": "precipitation_end_fade_out",
              "kind": "uint8",
              "display_label": "Precipitation - End Fade Out"
            },
            {
              "id": "thunder_lightning_begin_fade_in",
              "kind": "uint8",
              "display_label": "Thunder/Lightning - Begin Fade In"
            },
            {
              "id": "thunder_lightning_end_fade_out",
              "kind": "uint8",
              "display_label": "Thunder/Lightning - End Fade Out"
            },
            {
              "id": "thunder_lightning_frequency",
              "kind": "uint8",
              "display_label": "Thunder/Lightning - Frequency"
            },
            {
              "id": "flags",
              "kind": "uint8",
              "display_label": "Flags ",
              "enum_ref": "WTHR.DATA.flags"
            },
            {
              "id": "lightning_color_red",
              "kind": "uint8",
              "display_label": "Lightning Color Red"
            },
            {
              "id": "lightning_color_green",
              "kind": "uint8",
              "display_label": "Lightning Color Green"
            },
            {
              "id": "lightning_color_blue",
              "kind": "uint8",
              "display_label": "Lightning Color Blue"
            }
          ],
          "required": true
        },
        {
          "id": "SNAM",
          "kind": "parsed",
          "display_label": "Sound",
          "codec": "struct:I,I",
          "fields": [
            {
              "id": "sound",
              "kind": "formid",
              "display_label": "Sound",
              "formlink_targets": [
                "SNDR",
                "SOUN"
              ],
              "null_allowed": true
            },
            {
              "id": "type",
              "kind": "uint32",
              "display_label": "Type",
              "enum_ref": "WTHR.SNAM.type"
            }
          ],
          "repeatable": true
        }
      ],
      "display_label": "Weather",
      "record_flags": {
        "valid_mask": 4128
      }
    }
  ],
  "enums": [
    {
      "id": "ALCH.ENIT.flags",
      "values": [
        {
          "value": 1,
          "id": "no_auto_calculate"
        },
        {
          "value": 2,
          "id": "food_item"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "No Auto-Calculate"
        },
        {
          "value": 2,
          "label": "Food Item"
        }
      ],
      "storage_kind": "flags",
      "byte_width": 1
    },
    {
      "id": "APPA.DATA.type",
      "values": [
        {
          "value": 0,
          "id": "mortar_pestle"
        },
        {
          "value": 1,
          "id": "alembic"
        },
        {
          "value": 2,
          "id": "calcinator"
        },
        {
          "value": 3,
          "id": "retort"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "Mortar & Pestle"
        },
        {
          "value": 1,
          "label": "Alembic"
        },
        {
          "value": 2,
          "label": "Calcinator"
        },
        {
          "value": 3,
          "label": "Retort"
        }
      ],
      "byte_width": 1,
      "default_value": 0
    },
    {
      "id": "ARMO.BMDT.general_flags",
      "values": [
        {
          "value": 1,
          "id": "hide_rings"
        },
        {
          "value": 2,
          "id": "hide_amulets"
        },
        {
          "value": 64,
          "id": "non_playable"
        },
        {
          "value": 128,
          "id": "heavy_armor"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Hide Rings"
        },
        {
          "value": 2,
          "label": "Hide Amulets"
        },
        {
          "value": 64,
          "label": "Non-Playable"
        },
        {
          "value": 128,
          "label": "Heavy armor"
        }
      ],
      "storage_kind": "flags",
      "byte_width": 1
    },
    {
      "id": "BOOK.DATA.flags",
      "values": [
        {
          "value": 1,
          "id": "scroll"
        },
        {
          "value": 2,
          "id": "can_t_be_taken"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Scroll"
        },
        {
          "value": 2,
          "label": "Can't be taken"
        }
      ],
      "storage_kind": "flags",
      "byte_width": 1
    },
    {
      "id": "CELL.DATA.flags",
      "values": [
        {
          "value": 1,
          "id": "is_interior_cell"
        },
        {
          "value": 2,
          "id": "has_water"
        },
        {
          "value": 4,
          "id": "can_t_travel_from_here"
        },
        {
          "value": 8,
          "id": "force_hide_land_exterior_oblivion_interior_interior"
        },
        {
          "value": 32,
          "id": "public_area"
        },
        {
          "value": 64,
          "id": "hand_changed"
        },
        {
          "value": 128,
          "id": "behave_like_exterior"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Is Interior Cell"
        },
        {
          "value": 2,
          "label": "Has Water"
        },
        {
          "value": 4,
          "label": "Can't Travel From Here"
        },
        {
          "value": 8,
          "label": "Force Hide Land (Exterior) / Oblivion Interior (Interior)"
        },
        {
          "value": 32,
          "label": "Public Area"
        },
        {
          "value": 64,
          "label": "Hand Changed"
        },
        {
          "value": 128,
          "label": "Behave Like Exterior"
        }
      ],
      "storage_kind": "flags",
      "byte_width": 1
    },
    {
      "id": "CLAS.DATA.flags",
      "values": [
        {
          "value": 1,
          "id": "playable"
        },
        {
          "value": 2,
          "id": "guard"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Playable"
        },
        {
          "value": 2,
          "label": "Guard"
        }
      ],
      "storage_kind": "flags"
    },
    {
      "id": "CLOT.BMDT.general_flags",
      "values": [
        {
          "value": 1,
          "id": "hide_rings"
        },
        {
          "value": 2,
          "id": "hide_amulets"
        },
        {
          "value": 64,
          "id": "non_playable"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Hide Rings"
        },
        {
          "value": 2,
          "label": "Hide Amulets"
        },
        {
          "value": 64,
          "label": "Non-Playable"
        }
      ],
      "storage_kind": "flags",
      "byte_width": 1
    },
    {
      "id": "CONT.DATA.flags",
      "values": [
        {
          "value": 2,
          "id": "respawns"
        }
      ],
      "labels": [
        {
          "value": 2,
          "label": "Respawns"
        }
      ],
      "storage_kind": "flags",
      "byte_width": 1
    },
    {
      "id": "CREA.ACBS.flags",
      "values": [
        {
          "value": 1,
          "id": "biped"
        },
        {
          "value": 2,
          "id": "essential"
        },
        {
          "value": 4,
          "id": "weapon_shield"
        },
        {
          "value": 8,
          "id": "respawn"
        },
        {
          "value": 16,
          "id": "swims"
        },
        {
          "value": 32,
          "id": "flies"
        },
        {
          "value": 64,
          "id": "walks"
        },
        {
          "value": 128,
          "id": "pc_level_offset"
        },
        {
          "value": 512,
          "id": "no_low_level_processing"
        },
        {
          "value": 2048,
          "id": "no_blood_spray"
        },
        {
          "value": 4096,
          "id": "no_blood_decal"
        },
        {
          "value": 32768,
          "id": "no_head"
        },
        {
          "value": 65536,
          "id": "no_right_arm"
        },
        {
          "value": 131072,
          "id": "no_left_arm"
        },
        {
          "value": 262144,
          "id": "no_combat_in_water"
        },
        {
          "value": 524288,
          "id": "no_shadow"
        },
        {
          "value": 1048576,
          "id": "no_corpse_check"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Biped"
        },
        {
          "value": 2,
          "label": "Essential"
        },
        {
          "value": 4,
          "label": "Weapon & Shield"
        },
        {
          "value": 8,
          "label": "Respawn"
        },
        {
          "value": 16,
          "label": "Swims"
        },
        {
          "value": 32,
          "label": "Flies"
        },
        {
          "value": 64,
          "label": "Walks"
        },
        {
          "value": 128,
          "label": "PC Level Offset"
        },
        {
          "value": 512,
          "label": "No Low Level Processing"
        },
        {
          "value": 2048,
          "label": "No Blood Spray"
        },
        {
          "value": 4096,
          "label": "No Blood Decal"
        },
        {
          "value": 32768,
          "label": "No Head"
        },
        {
          "value": 65536,
          "label": "No Right Arm"
        },
        {
          "value": 131072,
          "label": "No Left Arm"
        },
        {
          "value": 262144,
          "label": "No Combat in Water"
        },
        {
          "value": 524288,
          "label": "No Shadow"
        },
        {
          "value": 1048576,
          "label": "No Corpse Check"
        }
      ],
      "storage_kind": "flags"
    },
    {
      "id": "CREA.CSDT.type",
      "values": [
        {
          "value": 0,
          "id": "left_foot"
        },
        {
          "value": 1,
          "id": "right_foot"
        },
        {
          "value": 2,
          "id": "left_back_foot"
        },
        {
          "value": 3,
          "id": "right_back_foot"
        },
        {
          "value": 4,
          "id": "idle"
        },
        {
          "value": 5,
          "id": "aware"
        },
        {
          "value": 6,
          "id": "attack"
        },
        {
          "value": 7,
          "id": "hit"
        },
        {
          "value": 8,
          "id": "death"
        },
        {
          "value": 9,
          "id": "weapon"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "Left Foot"
        },
        {
          "value": 1,
          "label": "Right Foot"
        },
        {
          "value": 2,
          "label": "Left Back Foot"
        },
        {
          "value": 3,
          "label": "Right Back Foot"
        },
        {
          "value": 4,
          "label": "Idle"
        },
        {
          "value": 5,
          "label": "Aware"
        },
        {
          "value": 6,
          "label": "Attack"
        },
        {
          "value": 7,
          "label": "Hit"
        },
        {
          "value": 8,
          "label": "Death"
        },
        {
          "value": 9,
          "label": "Weapon"
        }
      ],
      "default_value": 0
    },
    {
      "id": "CREA.DATA.type",
      "values": [
        {
          "value": 0,
          "id": "creature"
        },
        {
          "value": 1,
          "id": "daedra"
        },
        {
          "value": 2,
          "id": "undead"
        },
        {
          "value": 3,
          "id": "humanoid"
        },
        {
          "value": 4,
          "id": "horse"
        },
        {
          "value": 5,
          "id": "giant"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "Creature"
        },
        {
          "value": 1,
          "label": "Daedra"
        },
        {
          "value": 2,
          "label": "Undead"
        },
        {
          "value": 3,
          "label": "Humanoid"
        },
        {
          "value": 4,
          "label": "Horse"
        },
        {
          "value": 5,
          "label": "Giant"
        }
      ],
      "byte_width": 1,
      "default_value": 0
    },
    {
      "id": "CSTY.CSTD.flags",
      "values": [
        {
          "value": 1,
          "id": "advanced"
        },
        {
          "value": 2,
          "id": "choose_attack_using_chance"
        },
        {
          "value": 4,
          "id": "ignore_allies_in_area"
        },
        {
          "value": 8,
          "id": "will_yield"
        },
        {
          "value": 16,
          "id": "rejects_yields"
        },
        {
          "value": 32,
          "id": "fleeing_disabled"
        },
        {
          "value": 64,
          "id": "prefers_ranged"
        },
        {
          "value": 128,
          "id": "melee_alert_ok"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Advanced"
        },
        {
          "value": 2,
          "label": "Choose Attack using % Chance"
        },
        {
          "value": 4,
          "label": "Ignore Allies in Area"
        },
        {
          "value": 8,
          "label": "Will Yield"
        },
        {
          "value": 16,
          "label": "Rejects Yields"
        },
        {
          "value": 32,
          "label": "Fleeing Disabled"
        },
        {
          "value": 64,
          "label": "Prefers Ranged"
        },
        {
          "value": 128,
          "label": "Melee Alert OK"
        }
      ],
      "storage_kind": "flags",
      "byte_width": 1
    },
    {
      "id": "DOOR.FNAM.flags",
      "values": [
        {
          "value": 1,
          "id": "oblivion_gate"
        },
        {
          "value": 2,
          "id": "automatic_door"
        },
        {
          "value": 4,
          "id": "hidden"
        },
        {
          "value": 8,
          "id": "minimal_use"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Oblivion Gate"
        },
        {
          "value": 2,
          "label": "Automatic Door"
        },
        {
          "value": 4,
          "label": "Hidden"
        },
        {
          "value": 8,
          "label": "Minimal Use"
        }
      ],
      "storage_kind": "flags",
      "byte_width": 1
    },
    {
      "id": "EFSH.DATA.flags",
      "values": [
        {
          "value": 1,
          "id": "no_membrane_shader"
        },
        {
          "value": 8,
          "id": "no_particle_shader"
        },
        {
          "value": 16,
          "id": "edge_effect_inverse"
        },
        {
          "value": 32,
          "id": "membrane_shader_affect_skin_only"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "No Membrane Shader"
        },
        {
          "value": 8,
          "label": "No Particle Shader"
        },
        {
          "value": 16,
          "label": "Edge Effect - Inverse"
        },
        {
          "value": 32,
          "label": "Membrane Shader - Affect Skin Only"
        }
      ],
      "storage_kind": "flags",
      "byte_width": 1
    },
    {
      "id": "ENCH.ENIT.type",
      "values": [
        {
          "value": 0,
          "id": "scroll"
        },
        {
          "value": 1,
          "id": "staff"
        },
        {
          "value": 2,
          "id": "weapon"
        },
        {
          "value": 3,
          "id": "apparel"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "Scroll"
        },
        {
          "value": 1,
          "label": "Staff"
        },
        {
          "value": 2,
          "label": "Weapon"
        },
        {
          "value": 3,
          "label": "Apparel"
        }
      ],
      "default_value": 0
    },
    {
      "id": "FACT.DATA.flags",
      "values": [
        {
          "value": 1,
          "id": "hidden_from_player"
        },
        {
          "value": 2,
          "id": "evil"
        },
        {
          "value": 4,
          "id": "special_combat"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Hidden from Player"
        },
        {
          "value": 2,
          "label": "Evil"
        },
        {
          "value": 4,
          "label": "Special Combat"
        }
      ],
      "storage_kind": "flags",
      "byte_width": 1
    },
    {
      "id": "GRAS.DATA.flags",
      "values": [
        {
          "value": 1,
          "id": "vertex_lighting"
        },
        {
          "value": 2,
          "id": "uniform_scaling"
        },
        {
          "value": 4,
          "id": "fit_to_slope"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Vertex Lighting"
        },
        {
          "value": 2,
          "label": "Uniform Scaling"
        },
        {
          "value": 4,
          "label": "Fit to Slope"
        }
      ],
      "storage_kind": "flags",
      "byte_width": 1
    },
    {
      "id": "GRAS.DATA.unit_from_water_type",
      "values": [
        {
          "value": 0,
          "id": "above_at_least"
        },
        {
          "value": 1,
          "id": "above_at_most"
        },
        {
          "value": 2,
          "id": "below_at_least"
        },
        {
          "value": 3,
          "id": "below_at_most"
        },
        {
          "value": 4,
          "id": "either_at_least"
        },
        {
          "value": 5,
          "id": "either_at_most"
        },
        {
          "value": 6,
          "id": "either_at_most_above"
        },
        {
          "value": 7,
          "id": "either_at_most_below"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "Above - At Least"
        },
        {
          "value": 1,
          "label": "Above - At Most"
        },
        {
          "value": 2,
          "label": "Below - At Least"
        },
        {
          "value": 3,
          "label": "Below - At Most"
        },
        {
          "value": 4,
          "label": "Either - At Least"
        },
        {
          "value": 5,
          "label": "Either - At Most"
        },
        {
          "value": 6,
          "label": "Either - At Most Above"
        },
        {
          "value": 7,
          "label": "Either - At Most Below"
        }
      ],
      "default_value": 0
    },
    {
      "id": "HAIR.DATA.flags",
      "values": [
        {
          "value": 1,
          "id": "playable"
        },
        {
          "value": 2,
          "id": "not_male"
        },
        {
          "value": 4,
          "id": "not_female"
        },
        {
          "value": 8,
          "id": "fixed"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Playable"
        },
        {
          "value": 2,
          "label": "Not Male"
        },
        {
          "value": 4,
          "label": "Not Female"
        },
        {
          "value": 8,
          "label": "Fixed"
        }
      ],
      "storage_kind": "flags",
      "byte_width": 1
    },
    {
      "id": "INFO.DATA.flags",
      "values": [
        {
          "value": 1,
          "id": "goodbye"
        },
        {
          "value": 2,
          "id": "random"
        },
        {
          "value": 4,
          "id": "say_once"
        },
        {
          "value": 8,
          "id": "run_immediately"
        },
        {
          "value": 16,
          "id": "info_refusal"
        },
        {
          "value": 32,
          "id": "random_end"
        },
        {
          "value": 64,
          "id": "run_for_rumors"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Goodbye"
        },
        {
          "value": 2,
          "label": "Random"
        },
        {
          "value": 4,
          "label": "Say Once"
        },
        {
          "value": 8,
          "label": "Run Immediately"
        },
        {
          "value": 16,
          "label": "Info Refusal"
        },
        {
          "value": 32,
          "label": "Random End"
        },
        {
          "value": 64,
          "label": "Run for Rumors"
        }
      ],
      "storage_kind": "flags",
      "byte_width": 1
    },
    {
      "id": "INFO.DATA.next_speaker",
      "values": [
        {
          "value": 0,
          "id": "target"
        },
        {
          "value": 1,
          "id": "self"
        },
        {
          "value": 2,
          "id": "either"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "Target"
        },
        {
          "value": 1,
          "label": "Self"
        },
        {
          "value": 2,
          "label": "Either"
        }
      ],
      "byte_width": 1,
      "default_value": 0
    },
    {
      "id": "INFO.SCHD.type",
      "values": [
        {
          "value": 256,
          "id": "magic_effect"
        }
      ],
      "labels": [
        {
          "value": 256,
          "label": "Magic Effect"
        }
      ]
    },
    {
      "id": "INFO.SCHR.type",
      "values": [
        {
          "value": 256,
          "id": "magic_effect"
        }
      ],
      "labels": [
        {
          "value": 256,
          "label": "Magic Effect"
        }
      ]
    },
    {
      "id": "INFO.TRDT.emotion_type",
      "values": [
        {
          "value": 0,
          "id": "neutral"
        },
        {
          "value": 1,
          "id": "anger"
        },
        {
          "value": 2,
          "id": "disgust"
        },
        {
          "value": 3,
          "id": "fear"
        },
        {
          "value": 4,
          "id": "sad"
        },
        {
          "value": 5,
          "id": "happy"
        },
        {
          "value": 6,
          "id": "surprise"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "Neutral"
        },
        {
          "value": 1,
          "label": "Anger"
        },
        {
          "value": 2,
          "label": "Disgust"
        },
        {
          "value": 3,
          "label": "Fear"
        },
        {
          "value": 4,
          "label": "Sad"
        },
        {
          "value": 5,
          "label": "Happy"
        },
        {
          "value": 6,
          "label": "Surprise"
        }
      ],
      "default_value": 0
    },
    {
      "id": "INGR.ENIT.flags",
      "values": [
        {
          "value": 1,
          "id": "no_auto_calculate"
        },
        {
          "value": 2,
          "id": "food_item"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "No Auto-Calculate"
        },
        {
          "value": 2,
          "label": "Food Item"
        }
      ],
      "storage_kind": "flags",
      "byte_width": 1
    },
    {
      "id": "LAND.DATA.flags",
      "values": [
        {
          "value": 1,
          "id": "has_vertex_normals_height_map"
        },
        {
          "value": 2,
          "id": "has_vertex_colours"
        },
        {
          "value": 4,
          "id": "has_layers"
        },
        {
          "value": 8,
          "id": "unknown_4"
        },
        {
          "value": 16,
          "id": "auto_calc_normals"
        },
        {
          "value": 1024,
          "id": "ignored"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Has Vertex Normals/Height Map"
        },
        {
          "value": 2,
          "label": "Has Vertex Colours"
        },
        {
          "value": 4,
          "label": "Has Layers"
        },
        {
          "value": 8,
          "label": "Unknown 4"
        },
        {
          "value": 16,
          "label": "Auto-Calc Normals"
        },
        {
          "value": 1024,
          "label": "Ignored"
        }
      ],
      "storage_kind": "flags"
    },
    {
      "id": "LIGH.DATA.flags",
      "values": [
        {
          "value": 1,
          "id": "dynamic"
        },
        {
          "value": 2,
          "id": "can_be_carried"
        },
        {
          "value": 4,
          "id": "negative"
        },
        {
          "value": 8,
          "id": "flicker"
        },
        {
          "value": 16,
          "id": "unused"
        },
        {
          "value": 32,
          "id": "off_by_default"
        },
        {
          "value": 64,
          "id": "flicker_slow"
        },
        {
          "value": 128,
          "id": "pulse"
        },
        {
          "value": 256,
          "id": "pulse_slow"
        },
        {
          "value": 512,
          "id": "spot_light"
        },
        {
          "value": 1024,
          "id": "spot_shadow"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Dynamic"
        },
        {
          "value": 2,
          "label": "Can be Carried"
        },
        {
          "value": 4,
          "label": "Negative"
        },
        {
          "value": 8,
          "label": "Flicker"
        },
        {
          "value": 16,
          "label": "Unused"
        },
        {
          "value": 32,
          "label": "Off By Default"
        },
        {
          "value": 64,
          "label": "Flicker Slow"
        },
        {
          "value": 128,
          "label": "Pulse"
        },
        {
          "value": 256,
          "label": "Pulse Slow"
        },
        {
          "value": 512,
          "label": "Spot Light"
        },
        {
          "value": 1024,
          "label": "Spot Shadow"
        }
      ],
      "storage_kind": "flags"
    },
    {
      "id": "LTEX.HNAM.material_type",
      "values": [
        {
          "value": 0,
          "id": "stone"
        },
        {
          "value": 1,
          "id": "cloth"
        },
        {
          "value": 2,
          "id": "dirt"
        },
        {
          "value": 3,
          "id": "glass"
        },
        {
          "value": 4,
          "id": "grass"
        },
        {
          "value": 5,
          "id": "metal"
        },
        {
          "value": 6,
          "id": "organic"
        },
        {
          "value": 7,
          "id": "skin"
        },
        {
          "value": 8,
          "id": "water"
        },
        {
          "value": 9,
          "id": "wood"
        },
        {
          "value": 10,
          "id": "heavy_stone"
        },
        {
          "value": 11,
          "id": "heavy_metal"
        },
        {
          "value": 12,
          "id": "heavy_wood"
        },
        {
          "value": 13,
          "id": "chain"
        },
        {
          "value": 14,
          "id": "snow"
        },
        {
          "value": 15,
          "id": "stone_stairs"
        },
        {
          "value": 16,
          "id": "cloth_stairs"
        },
        {
          "value": 17,
          "id": "dirt_stairs"
        },
        {
          "value": 18,
          "id": "glass_stairs"
        },
        {
          "value": 19,
          "id": "grass_stairs"
        },
        {
          "value": 20,
          "id": "metal_stairs"
        },
        {
          "value": 21,
          "id": "organic_stairs"
        },
        {
          "value": 22,
          "id": "skin_stairs"
        },
        {
          "value": 23,
          "id": "water_stairs"
        },
        {
          "value": 24,
          "id": "wood_stairs"
        },
        {
          "value": 25,
          "id": "heavy_stone_stairs"
        },
        {
          "value": 26,
          "id": "heavy_metal_stairs"
        },
        {
          "value": 27,
          "id": "heavy_wood_stairs"
        },
        {
          "value": 28,
          "id": "chain_stairs"
        },
        {
          "value": 29,
          "id": "snow_stairs"
        },
        {
          "value": 30,
          "id": "elevator"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "Stone"
        },
        {
          "value": 1,
          "label": "Cloth"
        },
        {
          "value": 2,
          "label": "Dirt"
        },
        {
          "value": 3,
          "label": "Glass"
        },
        {
          "value": 4,
          "label": "Grass"
        },
        {
          "value": 5,
          "label": "Metal"
        },
        {
          "value": 6,
          "label": "Organic"
        },
        {
          "value": 7,
          "label": "Skin"
        },
        {
          "value": 8,
          "label": "Water"
        },
        {
          "value": 9,
          "label": "Wood"
        },
        {
          "value": 10,
          "label": "Heavy Stone"
        },
        {
          "value": 11,
          "label": "Heavy Metal"
        },
        {
          "value": 12,
          "label": "Heavy Wood"
        },
        {
          "value": 13,
          "label": "Chain"
        },
        {
          "value": 14,
          "label": "Snow"
        },
        {
          "value": 15,
          "label": "Stone Stairs"
        },
        {
          "value": 16,
          "label": "Cloth Stairs"
        },
        {
          "value": 17,
          "label": "Dirt Stairs"
        },
        {
          "value": 18,
          "label": "Glass Stairs"
        },
        {
          "value": 19,
          "label": "Grass Stairs"
        },
        {
          "value": 20,
          "label": "Metal Stairs"
        },
        {
          "value": 21,
          "label": "Organic Stairs"
        },
        {
          "value": 22,
          "label": "Skin Stairs"
        },
        {
          "value": 23,
          "label": "Water Stairs"
        },
        {
          "value": 24,
          "label": "Wood Stairs"
        },
        {
          "value": 25,
          "label": "Heavy Stone Stairs"
        },
        {
          "value": 26,
          "label": "Heavy Metal Stairs"
        },
        {
          "value": 27,
          "label": "Heavy Wood Stairs"
        },
        {
          "value": 28,
          "label": "Chain Stairs"
        },
        {
          "value": 29,
          "label": "Snow Stairs"
        },
        {
          "value": 30,
          "label": "Elevator"
        }
      ],
      "byte_width": 1,
      "default_value": 0
    },
    {
      "id": "LVLC.LVLF.flags",
      "values": [
        {
          "value": 1,
          "id": "calculate_from_all_levels_player_s_level"
        },
        {
          "value": 2,
          "id": "calculate_for_each_item_in_count"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Calculate from all levels <= player's level"
        },
        {
          "value": 2,
          "label": "Calculate for each item in count"
        }
      ],
      "storage_kind": "flags",
      "byte_width": 1
    },
    {
      "id": "LVLI.LVLF.flags",
      "values": [
        {
          "value": 1,
          "id": "calculate_from_all_levels_player_s_level"
        },
        {
          "value": 2,
          "id": "calculate_for_each_item_in_count"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Calculate from all levels <= player's level"
        },
        {
          "value": 2,
          "label": "Calculate for each item in count"
        }
      ],
      "storage_kind": "flags",
      "byte_width": 1
    },
    {
      "id": "LVSP.LVLF.flags",
      "values": [
        {
          "value": 1,
          "id": "calculate_from_all_levels_player_s_level"
        },
        {
          "value": 2,
          "id": "calculate_for_each_item_in_count"
        },
        {
          "value": 4,
          "id": "use_all_spells"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Calculate from all levels <= player's level"
        },
        {
          "value": 2,
          "label": "Calculate for each item in count"
        },
        {
          "value": 4,
          "label": "Use all spells"
        }
      ],
      "storage_kind": "flags",
      "byte_width": 1
    },
    {
      "id": "MGEF.DATA.flags",
      "values": [
        {
          "value": 1,
          "id": "hostile"
        },
        {
          "value": 2,
          "id": "recover"
        },
        {
          "value": 4,
          "id": "detrimental"
        },
        {
          "value": 8,
          "id": "magnitude"
        },
        {
          "value": 16,
          "id": "self"
        },
        {
          "value": 32,
          "id": "touch"
        },
        {
          "value": 64,
          "id": "target"
        },
        {
          "value": 128,
          "id": "no_duration"
        },
        {
          "value": 256,
          "id": "no_magnitude"
        },
        {
          "value": 512,
          "id": "no_area"
        },
        {
          "value": 1024,
          "id": "fx_persist"
        },
        {
          "value": 2048,
          "id": "spellmaking"
        },
        {
          "value": 4096,
          "id": "enchanting"
        },
        {
          "value": 8192,
          "id": "no_ingredient"
        },
        {
          "value": 65536,
          "id": "use_weapon"
        },
        {
          "value": 131072,
          "id": "use_armor"
        },
        {
          "value": 262144,
          "id": "use_creature"
        },
        {
          "value": 524288,
          "id": "use_skill"
        },
        {
          "value": 1048576,
          "id": "use_attribute"
        },
        {
          "value": 16777216,
          "id": "use_actor_value"
        },
        {
          "value": 33554432,
          "id": "spray_projectile_type_or_fog_if_bolt_is_specified_as_well"
        },
        {
          "value": 67108864,
          "id": "bolt_projectile_type"
        },
        {
          "value": 134217728,
          "id": "no_hit_effect"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Hostile"
        },
        {
          "value": 2,
          "label": "Recover"
        },
        {
          "value": 4,
          "label": "Detrimental"
        },
        {
          "value": 8,
          "label": "Magnitude %"
        },
        {
          "value": 16,
          "label": "Self"
        },
        {
          "value": 32,
          "label": "Touch"
        },
        {
          "value": 64,
          "label": "Target"
        },
        {
          "value": 128,
          "label": "No duration"
        },
        {
          "value": 256,
          "label": "No magnitude"
        },
        {
          "value": 512,
          "label": "No area"
        },
        {
          "value": 1024,
          "label": "FX persist"
        },
        {
          "value": 2048,
          "label": "Spellmaking"
        },
        {
          "value": 4096,
          "label": "Enchanting"
        },
        {
          "value": 8192,
          "label": "No Ingredient"
        },
        {
          "value": 65536,
          "label": "Use weapon"
        },
        {
          "value": 131072,
          "label": "Use armor"
        },
        {
          "value": 262144,
          "label": "Use creature"
        },
        {
          "value": 524288,
          "label": "Use skill"
        },
        {
          "value": 1048576,
          "label": "Use attribute"
        },
        {
          "value": 16777216,
          "label": "Use actor value"
        },
        {
          "value": 33554432,
          "label": "Spray projectile type (or Fog if Bolt is specified as well)"
        },
        {
          "value": 67108864,
          "label": "Bolt projectile type"
        },
        {
          "value": 134217728,
          "label": "No hit effect"
        }
      ],
      "storage_kind": "flags"
    },
    {
      "id": "MGEF.DATA.resist_value",
      "values": [
        {
          "value": 61,
          "id": "resist_fire"
        },
        {
          "value": 62,
          "id": "resist_frost"
        },
        {
          "value": 63,
          "id": "resist_disease"
        },
        {
          "value": 64,
          "id": "resist_magic"
        },
        {
          "value": 65,
          "id": "resist_normal_weapons"
        },
        {
          "value": 66,
          "id": "resist_paralysis"
        },
        {
          "value": 67,
          "id": "resist_poison"
        },
        {
          "value": 68,
          "id": "resist_shock"
        }
      ],
      "labels": [
        {
          "value": 61,
          "label": "Resist Fire"
        },
        {
          "value": 62,
          "label": "Resist Frost"
        },
        {
          "value": 63,
          "label": "Resist Disease"
        },
        {
          "value": 64,
          "label": "Resist Magic"
        },
        {
          "value": 65,
          "label": "Resist Normal Weapons"
        },
        {
          "value": 66,
          "label": "Resist Paralysis"
        },
        {
          "value": 67,
          "label": "Resist Poison"
        },
        {
          "value": 68,
          "label": "Resist Shock"
        }
      ]
    },
    {
      "id": "MISC.DATA.group_group",
      "values": [
        {
          "value": 0,
          "id": "attribute"
        },
        {
          "value": 1065353216,
          "id": "stat"
        },
        {
          "value": 1073741824,
          "id": "skill"
        },
        {
          "value": 1077936128,
          "id": "ai"
        },
        {
          "value": 1082130432,
          "id": "social"
        },
        {
          "value": 1084227584,
          "id": "misc"
        },
        {
          "value": 1086324736,
          "id": "combat"
        },
        {
          "value": 1088421888,
          "id": "none"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "Attribute"
        },
        {
          "value": 1065353216,
          "label": "Stat"
        },
        {
          "value": 1073741824,
          "label": "Skill"
        },
        {
          "value": 1077936128,
          "label": "AI"
        },
        {
          "value": 1082130432,
          "label": "Social"
        },
        {
          "value": 1084227584,
          "label": "Misc"
        },
        {
          "value": 1086324736,
          "label": "Combat"
        },
        {
          "value": 1088421888,
          "label": " [NONE]"
        }
      ],
      "default_value": 0
    },
    {
      "id": "NPC_.ACBS.flags",
      "values": [
        {
          "value": 1,
          "id": "female"
        },
        {
          "value": 2,
          "id": "essential"
        },
        {
          "value": 8,
          "id": "respawn"
        },
        {
          "value": 16,
          "id": "auto_calc_stats"
        },
        {
          "value": 128,
          "id": "pc_level_offset"
        },
        {
          "value": 512,
          "id": "no_low_level_processing"
        },
        {
          "value": 8192,
          "id": "no_rumors"
        },
        {
          "value": 16384,
          "id": "summonable"
        },
        {
          "value": 32768,
          "id": "no_persuasion"
        },
        {
          "value": 1048576,
          "id": "can_corpse_check"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Female"
        },
        {
          "value": 2,
          "label": "Essential"
        },
        {
          "value": 8,
          "label": "Respawn"
        },
        {
          "value": 16,
          "label": "Auto-calc stats"
        },
        {
          "value": 128,
          "label": "PC Level Offset"
        },
        {
          "value": 512,
          "label": "No Low Level Processing"
        },
        {
          "value": 8192,
          "label": "No Rumors"
        },
        {
          "value": 16384,
          "label": "Summonable"
        },
        {
          "value": 32768,
          "label": "No Persuasion"
        },
        {
          "value": 1048576,
          "label": "Can Corpse Check"
        }
      ],
      "storage_kind": "flags"
    },
    {
      "id": "PACK.PLDT.type",
      "values": [
        {
          "value": 0,
          "id": "near_reference"
        },
        {
          "value": 1,
          "id": "in_cell"
        },
        {
          "value": 2,
          "id": "near_current_location"
        },
        {
          "value": 3,
          "id": "near_editor_location"
        },
        {
          "value": 4,
          "id": "object_id"
        },
        {
          "value": 5,
          "id": "object_type"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "Near Reference"
        },
        {
          "value": 1,
          "label": "In Cell"
        },
        {
          "value": 2,
          "label": "Near Current Location"
        },
        {
          "value": 3,
          "label": "Near Editor Location"
        },
        {
          "value": 4,
          "label": "Object ID"
        },
        {
          "value": 5,
          "label": "Object Type"
        }
      ],
      "default_value": 0
    },
    {
      "id": "PACK.PTDT.object_type_object_type",
      "values": [
        {
          "value": 0,
          "id": "none"
        },
        {
          "value": 1,
          "id": "activators"
        },
        {
          "value": 2,
          "id": "apparatus"
        },
        {
          "value": 3,
          "id": "armor"
        },
        {
          "value": 4,
          "id": "books"
        },
        {
          "value": 5,
          "id": "clothing"
        },
        {
          "value": 6,
          "id": "containers"
        },
        {
          "value": 7,
          "id": "doors"
        },
        {
          "value": 8,
          "id": "ingredients"
        },
        {
          "value": 9,
          "id": "lights"
        },
        {
          "value": 10,
          "id": "miscellaneous"
        },
        {
          "value": 11,
          "id": "flora"
        },
        {
          "value": 12,
          "id": "furniture"
        },
        {
          "value": 13,
          "id": "weapons_all"
        },
        {
          "value": 14,
          "id": "ammo"
        },
        {
          "value": 15,
          "id": "npcs"
        },
        {
          "value": 16,
          "id": "creatures"
        },
        {
          "value": 17,
          "id": "soul_gems"
        },
        {
          "value": 18,
          "id": "keys"
        },
        {
          "value": 19,
          "id": "alchemy"
        },
        {
          "value": 20,
          "id": "food"
        },
        {
          "value": 21,
          "id": "all_combat_wearable"
        },
        {
          "value": 22,
          "id": "all_wearable"
        },
        {
          "value": 23,
          "id": "weapons_none"
        },
        {
          "value": 24,
          "id": "weapons_melee"
        },
        {
          "value": 25,
          "id": "weapons_ranged"
        },
        {
          "value": 26,
          "id": "spells_any"
        },
        {
          "value": 27,
          "id": "spells_range_target"
        },
        {
          "value": 28,
          "id": "spells_range_touch"
        },
        {
          "value": 29,
          "id": "spells_range_self"
        },
        {
          "value": 30,
          "id": "spells_school_alteration"
        },
        {
          "value": 31,
          "id": "spells_school_conjuration"
        },
        {
          "value": 32,
          "id": "spells_school_destruction"
        },
        {
          "value": 33,
          "id": "spells_school_illusion"
        },
        {
          "value": 34,
          "id": "spells_school_mysticism"
        },
        {
          "value": 35,
          "id": "spells_school_restoration"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "None"
        },
        {
          "value": 1,
          "label": "Activators"
        },
        {
          "value": 2,
          "label": "Apparatus"
        },
        {
          "value": 3,
          "label": "Armor"
        },
        {
          "value": 4,
          "label": "Books"
        },
        {
          "value": 5,
          "label": "Clothing"
        },
        {
          "value": 6,
          "label": "Containers"
        },
        {
          "value": 7,
          "label": "Doors"
        },
        {
          "value": 8,
          "label": "Ingredients"
        },
        {
          "value": 9,
          "label": "Lights"
        },
        {
          "value": 10,
          "label": "Miscellaneous"
        },
        {
          "value": 11,
          "label": "Flora"
        },
        {
          "value": 12,
          "label": "Furniture"
        },
        {
          "value": 13,
          "label": "Weapons: All"
        },
        {
          "value": 14,
          "label": "Ammo"
        },
        {
          "value": 15,
          "label": "NPCs"
        },
        {
          "value": 16,
          "label": "Creatures"
        },
        {
          "value": 17,
          "label": "Soul Gems"
        },
        {
          "value": 18,
          "label": "Keys"
        },
        {
          "value": 19,
          "label": "Alchemy"
        },
        {
          "value": 20,
          "label": "Food"
        },
        {
          "value": 21,
          "label": "All: Combat Wearable"
        },
        {
          "value": 22,
          "label": "All: Wearable"
        },
        {
          "value": 23,
          "label": "Weapons: None"
        },
        {
          "value": 24,
          "label": "Weapons: Melee"
        },
        {
          "value": 25,
          "label": "Weapons: Ranged"
        },
        {
          "value": 26,
          "label": "Spells: Any"
        },
        {
          "value": 27,
          "label": "Spells: Range Target"
        },
        {
          "value": 28,
          "label": "Spells: Range Touch"
        },
        {
          "value": 29,
          "label": "Spells: Range Self"
        },
        {
          "value": 30,
          "label": "Spells: School Alteration"
        },
        {
          "value": 31,
          "label": "Spells: School Conjuration"
        },
        {
          "value": 32,
          "label": "Spells: School Destruction"
        },
        {
          "value": 33,
          "label": "Spells: School Illusion"
        },
        {
          "value": 34,
          "label": "Spells: School Mysticism"
        },
        {
          "value": 35,
          "label": "Spells: School Restoration"
        }
      ],
      "default_value": 0
    },
    {
      "id": "PACK.PTDT.type",
      "values": [
        {
          "value": 0,
          "id": "specific_reference"
        },
        {
          "value": 1,
          "id": "object_id"
        },
        {
          "value": 2,
          "id": "object_type"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "Specific Reference"
        },
        {
          "value": 1,
          "label": "Object ID"
        },
        {
          "value": 2,
          "label": "Object Type"
        }
      ],
      "default_value": 0
    },
    {
      "id": "QUST.DATA.flags",
      "values": [
        {
          "value": 1,
          "id": "start_game_enabled"
        },
        {
          "value": 4,
          "id": "allow_repeated_conversation_topics"
        },
        {
          "value": 8,
          "id": "allow_repeated_stages"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Start game enabled"
        },
        {
          "value": 4,
          "label": "Allow repeated conversation topics"
        },
        {
          "value": 8,
          "label": "Allow repeated stages"
        }
      ],
      "storage_kind": "flags",
      "byte_width": 1
    },
    {
      "id": "QUST.SCHD.type",
      "values": [
        {
          "value": 256,
          "id": "magic_effect"
        }
      ],
      "labels": [
        {
          "value": 256,
          "label": "Magic Effect"
        }
      ]
    },
    {
      "id": "QUST.SCHR.type",
      "values": [
        {
          "value": 256,
          "id": "magic_effect"
        }
      ],
      "labels": [
        {
          "value": 256,
          "label": "Magic Effect"
        }
      ]
    },
    {
      "id": "REFR.FNAM.map_flags",
      "values": [
        {
          "value": 1,
          "id": "visible"
        },
        {
          "value": 2,
          "id": "can_travel_to"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Visible"
        },
        {
          "value": 2,
          "label": "Can Travel To"
        }
      ],
      "storage_kind": "flags",
      "byte_width": 1
    },
    {
      "id": "REFR.TNAM.type",
      "values": [
        {
          "value": 0,
          "id": "none"
        },
        {
          "value": 1,
          "id": "camp"
        },
        {
          "value": 2,
          "id": "cave"
        },
        {
          "value": 3,
          "id": "city"
        },
        {
          "value": 4,
          "id": "elven_ruin"
        },
        {
          "value": 5,
          "id": "fort_ruin"
        },
        {
          "value": 6,
          "id": "mine"
        },
        {
          "value": 7,
          "id": "landmark"
        },
        {
          "value": 8,
          "id": "tavern"
        },
        {
          "value": 9,
          "id": "settlement"
        },
        {
          "value": 10,
          "id": "daedric_shrine"
        },
        {
          "value": 11,
          "id": "oblivion_gate"
        },
        {
          "value": 12,
          "id": "unknown_door_icon"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "None"
        },
        {
          "value": 1,
          "label": "Camp"
        },
        {
          "value": 2,
          "label": "Cave"
        },
        {
          "value": 3,
          "label": "City"
        },
        {
          "value": 4,
          "label": "Elven Ruin"
        },
        {
          "value": 5,
          "label": "Fort Ruin"
        },
        {
          "value": 6,
          "label": "Mine"
        },
        {
          "value": 7,
          "label": "Landmark"
        },
        {
          "value": 8,
          "label": "Tavern"
        },
        {
          "value": 9,
          "label": "Settlement"
        },
        {
          "value": 10,
          "label": "Daedric Shrine"
        },
        {
          "value": 11,
          "label": "Oblivion Gate"
        },
        {
          "value": 12,
          "label": "Unknown? (door icon)"
        }
      ],
      "byte_width": 1,
      "default_value": 0
    },
    {
      "id": "REFR.XACT.action_flag",
      "values": [
        {
          "value": 1,
          "id": "use_default"
        },
        {
          "value": 2,
          "id": "activate"
        },
        {
          "value": 4,
          "id": "open"
        },
        {
          "value": 8,
          "id": "open_by_default"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Use Default"
        },
        {
          "value": 2,
          "label": "Activate"
        },
        {
          "value": 4,
          "label": "Open"
        },
        {
          "value": 8,
          "label": "Open by Default"
        }
      ],
      "storage_kind": "flags"
    },
    {
      "id": "REGN.RDAT.type",
      "values": [
        {
          "value": 2,
          "id": "objects"
        },
        {
          "value": 3,
          "id": "weather"
        },
        {
          "value": 4,
          "id": "map"
        },
        {
          "value": 5,
          "id": "land"
        },
        {
          "value": 6,
          "id": "grass"
        },
        {
          "value": 7,
          "id": "sound"
        }
      ],
      "labels": [
        {
          "value": 2,
          "label": "Objects"
        },
        {
          "value": 3,
          "label": "Weather"
        },
        {
          "value": 4,
          "label": "Map"
        },
        {
          "value": 5,
          "label": "Land"
        },
        {
          "value": 6,
          "label": "Grass"
        },
        {
          "value": 7,
          "label": "Sound"
        }
      ]
    },
    {
      "id": "REGN.RDOT.objects_flags",
      "values": [
        {
          "value": 1,
          "id": "conform_to_slope"
        },
        {
          "value": 2,
          "id": "paint_vertices"
        },
        {
          "value": 4,
          "id": "size_variance"
        },
        {
          "value": 8,
          "id": "x"
        },
        {
          "value": 16,
          "id": "y"
        },
        {
          "value": 32,
          "id": "z"
        },
        {
          "value": 64,
          "id": "tree"
        },
        {
          "value": 128,
          "id": "huge_rock"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Conform to slope"
        },
        {
          "value": 2,
          "label": "Paint Vertices"
        },
        {
          "value": 4,
          "label": "Size Variance +/-"
        },
        {
          "value": 8,
          "label": "X +/-"
        },
        {
          "value": 16,
          "label": "Y +/-"
        },
        {
          "value": 32,
          "label": "Z +/-"
        },
        {
          "value": 64,
          "label": "Tree"
        },
        {
          "value": 128,
          "label": "Huge Rock"
        }
      ],
      "storage_kind": "flags",
      "byte_width": 1
    },
    {
      "id": "SCPT.SCHD.type",
      "values": [
        {
          "value": 256,
          "id": "magic_effect"
        }
      ],
      "labels": [
        {
          "value": 256,
          "label": "Magic Effect"
        }
      ]
    },
    {
      "id": "SCPT.SCHR.type",
      "values": [
        {
          "value": 256,
          "id": "magic_effect"
        }
      ],
      "labels": [
        {
          "value": 256,
          "label": "Magic Effect"
        }
      ]
    },
    {
      "id": "SPEL.SPIT.flags",
      "values": [
        {
          "value": 1,
          "id": "manual_spell_cost"
        },
        {
          "value": 2,
          "id": "immune_to_silence_1"
        },
        {
          "value": 4,
          "id": "player_start_spell"
        },
        {
          "value": 8,
          "id": "immune_to_silence_2"
        },
        {
          "value": 16,
          "id": "area_effect_ignores_los"
        },
        {
          "value": 32,
          "id": "script_effect_always_applies"
        },
        {
          "value": 64,
          "id": "disallow_spell_absorb_reflect"
        },
        {
          "value": 128,
          "id": "touch_spell_explodes_w_no_target"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Manual Spell Cost"
        },
        {
          "value": 2,
          "label": "Immune to Silence 1"
        },
        {
          "value": 4,
          "label": "Player Start Spell"
        },
        {
          "value": 8,
          "label": "Immune to Silence 2"
        },
        {
          "value": 16,
          "label": "Area Effect Ignores LOS"
        },
        {
          "value": 32,
          "label": "Script Effect Always Applies"
        },
        {
          "value": 64,
          "label": "Disallow Spell Absorb/Reflect"
        },
        {
          "value": 128,
          "label": "Touch Spell Explodes w/ no Target"
        }
      ],
      "storage_kind": "flags",
      "byte_width": 1
    },
    {
      "id": "SPEL.SPIT.level",
      "values": [
        {
          "value": 0,
          "id": "novice"
        },
        {
          "value": 1,
          "id": "apprentice"
        },
        {
          "value": 2,
          "id": "journeyman"
        },
        {
          "value": 3,
          "id": "expert"
        },
        {
          "value": 4,
          "id": "master"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "Novice"
        },
        {
          "value": 1,
          "label": "Apprentice"
        },
        {
          "value": 2,
          "label": "Journeyman"
        },
        {
          "value": 3,
          "label": "Expert"
        },
        {
          "value": 4,
          "label": "Master"
        }
      ],
      "byte_width": 1,
      "default_value": 0
    },
    {
      "id": "SPEL.SPIT.type",
      "values": [
        {
          "value": 0,
          "id": "spell"
        },
        {
          "value": 1,
          "id": "disease"
        },
        {
          "value": 2,
          "id": "power"
        },
        {
          "value": 3,
          "id": "lesser_power"
        },
        {
          "value": 4,
          "id": "ability"
        },
        {
          "value": 5,
          "id": "poison"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "Spell"
        },
        {
          "value": 1,
          "label": "Disease"
        },
        {
          "value": 2,
          "label": "Power"
        },
        {
          "value": 3,
          "label": "Lesser Power"
        },
        {
          "value": 4,
          "label": "Ability"
        },
        {
          "value": 5,
          "label": "Poison"
        }
      ],
      "byte_width": 1,
      "default_value": 0
    },
    {
      "id": "WATR.FNAM.flags",
      "values": [
        {
          "value": 1,
          "id": "causes_damage"
        },
        {
          "value": 2,
          "id": "reflective"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Causes Damage"
        },
        {
          "value": 2,
          "label": "Reflective"
        }
      ],
      "storage_kind": "flags",
      "byte_width": 1
    },
    {
      "id": "WEAP.DATA.type",
      "values": [
        {
          "value": 0,
          "id": "blade_one_hand"
        },
        {
          "value": 1,
          "id": "blade_two_hand"
        },
        {
          "value": 2,
          "id": "blunt_one_hand"
        },
        {
          "value": 3,
          "id": "blunt_two_hand"
        },
        {
          "value": 4,
          "id": "staff"
        },
        {
          "value": 5,
          "id": "bow"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "Blade One Hand"
        },
        {
          "value": 1,
          "label": "Blade Two Hand"
        },
        {
          "value": 2,
          "label": "Blunt One Hand"
        },
        {
          "value": 3,
          "label": "Blunt Two Hand"
        },
        {
          "value": 4,
          "label": "Staff"
        },
        {
          "value": 5,
          "label": "Bow"
        }
      ],
      "byte_width": 1,
      "default_value": 0
    },
    {
      "id": "WRLD.DATA.flags",
      "values": [
        {
          "value": 1,
          "id": "small_world"
        },
        {
          "value": 2,
          "id": "can_t_fast_travel"
        },
        {
          "value": 4,
          "id": "oblivion_worldspace"
        },
        {
          "value": 16,
          "id": "no_lod_water"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Small world"
        },
        {
          "value": 2,
          "label": "Can't fast travel"
        },
        {
          "value": 4,
          "label": "Oblivion worldspace"
        },
        {
          "value": 16,
          "label": "No LOD water"
        }
      ],
      "storage_kind": "flags",
      "byte_width": 1
    },
    {
      "id": "WTHR.DATA.flags",
      "values": [
        {
          "value": 1,
          "id": "weather_pleasant"
        },
        {
          "value": 2,
          "id": "weather_cloudy"
        },
        {
          "value": 4,
          "id": "weather_rainy"
        },
        {
          "value": 8,
          "id": "weather_snow"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Weather - Pleasant"
        },
        {
          "value": 2,
          "label": "Weather - Cloudy"
        },
        {
          "value": 4,
          "label": "Weather - Rainy"
        },
        {
          "value": 8,
          "label": "Weather - Snow"
        }
      ],
      "storage_kind": "flags",
      "byte_width": 1
    },
    {
      "id": "WTHR.SNAM.type",
      "values": [
        {
          "value": 0,
          "id": "default"
        },
        {
          "value": 1,
          "id": "precipitation"
        },
        {
          "value": 2,
          "id": "wind"
        },
        {
          "value": 3,
          "id": "thunder"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "Default"
        },
        {
          "value": 1,
          "label": "Precipitation"
        },
        {
          "value": 2,
          "label": "Wind"
        },
        {
          "value": 3,
          "label": "Thunder"
        }
      ],
      "default_value": 0
    },
    {
      "id": "actor_value_enum",
      "values": [
        {
          "value": 0,
          "id": "strength"
        },
        {
          "value": 1,
          "id": "intelligence"
        },
        {
          "value": 2,
          "id": "willpower"
        },
        {
          "value": 3,
          "id": "agility"
        },
        {
          "value": 4,
          "id": "speed"
        },
        {
          "value": 5,
          "id": "endurance"
        },
        {
          "value": 6,
          "id": "personality"
        },
        {
          "value": 7,
          "id": "luck"
        },
        {
          "value": 8,
          "id": "health"
        },
        {
          "value": 9,
          "id": "magicka"
        },
        {
          "value": 10,
          "id": "fatigue"
        },
        {
          "value": 11,
          "id": "encumbrance"
        },
        {
          "value": 12,
          "id": "armorer"
        },
        {
          "value": 13,
          "id": "athletics"
        },
        {
          "value": 14,
          "id": "blade"
        },
        {
          "value": 15,
          "id": "block"
        },
        {
          "value": 16,
          "id": "blunt"
        },
        {
          "value": 17,
          "id": "hand_to_hand"
        },
        {
          "value": 18,
          "id": "heavy_armor"
        },
        {
          "value": 19,
          "id": "alchemy"
        },
        {
          "value": 20,
          "id": "alteration"
        },
        {
          "value": 21,
          "id": "conjuration"
        },
        {
          "value": 22,
          "id": "destruction"
        },
        {
          "value": 23,
          "id": "illusion"
        },
        {
          "value": 24,
          "id": "mysticism"
        },
        {
          "value": 25,
          "id": "restoration"
        },
        {
          "value": 26,
          "id": "acrobatics"
        },
        {
          "value": 27,
          "id": "light_armor"
        },
        {
          "value": 28,
          "id": "marksman"
        },
        {
          "value": 29,
          "id": "mercantile"
        },
        {
          "value": 30,
          "id": "security"
        },
        {
          "value": 31,
          "id": "sneak"
        },
        {
          "value": 32,
          "id": "speechcraft"
        },
        {
          "value": 33,
          "id": "aggression"
        },
        {
          "value": 34,
          "id": "confidence"
        },
        {
          "value": 35,
          "id": "energy"
        },
        {
          "value": 36,
          "id": "responsibility"
        },
        {
          "value": 37,
          "id": "bounty"
        },
        {
          "value": 38,
          "id": "fame"
        },
        {
          "value": 39,
          "id": "infamy"
        },
        {
          "value": 40,
          "id": "magicka_multiplier"
        },
        {
          "value": 41,
          "id": "night_eye_bonus"
        },
        {
          "value": 42,
          "id": "attack_bonus"
        },
        {
          "value": 43,
          "id": "defend_bonus"
        },
        {
          "value": 44,
          "id": "casting_penalty"
        },
        {
          "value": 45,
          "id": "blindness"
        },
        {
          "value": 46,
          "id": "chameleon"
        },
        {
          "value": 47,
          "id": "invisibility"
        },
        {
          "value": 48,
          "id": "paralysis"
        },
        {
          "value": 49,
          "id": "silence"
        },
        {
          "value": 50,
          "id": "confusion"
        },
        {
          "value": 51,
          "id": "detect_item_range"
        },
        {
          "value": 52,
          "id": "spell_absorb_chance"
        },
        {
          "value": 53,
          "id": "spell_reflect_chance"
        },
        {
          "value": 54,
          "id": "swim_speed_multiplier"
        },
        {
          "value": 55,
          "id": "water_breathing"
        },
        {
          "value": 56,
          "id": "water_walking"
        },
        {
          "value": 57,
          "id": "stunted_magicka"
        },
        {
          "value": 58,
          "id": "detect_life_range"
        },
        {
          "value": 59,
          "id": "reflect_damage"
        },
        {
          "value": 60,
          "id": "telekinesis"
        },
        {
          "value": 61,
          "id": "resist_fire"
        },
        {
          "value": 62,
          "id": "resist_frost"
        },
        {
          "value": 63,
          "id": "resist_disease"
        },
        {
          "value": 64,
          "id": "resist_magic"
        },
        {
          "value": 65,
          "id": "resist_normal_weapons"
        },
        {
          "value": 66,
          "id": "resist_paralysis"
        },
        {
          "value": 67,
          "id": "resist_poison"
        },
        {
          "value": 68,
          "id": "resist_shock"
        },
        {
          "value": 69,
          "id": "vampirism"
        },
        {
          "value": 70,
          "id": "darkness"
        },
        {
          "value": 71,
          "id": "resist_water_damage"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "Strength"
        },
        {
          "value": 1,
          "label": "Intelligence"
        },
        {
          "value": 2,
          "label": "Willpower"
        },
        {
          "value": 3,
          "label": "Agility"
        },
        {
          "value": 4,
          "label": "Speed"
        },
        {
          "value": 5,
          "label": "Endurance"
        },
        {
          "value": 6,
          "label": "Personality"
        },
        {
          "value": 7,
          "label": "Luck"
        },
        {
          "value": 8,
          "label": "Health"
        },
        {
          "value": 9,
          "label": "Magicka"
        },
        {
          "value": 10,
          "label": "Fatigue"
        },
        {
          "value": 11,
          "label": "Encumbrance"
        },
        {
          "value": 12,
          "label": "Armorer"
        },
        {
          "value": 13,
          "label": "Athletics"
        },
        {
          "value": 14,
          "label": "Blade"
        },
        {
          "value": 15,
          "label": "Block"
        },
        {
          "value": 16,
          "label": "Blunt"
        },
        {
          "value": 17,
          "label": "Hand To Hand"
        },
        {
          "value": 18,
          "label": "Heavy Armor"
        },
        {
          "value": 19,
          "label": "Alchemy"
        },
        {
          "value": 20,
          "label": "Alteration"
        },
        {
          "value": 21,
          "label": "Conjuration"
        },
        {
          "value": 22,
          "label": "Destruction"
        },
        {
          "value": 23,
          "label": "Illusion"
        },
        {
          "value": 24,
          "label": "Mysticism"
        },
        {
          "value": 25,
          "label": "Restoration"
        },
        {
          "value": 26,
          "label": "Acrobatics"
        },
        {
          "value": 27,
          "label": "Light Armor"
        },
        {
          "value": 28,
          "label": "Marksman"
        },
        {
          "value": 29,
          "label": "Mercantile"
        },
        {
          "value": 30,
          "label": "Security"
        },
        {
          "value": 31,
          "label": "Sneak"
        },
        {
          "value": 32,
          "label": "Speechcraft"
        },
        {
          "value": 33,
          "label": "Aggression"
        },
        {
          "value": 34,
          "label": "Confidence"
        },
        {
          "value": 35,
          "label": "Energy"
        },
        {
          "value": 36,
          "label": "Responsibility"
        },
        {
          "value": 37,
          "label": "Bounty"
        },
        {
          "value": 38,
          "label": "Fame"
        },
        {
          "value": 39,
          "label": "Infamy"
        },
        {
          "value": 40,
          "label": "Magicka Multiplier"
        },
        {
          "value": 41,
          "label": "Night Eye Bonus"
        },
        {
          "value": 42,
          "label": "Attack Bonus"
        },
        {
          "value": 43,
          "label": "Defend Bonus"
        },
        {
          "value": 44,
          "label": "Casting Penalty"
        },
        {
          "value": 45,
          "label": "Blindness"
        },
        {
          "value": 46,
          "label": "Chameleon"
        },
        {
          "value": 47,
          "label": "Invisibility"
        },
        {
          "value": 48,
          "label": "Paralysis"
        },
        {
          "value": 49,
          "label": "Silence"
        },
        {
          "value": 50,
          "label": "Confusion"
        },
        {
          "value": 51,
          "label": "Detect Item Range"
        },
        {
          "value": 52,
          "label": "Spell Absorb Chance"
        },
        {
          "value": 53,
          "label": "Spell Reflect Chance"
        },
        {
          "value": 54,
          "label": "Swim Speed Multiplier"
        },
        {
          "value": 55,
          "label": "Water Breathing"
        },
        {
          "value": 56,
          "label": "Water Walking"
        },
        {
          "value": 57,
          "label": "Stunted Magicka"
        },
        {
          "value": 58,
          "label": "Detect Life Range"
        },
        {
          "value": 59,
          "label": "Reflect Damage"
        },
        {
          "value": 60,
          "label": "Telekinesis"
        },
        {
          "value": 61,
          "label": "Resist Fire"
        },
        {
          "value": 62,
          "label": "Resist Frost"
        },
        {
          "value": 63,
          "label": "Resist Disease"
        },
        {
          "value": 64,
          "label": "Resist Magic"
        },
        {
          "value": 65,
          "label": "Resist Normal Weapons"
        },
        {
          "value": 66,
          "label": "Resist Paralysis"
        },
        {
          "value": 67,
          "label": "Resist Poison"
        },
        {
          "value": 68,
          "label": "Resist Shock"
        },
        {
          "value": 69,
          "label": "Vampirism"
        },
        {
          "value": 70,
          "label": "Darkness"
        },
        {
          "value": 71,
          "label": "Resist Water Damage"
        }
      ],
      "default_value": 0
    },
    {
      "id": "attribute_enum",
      "values": [
        {
          "value": 0,
          "id": "strength"
        },
        {
          "value": 1,
          "id": "intelligence"
        },
        {
          "value": 2,
          "id": "willpower"
        },
        {
          "value": 3,
          "id": "agility"
        },
        {
          "value": 4,
          "id": "speed"
        },
        {
          "value": 5,
          "id": "endurance"
        },
        {
          "value": 6,
          "id": "personality"
        },
        {
          "value": 7,
          "id": "luck"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "Strength"
        },
        {
          "value": 1,
          "label": "Intelligence"
        },
        {
          "value": 2,
          "label": "Willpower"
        },
        {
          "value": 3,
          "label": "Agility"
        },
        {
          "value": 4,
          "label": "Speed"
        },
        {
          "value": 5,
          "label": "Endurance"
        },
        {
          "value": 6,
          "label": "Personality"
        },
        {
          "value": 7,
          "label": "Luck"
        }
      ],
      "default_value": 0
    },
    {
      "id": "biped_flags",
      "values": [
        {
          "value": 1,
          "id": "head"
        },
        {
          "value": 2,
          "id": "hair"
        },
        {
          "value": 4,
          "id": "upper_body"
        },
        {
          "value": 8,
          "id": "lower_body"
        },
        {
          "value": 16,
          "id": "hand"
        },
        {
          "value": 32,
          "id": "foot"
        },
        {
          "value": 64,
          "id": "right_ring"
        },
        {
          "value": 128,
          "id": "left_ring"
        },
        {
          "value": 256,
          "id": "amulet"
        },
        {
          "value": 512,
          "id": "weapon"
        },
        {
          "value": 1024,
          "id": "back_weapon"
        },
        {
          "value": 2048,
          "id": "side_weapon"
        },
        {
          "value": 4096,
          "id": "quiver"
        },
        {
          "value": 8192,
          "id": "shield"
        },
        {
          "value": 16384,
          "id": "torch"
        },
        {
          "value": 32768,
          "id": "tail"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Head"
        },
        {
          "value": 2,
          "label": "Hair"
        },
        {
          "value": 4,
          "label": "Upper Body"
        },
        {
          "value": 8,
          "label": "Lower Body"
        },
        {
          "value": 16,
          "label": "Hand"
        },
        {
          "value": 32,
          "label": "Foot"
        },
        {
          "value": 64,
          "label": "Right Ring"
        },
        {
          "value": 128,
          "label": "Left Ring"
        },
        {
          "value": 256,
          "label": "Amulet"
        },
        {
          "value": 512,
          "label": "Weapon"
        },
        {
          "value": 1024,
          "label": "Back Weapon"
        },
        {
          "value": 2048,
          "label": "Side Weapon"
        },
        {
          "value": 4096,
          "label": "Quiver"
        },
        {
          "value": 8192,
          "label": "Shield"
        },
        {
          "value": 16384,
          "label": "Torch"
        },
        {
          "value": 32768,
          "label": "Tail"
        }
      ],
      "storage_kind": "flags",
      "byte_width": 2
    },
    {
      "id": "blend_mode_enum",
      "values": [
        {
          "value": 0,
          "id": "value"
        },
        {
          "value": 1,
          "id": "zero"
        },
        {
          "value": 2,
          "id": "one"
        },
        {
          "value": 3,
          "id": "source_color"
        },
        {
          "value": 4,
          "id": "source_inverse_color"
        },
        {
          "value": 5,
          "id": "source_alpha"
        },
        {
          "value": 6,
          "id": "source_inverted_alpha"
        },
        {
          "value": 7,
          "id": "dest_alpha"
        },
        {
          "value": 8,
          "id": "dest_inverted_alpha"
        },
        {
          "value": 9,
          "id": "dest_color"
        },
        {
          "value": 10,
          "id": "dest_inverse_color"
        },
        {
          "value": 11,
          "id": "source_alpha_sat"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": ""
        },
        {
          "value": 1,
          "label": "Zero"
        },
        {
          "value": 2,
          "label": "One"
        },
        {
          "value": 3,
          "label": "Source Color"
        },
        {
          "value": 4,
          "label": "Source Inverse Color"
        },
        {
          "value": 5,
          "label": "Source Alpha"
        },
        {
          "value": 6,
          "label": "Source Inverted Alpha"
        },
        {
          "value": 7,
          "label": "Dest Alpha"
        },
        {
          "value": 8,
          "label": "Dest Inverted Alpha"
        },
        {
          "value": 9,
          "label": "Dest Color"
        },
        {
          "value": 10,
          "label": "Dest Inverse Color"
        },
        {
          "value": 11,
          "label": "Source Alpha SAT"
        }
      ],
      "default_value": 0
    },
    {
      "id": "blend_op_enum",
      "values": [
        {
          "value": 0,
          "id": "value"
        },
        {
          "value": 1,
          "id": "add"
        },
        {
          "value": 2,
          "id": "subtract"
        },
        {
          "value": 3,
          "id": "reverse_subtract"
        },
        {
          "value": 4,
          "id": "minimum"
        },
        {
          "value": 5,
          "id": "maximum"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": ""
        },
        {
          "value": 1,
          "label": "Add"
        },
        {
          "value": 2,
          "label": "Subtract"
        },
        {
          "value": 3,
          "label": "Reverse Subtract"
        },
        {
          "value": 4,
          "label": "Minimum"
        },
        {
          "value": 5,
          "label": "Maximum"
        }
      ],
      "default_value": 0
    },
    {
      "id": "body_part_index_enum",
      "values": [
        {
          "value": 0,
          "id": "upper_body"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "Upper Body"
        }
      ],
      "default_value": 0
    },
    {
      "id": "bool_enum",
      "values": [
        {
          "value": 0,
          "id": "false"
        },
        {
          "value": 1,
          "id": "true"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "False"
        },
        {
          "value": 1,
          "label": "True"
        }
      ],
      "byte_width": 1,
      "default_value": 0
    },
    {
      "id": "dialogue_type_enum",
      "values": [
        {
          "value": 0,
          "id": "topic"
        },
        {
          "value": 1,
          "id": "conversation"
        },
        {
          "value": 2,
          "id": "combat"
        },
        {
          "value": 3,
          "id": "persuasion"
        },
        {
          "value": 4,
          "id": "detection"
        },
        {
          "value": 5,
          "id": "service"
        },
        {
          "value": 6,
          "id": "miscellaneous"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "Topic"
        },
        {
          "value": 1,
          "label": "Conversation"
        },
        {
          "value": 2,
          "label": "Combat"
        },
        {
          "value": 3,
          "label": "Persuasion"
        },
        {
          "value": 4,
          "label": "Detection"
        },
        {
          "value": 5,
          "label": "Service"
        },
        {
          "value": 6,
          "label": "Miscellaneous"
        }
      ],
      "byte_width": 1,
      "default_value": 0
    },
    {
      "id": "effect_type_enum",
      "values": [
        {
          "value": 0,
          "id": "self"
        },
        {
          "value": 1,
          "id": "touch"
        },
        {
          "value": 2,
          "id": "target"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "Self"
        },
        {
          "value": 1,
          "label": "Touch"
        },
        {
          "value": 2,
          "label": "Target"
        }
      ],
      "default_value": 0
    },
    {
      "id": "magic_school_enum",
      "values": [
        {
          "value": 0,
          "id": "alteration"
        },
        {
          "value": 1,
          "id": "conjuration"
        },
        {
          "value": 2,
          "id": "destruction"
        },
        {
          "value": 3,
          "id": "illusion"
        },
        {
          "value": 4,
          "id": "mysticism"
        },
        {
          "value": 5,
          "id": "restoration"
        },
        {
          "value": 6,
          "id": "none"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "Alteration"
        },
        {
          "value": 1,
          "label": "Conjuration"
        },
        {
          "value": 2,
          "label": "Destruction"
        },
        {
          "value": 3,
          "label": "Illusion"
        },
        {
          "value": 4,
          "label": "Mysticism"
        },
        {
          "value": 5,
          "label": "Restoration"
        },
        {
          "value": 6,
          "label": "None"
        }
      ],
      "default_value": 0
    },
    {
      "id": "major_skill_enum",
      "values": [
        {
          "value": 12,
          "id": "armorer"
        },
        {
          "value": 13,
          "id": "athletics"
        },
        {
          "value": 14,
          "id": "blade"
        },
        {
          "value": 15,
          "id": "block"
        },
        {
          "value": 16,
          "id": "blunt"
        },
        {
          "value": 17,
          "id": "hand_to_hand"
        },
        {
          "value": 18,
          "id": "heavy_armor"
        },
        {
          "value": 19,
          "id": "alchemy"
        },
        {
          "value": 20,
          "id": "alteration"
        },
        {
          "value": 21,
          "id": "conjuration"
        },
        {
          "value": 22,
          "id": "destruction"
        },
        {
          "value": 23,
          "id": "illusion"
        },
        {
          "value": 24,
          "id": "mysticism"
        },
        {
          "value": 25,
          "id": "restoration"
        },
        {
          "value": 26,
          "id": "acrobatics"
        },
        {
          "value": 27,
          "id": "light_armor"
        },
        {
          "value": 28,
          "id": "marksman"
        },
        {
          "value": 29,
          "id": "mercantile"
        },
        {
          "value": 30,
          "id": "security"
        },
        {
          "value": 31,
          "id": "sneak"
        },
        {
          "value": 32,
          "id": "speechcraft"
        }
      ],
      "labels": [
        {
          "value": 12,
          "label": "Armorer"
        },
        {
          "value": 13,
          "label": "Athletics"
        },
        {
          "value": 14,
          "label": "Blade"
        },
        {
          "value": 15,
          "label": "Block"
        },
        {
          "value": 16,
          "label": "Blunt"
        },
        {
          "value": 17,
          "label": "Hand To Hand"
        },
        {
          "value": 18,
          "label": "Heavy Armor"
        },
        {
          "value": 19,
          "label": "Alchemy"
        },
        {
          "value": 20,
          "label": "Alteration"
        },
        {
          "value": 21,
          "label": "Conjuration"
        },
        {
          "value": 22,
          "label": "Destruction"
        },
        {
          "value": 23,
          "label": "Illusion"
        },
        {
          "value": 24,
          "label": "Mysticism"
        },
        {
          "value": 25,
          "label": "Restoration"
        },
        {
          "value": 26,
          "label": "Acrobatics"
        },
        {
          "value": 27,
          "label": "Light Armor"
        },
        {
          "value": 28,
          "label": "Marksman"
        },
        {
          "value": 29,
          "label": "Mercantile"
        },
        {
          "value": 30,
          "label": "Security"
        },
        {
          "value": 31,
          "label": "Sneak"
        },
        {
          "value": 32,
          "label": "Speechcraft"
        }
      ]
    },
    {
      "id": "music_enum",
      "values": [
        {
          "value": 0,
          "id": "default"
        },
        {
          "value": 1,
          "id": "public"
        },
        {
          "value": 2,
          "id": "dungeon"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "Default"
        },
        {
          "value": 1,
          "label": "Public"
        },
        {
          "value": 2,
          "label": "Dungeon"
        }
      ],
      "byte_width": 1,
      "default_value": 0
    },
    {
      "id": "package_flags",
      "values": [
        {
          "value": 1,
          "id": "offers_services"
        },
        {
          "value": 4,
          "id": "must_complete"
        },
        {
          "value": 64,
          "id": "unlock_doors_at_package_start"
        },
        {
          "value": 128,
          "id": "unlock_doors_at_package_end"
        },
        {
          "value": 512,
          "id": "continue_if_pc_near"
        },
        {
          "value": 1024,
          "id": "once_per_day"
        },
        {
          "value": 131072,
          "id": "always_sneak"
        },
        {
          "value": 262144,
          "id": "allow_swimming"
        },
        {
          "value": 2097152,
          "id": "weapons_unequipped"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Offers Services"
        },
        {
          "value": 4,
          "label": "Must Complete"
        },
        {
          "value": 64,
          "label": "Unlock Doors At Package Start"
        },
        {
          "value": 128,
          "label": "Unlock Doors At Package End"
        },
        {
          "value": 512,
          "label": "Continue If PC Near"
        },
        {
          "value": 1024,
          "label": "Once Per Day"
        },
        {
          "value": 131072,
          "label": "Always Sneak"
        },
        {
          "value": 262144,
          "label": "Allow Swimming"
        },
        {
          "value": 2097152,
          "label": "Weapons Unequipped"
        }
      ],
      "storage_kind": "flags",
      "byte_width": 2
    },
    {
      "id": "package_schedule_day_of_month_enum",
      "values": [
        {
          "value": 0,
          "id": "any"
        },
        {
          "value": 1,
          "id": "1"
        },
        {
          "value": 2,
          "id": "2"
        },
        {
          "value": 3,
          "id": "3"
        },
        {
          "value": 4,
          "id": "4"
        },
        {
          "value": 5,
          "id": "5"
        },
        {
          "value": 6,
          "id": "6"
        },
        {
          "value": 7,
          "id": "7"
        },
        {
          "value": 8,
          "id": "8"
        },
        {
          "value": 9,
          "id": "9"
        },
        {
          "value": 10,
          "id": "10"
        },
        {
          "value": 11,
          "id": "11"
        },
        {
          "value": 12,
          "id": "12"
        },
        {
          "value": 13,
          "id": "13"
        },
        {
          "value": 14,
          "id": "14"
        },
        {
          "value": 15,
          "id": "15"
        },
        {
          "value": 16,
          "id": "16"
        },
        {
          "value": 17,
          "id": "17"
        },
        {
          "value": 18,
          "id": "18"
        },
        {
          "value": 19,
          "id": "19"
        },
        {
          "value": 20,
          "id": "20"
        },
        {
          "value": 21,
          "id": "21"
        },
        {
          "value": 22,
          "id": "22"
        },
        {
          "value": 23,
          "id": "23"
        },
        {
          "value": 24,
          "id": "24"
        },
        {
          "value": 25,
          "id": "25"
        },
        {
          "value": 26,
          "id": "26"
        },
        {
          "value": 27,
          "id": "27"
        },
        {
          "value": 28,
          "id": "28"
        },
        {
          "value": 29,
          "id": "29"
        },
        {
          "value": 30,
          "id": "30"
        },
        {
          "value": 31,
          "id": "31"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "Any"
        },
        {
          "value": 1,
          "label": "1"
        },
        {
          "value": 2,
          "label": "2"
        },
        {
          "value": 3,
          "label": "3"
        },
        {
          "value": 4,
          "label": "4"
        },
        {
          "value": 5,
          "label": "5"
        },
        {
          "value": 6,
          "label": "6"
        },
        {
          "value": 7,
          "label": "7"
        },
        {
          "value": 8,
          "label": "8"
        },
        {
          "value": 9,
          "label": "9"
        },
        {
          "value": 10,
          "label": "10"
        },
        {
          "value": 11,
          "label": "11"
        },
        {
          "value": 12,
          "label": "12"
        },
        {
          "value": 13,
          "label": "13"
        },
        {
          "value": 14,
          "label": "14"
        },
        {
          "value": 15,
          "label": "15"
        },
        {
          "value": 16,
          "label": "16"
        },
        {
          "value": 17,
          "label": "17"
        },
        {
          "value": 18,
          "label": "18"
        },
        {
          "value": 19,
          "label": "19"
        },
        {
          "value": 20,
          "label": "20"
        },
        {
          "value": 21,
          "label": "21"
        },
        {
          "value": 22,
          "label": "22"
        },
        {
          "value": 23,
          "label": "23"
        },
        {
          "value": 24,
          "label": "24"
        },
        {
          "value": 25,
          "label": "25"
        },
        {
          "value": 26,
          "label": "26"
        },
        {
          "value": 27,
          "label": "27"
        },
        {
          "value": 28,
          "label": "28"
        },
        {
          "value": 29,
          "label": "29"
        },
        {
          "value": 30,
          "label": "30"
        },
        {
          "value": 31,
          "label": "31"
        }
      ],
      "byte_width": 1,
      "default_value": 0
    },
    {
      "id": "package_schedule_day_of_week_enum",
      "values": [
        {
          "value": 0,
          "id": "sunday"
        },
        {
          "value": 1,
          "id": "monday"
        },
        {
          "value": 2,
          "id": "tuesday"
        },
        {
          "value": 3,
          "id": "wednesday"
        },
        {
          "value": 4,
          "id": "thursday"
        },
        {
          "value": 5,
          "id": "friday"
        },
        {
          "value": 6,
          "id": "saturday"
        },
        {
          "value": 7,
          "id": "weekdays_mtwtf"
        },
        {
          "value": 8,
          "id": "weekends_ss"
        },
        {
          "value": 9,
          "id": "monday_wednesday_friday"
        },
        {
          "value": 10,
          "id": "tuesday_thursday"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "Sunday"
        },
        {
          "value": 1,
          "label": "Monday"
        },
        {
          "value": 2,
          "label": "Tuesday"
        },
        {
          "value": 3,
          "label": "Wednesday"
        },
        {
          "value": 4,
          "label": "Thursday"
        },
        {
          "value": 5,
          "label": "Friday"
        },
        {
          "value": 6,
          "label": "Saturday"
        },
        {
          "value": 7,
          "label": "Weekdays (MTWTF)"
        },
        {
          "value": 8,
          "label": "Weekends (SS)"
        },
        {
          "value": 9,
          "label": "Monday, Wednesday, Friday"
        },
        {
          "value": 10,
          "label": "Tuesday, Thursday"
        }
      ],
      "byte_width": 1,
      "default_value": 0
    },
    {
      "id": "package_schedule_hours_enum",
      "values": [
        {
          "value": 0,
          "id": "0"
        },
        {
          "value": 1,
          "id": "1"
        },
        {
          "value": 2,
          "id": "2"
        },
        {
          "value": 3,
          "id": "3"
        },
        {
          "value": 4,
          "id": "4"
        },
        {
          "value": 5,
          "id": "5"
        },
        {
          "value": 6,
          "id": "6"
        },
        {
          "value": 7,
          "id": "7"
        },
        {
          "value": 8,
          "id": "8"
        },
        {
          "value": 9,
          "id": "9"
        },
        {
          "value": 10,
          "id": "10"
        },
        {
          "value": 11,
          "id": "11"
        },
        {
          "value": 12,
          "id": "12"
        },
        {
          "value": 13,
          "id": "13"
        },
        {
          "value": 14,
          "id": "14"
        },
        {
          "value": 15,
          "id": "15"
        },
        {
          "value": 16,
          "id": "16"
        },
        {
          "value": 17,
          "id": "17"
        },
        {
          "value": 18,
          "id": "18"
        },
        {
          "value": 19,
          "id": "19"
        },
        {
          "value": 20,
          "id": "20"
        },
        {
          "value": 21,
          "id": "21"
        },
        {
          "value": 22,
          "id": "22"
        },
        {
          "value": 23,
          "id": "23"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "0"
        },
        {
          "value": 1,
          "label": "1"
        },
        {
          "value": 2,
          "label": "2"
        },
        {
          "value": 3,
          "label": "3"
        },
        {
          "value": 4,
          "label": "4"
        },
        {
          "value": 5,
          "label": "5"
        },
        {
          "value": 6,
          "label": "6"
        },
        {
          "value": 7,
          "label": "7"
        },
        {
          "value": 8,
          "label": "8"
        },
        {
          "value": 9,
          "label": "9"
        },
        {
          "value": 10,
          "label": "10"
        },
        {
          "value": 11,
          "label": "11"
        },
        {
          "value": 12,
          "label": "12"
        },
        {
          "value": 13,
          "label": "13"
        },
        {
          "value": 14,
          "label": "14"
        },
        {
          "value": 15,
          "label": "15"
        },
        {
          "value": 16,
          "label": "16"
        },
        {
          "value": 17,
          "label": "17"
        },
        {
          "value": 18,
          "label": "18"
        },
        {
          "value": 19,
          "label": "19"
        },
        {
          "value": 20,
          "label": "20"
        },
        {
          "value": 21,
          "label": "21"
        },
        {
          "value": 22,
          "label": "22"
        },
        {
          "value": 23,
          "label": "23"
        }
      ],
      "byte_width": 1,
      "default_value": 0
    },
    {
      "id": "package_type_enum",
      "values": [
        {
          "value": 0,
          "id": "find"
        },
        {
          "value": 1,
          "id": "follow"
        },
        {
          "value": 2,
          "id": "escort"
        },
        {
          "value": 3,
          "id": "eat"
        },
        {
          "value": 4,
          "id": "sleep"
        },
        {
          "value": 5,
          "id": "wander"
        },
        {
          "value": 6,
          "id": "travel"
        },
        {
          "value": 7,
          "id": "accompany"
        },
        {
          "value": 8,
          "id": "use_item_at"
        },
        {
          "value": 9,
          "id": "ambush"
        },
        {
          "value": 10,
          "id": "flee_not_combat"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "Find"
        },
        {
          "value": 1,
          "label": "Follow"
        },
        {
          "value": 2,
          "label": "Escort"
        },
        {
          "value": 3,
          "label": "Eat"
        },
        {
          "value": 4,
          "label": "Sleep"
        },
        {
          "value": 5,
          "label": "Wander"
        },
        {
          "value": 6,
          "label": "Travel"
        },
        {
          "value": 7,
          "label": "Accompany"
        },
        {
          "value": 8,
          "label": "Use Item At"
        },
        {
          "value": 9,
          "label": "Ambush"
        },
        {
          "value": 10,
          "label": "Flee Not Combat"
        }
      ],
      "byte_width": 1,
      "default_value": 0
    },
    {
      "id": "pgag_flags",
      "values": [
        {
          "value": 1,
          "id": "point_1"
        },
        {
          "value": 2,
          "id": "point_2"
        },
        {
          "value": 4,
          "id": "point_3"
        },
        {
          "value": 8,
          "id": "point_4"
        },
        {
          "value": 16,
          "id": "point_5"
        },
        {
          "value": 32,
          "id": "point_6"
        },
        {
          "value": 64,
          "id": "point_7"
        },
        {
          "value": 128,
          "id": "point_8"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Point 1"
        },
        {
          "value": 2,
          "label": "Point 2"
        },
        {
          "value": 4,
          "label": "Point 3"
        },
        {
          "value": 8,
          "label": "Point 4"
        },
        {
          "value": 16,
          "label": "Point 5"
        },
        {
          "value": 32,
          "label": "Point 6"
        },
        {
          "value": 64,
          "label": "Point 7"
        },
        {
          "value": 128,
          "label": "Point 8"
        }
      ],
      "storage_kind": "flags",
      "byte_width": 1
    },
    {
      "id": "quadrant_enum",
      "values": [
        {
          "value": 0,
          "id": "bottom_left"
        },
        {
          "value": 1,
          "id": "bottom_right"
        },
        {
          "value": 2,
          "id": "top_left"
        },
        {
          "value": 3,
          "id": "top_right"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "Bottom Left"
        },
        {
          "value": 1,
          "label": "Bottom Right"
        },
        {
          "value": 2,
          "label": "Top Left"
        },
        {
          "value": 3,
          "label": "Top Right"
        }
      ],
      "byte_width": 1,
      "default_value": 0
    },
    {
      "id": "service_flags",
      "values": [
        {
          "value": 1,
          "id": "weapons"
        },
        {
          "value": 2,
          "id": "armor"
        },
        {
          "value": 4,
          "id": "clothing"
        },
        {
          "value": 8,
          "id": "books"
        },
        {
          "value": 16,
          "id": "ingredients"
        },
        {
          "value": 32,
          "id": "value"
        },
        {
          "value": 64,
          "id": "value"
        },
        {
          "value": 128,
          "id": "lights"
        },
        {
          "value": 256,
          "id": "apparatus"
        },
        {
          "value": 512,
          "id": "value"
        },
        {
          "value": 1024,
          "id": "miscellaneous"
        },
        {
          "value": 2048,
          "id": "spells"
        },
        {
          "value": 4096,
          "id": "magic_items"
        },
        {
          "value": 8192,
          "id": "potions"
        },
        {
          "value": 16384,
          "id": "training"
        },
        {
          "value": 32768,
          "id": "value"
        },
        {
          "value": 65536,
          "id": "recharge"
        },
        {
          "value": 131072,
          "id": "repair"
        }
      ],
      "labels": [
        {
          "value": 1,
          "label": "Weapons"
        },
        {
          "value": 2,
          "label": "Armor"
        },
        {
          "value": 4,
          "label": "Clothing"
        },
        {
          "value": 8,
          "label": "Books"
        },
        {
          "value": 16,
          "label": "Ingredients"
        },
        {
          "value": 32,
          "label": ""
        },
        {
          "value": 64,
          "label": ""
        },
        {
          "value": 128,
          "label": "Lights"
        },
        {
          "value": 256,
          "label": "Apparatus"
        },
        {
          "value": 512,
          "label": ""
        },
        {
          "value": 1024,
          "label": "Miscellaneous"
        },
        {
          "value": 2048,
          "label": "Spells"
        },
        {
          "value": 4096,
          "label": "Magic Items"
        },
        {
          "value": 8192,
          "label": "Potions"
        },
        {
          "value": 16384,
          "label": "Training"
        },
        {
          "value": 32768,
          "label": ""
        },
        {
          "value": 65536,
          "label": "Recharge"
        },
        {
          "value": 131072,
          "label": "Repair"
        }
      ],
      "storage_kind": "flags"
    },
    {
      "id": "skill_enum",
      "values": [
        {
          "value": 0,
          "id": "armorer"
        },
        {
          "value": 1,
          "id": "athletics"
        },
        {
          "value": 2,
          "id": "blade"
        },
        {
          "value": 3,
          "id": "block"
        },
        {
          "value": 4,
          "id": "blunt"
        },
        {
          "value": 5,
          "id": "hand_to_hand"
        },
        {
          "value": 6,
          "id": "heavy_armor"
        },
        {
          "value": 7,
          "id": "alchemy"
        },
        {
          "value": 8,
          "id": "alteration"
        },
        {
          "value": 9,
          "id": "conjuration"
        },
        {
          "value": 10,
          "id": "destruction"
        },
        {
          "value": 11,
          "id": "illusion"
        },
        {
          "value": 12,
          "id": "mysticism"
        },
        {
          "value": 13,
          "id": "restoration"
        },
        {
          "value": 14,
          "id": "acrobatics"
        },
        {
          "value": 15,
          "id": "light_armor"
        },
        {
          "value": 16,
          "id": "marksman"
        },
        {
          "value": 17,
          "id": "mercantile"
        },
        {
          "value": 18,
          "id": "security"
        },
        {
          "value": 19,
          "id": "sneak"
        },
        {
          "value": 20,
          "id": "speechcraft"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "Armorer"
        },
        {
          "value": 1,
          "label": "Athletics"
        },
        {
          "value": 2,
          "label": "Blade"
        },
        {
          "value": 3,
          "label": "Block"
        },
        {
          "value": 4,
          "label": "Blunt"
        },
        {
          "value": 5,
          "label": "Hand To Hand"
        },
        {
          "value": 6,
          "label": "Heavy Armor"
        },
        {
          "value": 7,
          "label": "Alchemy"
        },
        {
          "value": 8,
          "label": "Alteration"
        },
        {
          "value": 9,
          "label": "Conjuration"
        },
        {
          "value": 10,
          "label": "Destruction"
        },
        {
          "value": 11,
          "label": "Illusion"
        },
        {
          "value": 12,
          "label": "Mysticism"
        },
        {
          "value": 13,
          "label": "Restoration"
        },
        {
          "value": 14,
          "label": "Acrobatics"
        },
        {
          "value": 15,
          "label": "Light Armor"
        },
        {
          "value": 16,
          "label": "Marksman"
        },
        {
          "value": 17,
          "label": "Mercantile"
        },
        {
          "value": 18,
          "label": "Security"
        },
        {
          "value": 19,
          "label": "Sneak"
        },
        {
          "value": 20,
          "label": "Speechcraft"
        }
      ],
      "byte_width": 1,
      "default_value": 0
    },
    {
      "id": "soul_gem_enum",
      "values": [
        {
          "value": 0,
          "id": "none"
        },
        {
          "value": 1,
          "id": "petty"
        },
        {
          "value": 2,
          "id": "lesser"
        },
        {
          "value": 3,
          "id": "common"
        },
        {
          "value": 4,
          "id": "greater"
        },
        {
          "value": 5,
          "id": "grand"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "None"
        },
        {
          "value": 1,
          "label": "Petty"
        },
        {
          "value": 2,
          "label": "Lesser"
        },
        {
          "value": 3,
          "label": "Common"
        },
        {
          "value": 4,
          "label": "Greater"
        },
        {
          "value": 5,
          "label": "Grand"
        }
      ],
      "byte_width": 1,
      "default_value": 0
    },
    {
      "id": "specialization_enum",
      "values": [
        {
          "value": 0,
          "id": "combat"
        },
        {
          "value": 1,
          "id": "magic"
        },
        {
          "value": 2,
          "id": "stealth"
        }
      ],
      "labels": [
        {
          "value": 0,
          "label": "Combat"
        },
        {
          "value": 1,
          "label": "Magic"
        },
        {
          "value": 2,
          "label": "Stealth"
        }
      ],
      "default_value": 0
    },
    {
      "id": "z_test_func_enum",
      "values": [
        {
          "value": 3,
          "id": "equal_to"
        },
        {
          "value": 5,
          "id": "greater_than"
        },
        {
          "value": 7,
          "id": "greater_than_or_equal_to"
        },
        {
          "value": 8,
          "id": "always_show"
        }
      ],
      "labels": [
        {
          "value": 3,
          "label": "Equal To"
        },
        {
          "value": 5,
          "label": "Greater Than"
        },
        {
          "value": 7,
          "label": "Greater Than or Equal To"
        },
        {
          "value": 8,
          "label": "Always Show"
        }
      ]
    }
  ]
}
"#;
