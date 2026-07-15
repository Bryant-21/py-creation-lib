# Starfield Native Animation Formats — Binary Specification

**Byte order:** All binary formats are **little-endian** (x86/x64 native).

Starfield replaced Havok for animations with proprietary BGS formats. Havok is retained only for physics/collision (`hknp*`).

| Component       | Legacy (FO4/Skyrim) | Starfield        |
|-----------------|---------------------|------------------|
| Skeletons       | `.hkx` (hkaSkeleton)     | `.rig`    |
| Animations      | `.hkx` (hkaAnimation)    | `.af`     |
| Behavior Graphs | `.hkx` (hkbBehaviorGraph)| `.agx` (XML) |

**Dataset:** 394 `.rig`, 19,372 `.af`, 347 `.agx` files in `extracted/starfield/meshes/`.

**References:**
- `refs/CALUMI.Animation/CALUMI.Animation/SFBGS_SkeletonRig.cpp` — rig binary format
- `refs/CALUMI.Animation/CALUMI.Animation/SFBGS_Animation.h` / `.cpp` — af binary format
- `refs/CALUMI.Motion/CALUMI.Motion/AgxNodes/AgxNode.h` — agx node type enum
- `refs/CALUMI.Motion/CALUMI.Motion/CALUMIMotion.cpp` — agx XML I/O
- `refs/sf_animation_io/` — Python ctypes bindings (Blender addon, 140+ DLL function signatures)

---

## `.rig` Format — SFBGS Skeleton Rig

**Source:** `SFBGS_SkeletonRig.cpp` lines 568–793

Binary file consisting of four sequential sections: header, bone entries, bone map, and string table.

### Header (80 bytes)

| Offset | Size | Type   | Field              | Notes |
|--------|------|--------|--------------------|-------|
| 0      | 4    | i32    | version            | Always 5 |
| 4      | 4    | u32    | file_size          | Total file size in bytes |
| 8      | 4    | u32    | header_size        | Always 0x50 (80) |
| 12     | 4    | u32    | _pad1              | Always 0 |
| 16     | 4    | u32    | bone_map_offset    | = 80 + (96 * bone_count) |
| 20     | 4    | u32    | _pad2              | Always 0 |
| 24     | 8    | u64    | tracking_value_1   | Internal BGS tracking |
| 32     | 8    | u64    | tracking_value_2   | Same as tracking_value_1 |
| 40     | 8    | u64    | tracking_value_3   | Same as tracking_value_1 |
| 48     | 4    | f32    | low_precision      | Default 0.03125 (1/32); ships use 0.25 |
| 52     | 4    | f32    | high_precision     | Default 0.00025 (1/4000); ships use 0.002 |
| 56     | 2    | u16    | bone_count         | Number of bones |
| 58     | 2    | u16    | animated_bone_count| Non-twist bones (bones eligible for animation) |
| 60     | 4    | u32    | _pad3              | Always 0 |
| 64     | 16   | bytes  | _reserved          | End-of-header region, zeroed |

**Known constants:**
- `version` = 5 in all observed files
- `header_size` = 0x50 (80) in all observed files
- `bone_map_offset` is always exactly `80 + (96 * bone_count)`
- `low_precision` and `high_precision` control animation compression in paired `.af` files

### Bone Entry (96 bytes each, sequential after header)

Bone entries start at offset 80. There are `bone_count` entries, each 96 bytes.

| Offset | Size | Type   | Field                  | Notes |
|--------|------|--------|------------------------|-------|
| 0      | 16   | f32x4  | local_rotation         | Quaternion (w, x, y, z) — bone-local |
| 16     | 16   | f32x4  | global_rotation        | Quaternion (w, x, y, z) — model-space |
| 32     | 12   | f32x3  | position               | Translation (x, y, z) |
| 44     | 4    | i32    | bone_type              | -1 = Default, 1 = Twist |
| 48     | 8    | u64    | name_offset            | Absolute offset into string table |
| 56     | 4    | i32    | parent_index           | -1 for root bone |
| 60     | 4    | i32    | twist_driver_mqn_index | MQN index for twist driver |
| 64     | 4    | i32    | twist_driver_index     | Bone index of twist driver |
| 68     | 4    | i32    | _pad1                  | Always -1 |
| 72     | 4    | i32    | mirror_index           | Mirrored bone index (default = self) |
| 76     | 4    | i32    | _term                  | Terminator field |
| 80     | 4    | f32    | twist_driver_weight    | Weight for twist bone interpolation |
| 84     | 4    | i32    | _pad2                  | Padding |
| 88     | 4    | f32    | _unknown_scalar        | Purpose unknown |
| 92     | 4    | i32    | _pad3                  | Padding |

**struct format (Python):** `<4f4f3fiQiiiiiififi` (96 bytes)

**Quaternion convention:** Binary stores **wxyz**. For compatibility with the existing `SkeletonData` convention used by Havok parsers, the reader normalizes to **xyzw** on output.

### Bone Map (314 bytes)

Starts at `bone_map_offset` (= 80 + 96 * bone_count).

- 157 x i16 entries = 314 bytes total
- Constant count defined by `SFBGSMAPSIZE = 157` in `SFBGS_RigPackage.h`
- Maps standardized bone IDs (0–156) to rig-specific bone indices
- Initialized to -1 (0xFFFF as signed i16 = "unmapped")

**struct format (Python):** `<157h`

### String Table

Starts at `bone_map_offset + 314`.

- Null-terminated UTF-8 strings, packed sequentially
- Each bone's `name_offset` field points to the absolute file offset of its name string
- Read by scanning from the offset until `\x00`

### File Layout Summary

```
[0x00]  Header              80 bytes
[0x50]  Bone entries         96 * bone_count bytes
[...]   Bone map            314 bytes
[...]   String table        variable length
```

---

## `.af` Format — SFBGS Animation Format

**Source:** `SFBGS_Animation.h` lines 106–200, `SFBGS_Animation.cpp` lines 970–1018

Binary file with a 64-byte header followed by per-bone animation data blocks. Only the header is decoded by the current reader; full keyframe decoding (prefix folding compression) is deferred.

### Header (64 bytes)

| Offset | Size | Type   | Field              | Notes |
|--------|------|--------|--------------------|-------|
| 0      | 8    | u64    | magic              | File signature |
| 8      | 16   | f32x4  | header_rotation    | Reference quaternion (w, x, y, z) |
| 24     | 12   | f32x3  | header_translation | Reference position (x, y, z) |
| 36     | 4    | u8x4   | flags              | Byte 0 = bit field, bytes 1–3 = reserved (usually 0) |
| 40     | 2    | i16    | version            | Format version |
| 42     | 2    | u16    | bone_count         | Bones with animation data |
| 44     | 2    | u16    | frame_count        | Total animation frames |
| 46     | 2    | u16    | index_atlas_count  | Index atlas entries |
| 48     | 2    | u16    | fill_count         | Unknown fill count |
| 50     | 2    | u16    | preamble_offset    | Offset to preamble section |
| 52     | 12   | f32x3  | _zero_floats       | Three zero floats (reserved) |

### Flags Byte 0 — Bit Field

| Bit | Mask | Name                    | Description |
|-----|------|-------------------------|-------------|
| 0   | 0x01 | first_entry             | First entry flag |
| 1   | 0x02 | short_key_counters      | Use 2-byte (u16) key counters instead of 1-byte |
| 2   | 0x04 | short_key_frame_entries | Use 2-byte (u16) frame indices instead of 1-byte |
| 3   | 0x08 | has_scalar_sequence     | Scalar sequence data present |
| 4–7 |      | _reserved               | Unknown / unused |

### Derived Fields

- **duration** = `frame_count / 30.0` (Starfield uses 30 fps; constant `_DEFAULT_FPS` in reader)
- **compression_type** = `"sfbgs"` (distinguishes from Havok `hkaSplineCompressedAnimation`, etc.)
- Quaternion convention: binary stores **wxyz**, reader normalizes to **xyzw**

### Per-Bone Animation Blocks (metadata only — full decode deferred)

After the 64-byte header, each bone contains counted sequences of compressed keyframes:

| Sequence     | Key Index Type | Value Type | Notes |
|--------------|----------------|------------|-------|
| Rotation     | u16[]          | Compressed quaternion | Prefix-folded encoding |
| Translation  | u16[]          | Compressed vector     | Prefix-folded, dual precision |
| Scalar       | u16[]          | i16 (÷ 5000.0)       | Only if `has_scalar` flag set |
| Priority     | u16[]          | u8                    | Per-bone priority |

**Compression:** Prefix folding technique for rotation and translation data. Precision is controlled by the paired `.rig` file's `low_precision` and `high_precision` values. Full decompression requires both the `.af` and its corresponding `.rig`.

### File Layout Summary

```
[0x00]  Header                  64 bytes
[0x40]  Per-bone anim blocks    variable length (bone_count blocks)
```

---

## `.agx` Format — Animation Behavior Graph (XML)

**Source:** `CALUMIMotion.cpp` (I/O), `AgxNodes/AgxNode.h` lines 39–113 (node types)

Unlike `.rig` and `.af`, `.agx` files are **plain XML** parsed with pugiXML (C++) or `xml.etree.ElementTree` (Python reader).

### XML Schema

```xml
<root>
  <Name>Data\Meshes\AnimTextData\Tables\Graphs\filename.agx</Name>
  <Category>CategoryName</Category>
  <Link_Style>0</Link_Style>
  <node>
    <node_type>NT_GRAPH_REF</node_type>
    <name>Graph Reference</name>
    <guid>4b08b3d3-9ed8-4c47-8bec-...</guid>
    <noninstanced>False</noninstanced>
    <pos_x>362</pos_x>
    <pos_y>242</pos_y>
    <expanded_pos_x>362</expanded_pos_x>
    <expanded_pos_y>242</expanded_pos_y>
    <use_color_2>False</use_color_2>
    <user_id>0</user_id>
    <collapsed>True</collapsed>
    <output>
      <name>Out</name>
      <id>1</id>
      <idx>0</idx>
    </output>
    <input>
      <name>In</name>
      <id>2</id>
      <idx>0</idx>
      <connection>
        <id>...</id>
        <node_index>...</node_index>
      </connection>
    </input>
    <property_sheet>
      <num_columns>2</num_columns>
      <column> <header>Property</header> <types>5</types> </column>
      <column> <header>Value</header> <types>5</types> </column>
      <row>
        <prop> <type>2</type> <value>Name</value> </prop>
        <prop> <type>2</type> <value>value</value> </prop>
      </row>
    </property_sheet>
  </node>
</root>
```

### Top-Level Elements

| Element      | Type   | Description |
|--------------|--------|-------------|
| `<Name>`     | string | Full path name (e.g., `Data\Meshes\AnimTextData\Tables\Graphs\filename.agx`) |
| `<Category>` | string | Graph category (e.g., `System`, `Player`, `Creature`, `Effect`) |
| `<Link_Style>` | int  | Visual link style for editor (0 = default) |
| `<node>`     | element| Repeated — one per graph node |

### Node Elements

Each `<node>` contains:

| Element          | Type   | Description |
|------------------|--------|-------------|
| `<node_type>`    | string | Node type enum value (see table below) |
| `<name>`         | string | User-visible node name |
| `<guid>`         | string | Unique identifier (UUID format) |
| `<noninstanced>` | bool   | Whether node is non-instanced |
| `<pos_x>`, `<pos_y>` | int | Editor canvas position |
| `<collapsed>`    | bool   | Editor collapse state |
| `<user_id>`      | int    | User-assigned ID |
| `<output>`       | element| Output port (name, id, idx) |
| `<input>`        | element| Input port with optional `<connection>` |
| `<property_sheet>` | element | Typed property table (columns + rows) |

### Indexing-Relevant Node Types

The reader extracts data from specific node types for BehaviorData-compatible output:

| Node Type               | Extracted Data | Field |
|-------------------------|----------------|-------|
| `NT_ANIMATION_NODE`     | `<animation_name>` | `sequences` |
| `NT_EVENT_CONTROLLER`   | `<event_name>` | `events` |
| `NT_ASSIGN_VARIABLE`    | `<variable_name>` | `variables` (name, "unknown") |
| `NT_EVALUATE_CONDITION_VARIABLE` | `<variable_name>` | `variables` |
| `NT_DAMPEN_VARIABLE`    | `<variable_name>` | `variables` |
| `NT_LINEAR_VARIABLE`    | `<variable_name>` | `variables` |
| `NT_MASS_SPRING_DAMPEN_VARIABLE` | `<variable_name>` | `variables` |
| `NT_VARIABLE_COMBINER`  | `<variable_name>` | `variables` |
| `NT_STATE_VARIABLE_CONTROL` | `<variable_name>` | `variables` |

### Node Type Enum (56+ types)

From `AgxNodes/AgxNode.h` lines 39–113. Enum type: `AgxNodeType : uint8_t`.

| Value | Name | Notes |
|-------|------|-------|
| 0     | `NT_ANIMATION_CORRECTED_NODE` | Unused |
| 1     | `NT_ANIMATION_IMPACT_SELECTOR` | |
| 2     | `NT_ANIMATION_NODE` | Animation clip playback |
| 3     | `NT_ASSIGN_ISTATE` | |
| 4     | `NT_ASSIGN_VARIABLE` | Variable assignment |
| 5     | `NT_BLEND_NODE` | Blend two inputs |
| 6     | `NT_BLEND_TREE_EMBEDDED` | Embedded blend tree |
| 7     | `NT_BONE_CONSTRAINT` | |
| 8     | `NT_CLONE_POSE` | |
| 9     | `NT_CONVERT_BONE_DATA_TO_VARIABLES` | |
| 10    | `NT_COPY_BONE_WEIGHTS` | |
| 11    | `NT_CRITICALLY_DAMPEN_VARIABLE` | Unused |
| 12    | `NT_CUMULATIVE_ANIMATION` | |
| 13    | `NT_CURVED_PATH_BLENDER` | |
| 14    | `NT_DAMPEN_VARIABLE` | Unused |
| 15    | `NT_DIRECT_AT` | |
| 16    | `NT_DUAL_DIRECT_AT` | |
| 17    | `NT_DYNAMIC_ANIMATION` | |
| 18    | `NT_DYNAMIC_GRAPH_REFERENCE` | |
| 19    | `NT_EFFECT_SEQUENCE` | |
| 20    | `NT_EVALUATE_CONDITION_VARIABLE` | Condition evaluation |
| 21    | `NT_EVENT_CONTROLLER` | Event emission |
| 22    | `NT_EVENT_FROM_RANGE` | Unused |
| 23    | `NT_EVERY_N_EVENTS_MODIFIER` | |
| 24    | `NT_FOOT_IK` | |
| 25    | `NT_GAMEBRYO_SEQUENCE` | |
| 26    | `NT_GRAPH_REF` | Sub-graph reference |
| 27    | `NT_INVALID` | Unused |
| 28    | `NT_LINEAR_VARIABLE` | |
| 29    | `NT_LOCOMOTION_BLEND` | |
| 30    | `NT_LOOK_AT` | Unused |
| 31    | `NT_MASS_SPRING_DAMPEN_VARIABLE` | |
| 32    | `NT_MATERIAL_LAYER_SEQUENCE` | |
| 33    | `NT_MERGE_NODE` | |
| 34    | `NT_MIRROR_MODIFIER` | |
| 35    | `NT_MODIFY_GROUP` | Unused |
| 36    | `NT_MOMENTUM_ANIMATION` | |
| 37    | `NT_MOMENTUM_SWITCHBACK` | |
| 38    | `NT_MULTI_FOOT_IK` | |
| 39    | `NT_NORMALIZE_ROTATION` | |
| 40    | `NT_NUM_ANIMATION_NODES` | Unused (sentinel) |
| 41    | `NT_PAIRED_ANIMATION` | |
| 42    | `NT_PARTICLE_SEQUENCE` | |
| 43    | `NT_PATHING_ANIMATIONS` | |
| 44    | `NT_PHYSICS_CONTACT_LISTENER` | |
| 45    | `NT_POST_BONE_MODIFIER_CONTROL` | |
| 46    | `NT_RAGDOLL` | Unused |
| 47    | `NT_RAGDOLL_DRIVE` | |
| 48    | `NT_RAGDOLL_GET_UP` | |
| 49    | `NT_RANDOM_ANIMATION_NODE` | |
| 50    | `NT_RIG_SWITCH` | |
| 51    | `NT_ROLLING_BONE` | Unused |
| 52    | `NT_ROOT_TWIST` | |
| 53    | `NT_ROTATION_VARIABLE` | |
| 54    | `NT_SET_ORIENT` | |
| 55    | `NT_SET_POS` | |
| 56    | `NT_SINGLE_BONE_IK` | Unused |
| 57    | `NT_SPEED_SCALE` | |
| 58    | `NT_STAGGER_METER` | |
| 59    | `NT_STATE_MACHINE_EMBEDDED` | Embedded state machine |
| 60    | `NT_STATE_VARIABLE_CONTROL` | |
| 61    | `NT_SWAP_GRAPH` | |
| 62    | `NT_SWITCH_NODE` | |
| 63    | `NT_TAG_PROPAGATION` | |
| 64    | `NT_TIMER_EVENT` | |
| 65    | `NT_TRANSLATION_ADJUSTMENT` | |
| 66    | `NT_TWO_BONE_IK` | |
| 67    | `NT_VARIABLE_COMBINER` | |
| 0xFD  | `Comment` | Editor-only comment node |
| 0xFE  | `DEBUG` | Debug node |
| 0xFF  | `UNDEFINED` | Invalid/undefined |

**Total:** 68 entries in the enum, of which 8 are marked unused, 3 are special (Comment, DEBUG, UNDEFINED), leaving 57 active node types.

---

## Cross-Format Relationships

```
.rig (skeleton)
  |
  |-- bone_count, low_precision, high_precision
  |     used by .af decompression
  |
  +-- bone_map: standardized bone ID -> rig bone index
        used for animation retargeting

.af (animation)
  |
  |-- references paired .rig for decompression precision
  |-- bone_count must match .rig bone_count
  |
  +-- keyframes: rotation, translation, scalar, priority per bone

.agx (behavior graph)
  |
  |-- NT_GRAPH_REF: references other .agx files
  |-- NT_ANIMATION_NODE: references .af clips by name
  |
  +-- drives animation state machine, blending, IK, events
```

The `.agx` behavior graph orchestrates `.af` animations on a `.rig` skeleton. Animation clips referenced by `NT_ANIMATION_NODE` nodes correspond to `.af` files. The `.rig` provides the bone hierarchy and precision parameters needed to decompress `.af` keyframe data.
