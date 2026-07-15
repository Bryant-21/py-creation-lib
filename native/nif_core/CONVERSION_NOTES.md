# NIF Conversion Roadmap Notes

These notes were migrated from the removed Python NIF conversion stubs.
`nif_core_native` owns NIF conversion now; do not recreate Python conversion
pipelines for these routes. Use this file as the future-work map when adding
native conversion support.

## FO4 -> FO4

Former same-game normalization sequence:

- Normalize shader material paths.
- Rewire orphan `BSShaderTextureSet` blocks.
- Normalize texture paths.

The native converter currently copies same-game NIFs unless a native rewrite is
needed, such as addon-node index remapping.

## FO4 -> FNV / FO3

FO4 -> FNV and FO4 -> FO3 share the target NIF layout:

- BS version 34.
- `NiTriStrips` plus `TallGrassShaderProperty` or `BSShaderPPLightingProperty`.
- `bhkRigidBody` plus `bhkConvexVerticesShape`.
- Havok scale 1.0.
- `Fallout3Layer` collision enum.

FNV adds richer attachment-point conventions over FO3, but the base layout is
identical.

Required native work:

- Convert `BSTriShape` to `NiTriStrips` plus `NiTriStripsData`.
- Convert `BSLightingShaderProperty` plus `BSShaderTextureSet` to
  `TallGrassShaderProperty` or `BSShaderPPLightingProperty` plus sibling
  `NiMaterialProperty` and sibling texture set.
- Convert `bhkNPCollisionObject` plus Havok 2014 packfile to a legacy
  `bhkRigidBody` plus `bhkConvexVerticesShape` chain.
- Adjust Havok scale from 69.99125 to 1.0.
- Convert `BSConnectPoint::Parents` / `BSConnectPoint::Children` to
  `NiStringExtraData` `"Prn"` plus `##`-prefixed nodes.
- Retarget header from BS version 130 / user version 12 to BS version 34 /
  user version 11.
- Translate shader flag bitfields to FO3/FNV enum-name lists, inverse of the
  FNV/FO3 -> FO4 legacy shader path.

## FO4 -> FO76

Required native work:

- Convert FO4 Havok packfile blobs to FO76 TAG0 blobs.
- Re-nest `BSEffectShaderProperty` fields into FO76 `Shader Property Data`,
  inverse of FO76 effect-shader flattening.
- Re-nest `BSLightingShaderProperty` fields into FO76 `Shader Property Data`,
  inverse of FO76 lighting-shader flattening.
- Translate FO4 `Shader Flags 1` / `Shader Flags 2` bitfields to FO76 `SF1` /
  `SF2` CRC arrays.
- Expand `BSShaderTextureSet` slots from FO4's 10 slots to the FO76 slot layout.
- Keep Havok scale at 69.99125, but verify each blob type because FO76 and FO4
  share the nominal scale while formats differ.
- Retarget header from BS version 130 to the FO76 baseline.
- Reverse-map float-controller enums, inverse of the FO76 -> FO4 controller fix.

## FO4 -> Skyrim SE

Required native work:

- Convert FO4 uncompressed `BSTriShape` vertex data to Skyrim SE compressed
  `BSTriShape` vertex format, including half-float fields.
- Convert FO4 shader type enum values to Skyrim SE shader type enum values.
- Convert `Fallout4Layer` collision layers to `SkyrimLayer`.
- Drop FO4-specific `BSConnectPoint` data; Skyrim has no equivalent.
- Remap `BSDismemberSkinInstance` partition flags from `Fallout4BodyPart` to
  `SkyrimBodyPart`.

## FO4 -> Starfield

Required native work:

- Convert `BSTriShape` to `BSGeometry`.
- Convert `BSLightingShaderProperty` material references to `BSLayeredMaterial`
  and handle the `.bgsm` -> `.mat` material side.
- Convert FO4 Havok packfiles to Havok 2019 TAG0.
- Adjust Havok scale from 69.99125 to 1.0.
- Remap `BSShaderTextureSet` slots from FO4's 10-slot layout to the Starfield
  layout.
- Convert FO4 `BSSkin::Instance` data to Starfield `BSGeometry`-embedded skin
  data.

## FO76 -> FO4

Native route exists. The old Python sequence was:

- Fix FO76 float-controller enum values.
- Flatten `BSEffectShaderProperty` `Shader Property Data`.
- Flatten `BSLightingShaderProperty` `Shader Property Data`.
- Normalize shader material paths.
- Inject inline shader cubemaps.
- Rewire orphan texture sets.
- Remap FO76 texture slots to FO4 texture slots.
- Adjust texture set slot count.
- Normalize texture paths.
- Rename texture suffixes.
- Upgrade or convert embedded Havok blobs.
- Adjust Havok scale.
- Remap collision layers.

When maintaining the native route, use this as the parity checklist.

## FO76 -> FNV

This route was planned as an FO4 pivot:

- Complete FO4 -> FNV native NIF conversion first.
- Then route FO76 -> FO4 -> FNV, preserving any FO76-specific preprocessing
  before the FO4 -> FNV leg.

## Starfield -> FO4

Required native work:

- Convert `BSGeometry` to `BSTriShape`.
- Convert `BSLayeredMaterial` to `BSLightingShaderProperty` and handle the
  `.mat` -> `.bgsm` material side.
- Convert Starfield Havok 2019 TAG0 blobs to FO4 Havok 2014 packfiles.
- Adjust Havok scale from 1.0 to 69.99125.
- Remap `BSShaderTextureSet` slots from Starfield layout to FO4's 10-slot
  layout.
- Convert Starfield `BSGeometry`-embedded skin data to FO4 `BSSkin::Instance`.

## Deleted Python Audit Notes

These notes preserve the developer guidance from the deleted Python
`creation_lib.nif.convert`, `creation_lib.nif.conversions`, and
`creation_lib.nif.transforms` files. They are not a request to recreate the
Python pipeline. Native code should use them as parity notes and future-work
checklists.

### Former Dispatcher Contract

The deleted Python dispatcher used a `(source_game_id, target_game_id)` pipeline
registry. Each converter took `(nif, source, target, report)` and ran transforms
in order. The registered routes were:

- `fo4 -> fo4`
- `fo4 -> fo3`
- `fo4 -> fo76`
- `fo4 -> fnv`
- `fo4 -> skyrimse`
- `fo4 -> starfield`
- `fnv -> fo4`
- `fo3 -> fo4`
- `fo76 -> fo4`
- `fo76 -> fnv`
- `skyrimse -> fo4`
- `starfield -> fo4`

The old `ConversionReport.info(...)` intentionally appended to `changes`, not a
separate channel. FO76 float-controller cleanup depended on this so partial
conversions did not crash on an `AttributeError`.

The old `convert_header` pass had two non-obvious responsibilities:

- It injected target-version fields missing from the source before write. This
  prevents binary layout misalignment, such as FO76 -> FO4 missing `Shader Type`
  shifting the `NiObjectNET` layout and making NifSkope read `Controller` as an
  array size.
- It injected fields in two passes: version-conditional fields with no data
  condition first, then data-conditional fields whose conditions became true
  after the first pass. This matters for fields such as `Backlight Power` that
  depend on an injected `Rimlight Power` default.

The old `patch_addon_node_indices` behavior is now native work: scan
`BSValueNode` blocks named `AddOnNode<N>`, update `Value`, and rename the block
to `AddOnNode<new_index>` when `N` is present in the caller-provided map.

### Former Route Ordering

FNV/FO3 -> FO4 used this ordering:

1. Geometry first: convert `NiTriStrips` to `BSTriShape`.
2. Shaders: convert `TallGrassShaderProperty` and `BSShaderPPLightingProperty`
   to `BSLightingShaderProperty` plus texture sets.
3. Collision: strip legacy `bhk*` trees and rebuild through the FO4 collision
   generator.
4. Roots: FO4 exports should start at `NiNode`, not `BSFadeNode`.
5. Attachment points: planned `legacy_attach_to_connect_points` pass.
6. Generic post-passes: normalize shader paths, rewire orphan texture sets,
   resize texture sets, normalize texture paths, and rename texture suffixes.
7. Physics post-passes: Havok scale adjustment and collision enum reporting.

FO76 -> FO4 used this ordering:

1. Fix FO76 shader float-controller enum values.
2. Flatten FO76 effect shader nested data.
3. Flatten FO76 lighting shader nested data.
4. Normalize shader material paths.
5. Inject inline shader cubemaps.
6. Rewire orphan texture sets.
7. Remap FO76 texture slots.
8. Resize texture sets to FO4 slot count.
9. Normalize texture paths.
10. Rename texture suffixes.
11. Upgrade embedded Havok blobs from FO76 TAG0 to FO4 packfiles.
12. Adjust Havok scale.
13. Report collision layer and physics material enum transitions.

### Geometry Transform Notes

`strips_to_tri_shape` converted every legacy `NiTriStrips` block to `BSTriShape`.
Native parity notes:

- Decode strip indices with alternating winding and skip degenerate triangles.
- Accept both nested strip-point arrays and flat `Points` plus `Strip Lengths`.
- Build FO4 packed vertex data from legacy vertices, normals, tangents,
  bitangents, first UV set, and optional vertex colors.
- Use `Vertex Desc` flags for vertex, UVs, normals, tangents, and optional
  vertex colors. The old Python packed stride was 5 without colors and 6 with
  colors; color offset was 5 when present.
- Preserve name, controller, transform, scale, collision object, bounding
  sphere, shader property, and alpha property where possible.
- Treat `TallGrassShaderProperty`, `BSShaderPPLightingProperty`,
  `BSLightingShaderProperty`, `BSEffectShaderProperty`, and
  `Lighting30ShaderProperty` as shader-property candidates.
- Remove `NiStencilProperty` with the old strip/data blocks.
- Rewrite parent `Children` references from the old strip block to the new
  `BSTriShape`.

`tri_shape_to_strips` was a stub. Required native work:

- Decode the `BSTriShape` packed vertex stream.
- Rebuild `NiTriStripsData` with strip-encoded indices via stripify.
- Reattach the result in the legacy sibling property layout under the parent
  `NiNode`.

`bs_geometry_to_tri_shape` was a stub. Required native work:

- Read the Starfield `BSGeometry` mesh data section.
- Extract vertices, normals, UVs, and triangles.
- Build FO4 `BSTriShape` data and remap Starfield shader bindings.

`tri_shape_to_bs_geometry` was a stub. Required native work:

- Read the FO4 `BSTriShape` packed vertex stream.
- Convert to Starfield `BSGeometry` separate-stream layout and LOD chain.
- Produce a `BSLayeredMaterial` reference.

### Shader And Material Notes

`legacy_shader_to_lighting` converted legacy FNV/FO3 shader properties to FO4
lighting shaders:

- `TallGrassShaderProperty` became `BSLightingShaderProperty(Default)` with a
  10-slot `BSShaderTextureSet`; slot 0 came from `File Name`.
- `BSShaderPPLightingProperty` became `BSLightingShaderProperty` and preserved
  texture clamp mode plus refraction strength.
- Direct following `NiMaterialProperty` blocks were paired with the preceding
  `BSShaderPPLightingProperty`. If a direct material was missing, the old code
  reused a material by shader signature.
- `NiMaterialProperty` fields mapped as:
  - `Emissive Color` -> `Emissive Color`
  - `Specular Color` -> `Specular Color`
  - `Alpha` -> `Alpha`
  - `Glossiness / 100.0` -> `Smoothness`
  - `Emissive Mult` -> `Emissive Multiple`
- BGSM emission used texture slot 0 as the output path seed and wrote one BGSM
  per converted lighting shader when the target material format was `bgsm`.
- BGSM material fields preserved smoothness, emissive color and multiplier,
  alpha, diffuse, normal, emissive, greyscale, envmap, smooth/spec texture, and
  flags such as cast shadows, model-space normals, external emittance, and
  glow map.

Legacy shader flag name mapping used these FO4 bit positions:

```text
Shader Flags 1:
Specular=0, Skinned=1, Vertex_Alpha=3,
GreyscaleToPalette_Color=4, GreyscaleToPalette_Alpha=5,
Use_Falloff=6, Environment_Mapping=7, RGB_Falloff=8,
Cast_Shadows=9, Face=10, Model_Space_Normals=12,
Refraction=15, Hair=18, Skin_Tint=21, Own_Emit=22,
Decal=26, Dynamic_Decal=27, External_Emittance=29,
Soft_Effect=30, ZBuffer_Test=31.

Shader Flags 2:
ZBuffer_Write=0, LOD_Objects=2, No_Fade=3, Double_Sided=4,
Vertex_Colors=5, Glow_Map=6, Transform_Changed=7.
```

`lighting_to_legacy_shader` was the inverse stub. Required native work:

- Convert `Tall_Grass` lighting blocks to `TallGrassShaderProperty`.
- Convert default lighting blocks to `BSShaderPPLightingProperty` plus
  `NiMaterialProperty`.
- Translate FO4 shader flag bitfields back to FO3 enum-name lists.

`flatten_fo76_effect_shader` unpacked FO76
`BSEffectShaderProperty.Shader Property Data` into FO4 top-level fields:

- Copy UV offset/scale, source texture, clamp mode, lighting influence,
  env-map min LOD, unused byte, falloff angles/opacities, base color and scale,
  soft falloff depth, greyscale/env/normal/env-mask textures, and environment
  map scale.
- FO76-only fields with no FO4 equivalent were dropped silently:
  refraction power, reflectance texture, lighting texture, emittance color,
  emit gradient texture, and luminance.
- Always write `Shader Flags 1` and `Shader Flags 2`, even as zero, after CRC
  translation so injected schema defaults do not leak through.

`flatten_fo76_lighting_shader` unpacked FO76
`BSLightingShaderProperty.Shader Property Data` into FO4 top-level fields:

- Copy shader type, UVs, texture set, emissive fields, root material, clamp
  mode, alpha, refraction strength, smoothness, specular fields, grayscale
  scale, fresnel, wetness, and shader-type conditional fields.
- Renumber the shader type from FO76 `BSShaderType155` to FO4 numbering:
  FO76 has no Parallax slot, so Face/Skin/Hair Tint sit at 3/4/5 (FO4 4/5/6)
  and Eye Envmap at 12 (FO4 16). FO76's Color4 `Skin Tint Color` splits into
  FO4's Color3 + `Skin Tint Alpha`; without the renumber the writer drops the
  type-5-conditional tint and facegen rear-head/neck shapes render untinted.
- Shader-type conditional fields were intentionally copied even if not
  applicable to the current shader type because the writer is version and
  condition aware.
- FO76-only subfields were dropped silently: translucency fields, texture array
  fields, and luminance.
- Detect the FO76 PBR CRC before flag translation consumes `SF1`/`SF2`.
- PBR scalar reweighting used shared `pbr_to_specgloss` math. The old Python
  treated FO76 `Smoothness` as `1 - roughness`, `Specular Strength` as the
  metallic-roughness F0 term, diffuse as neutral gray, and used explicit
  `None` checks so valid `0.0` values were not replaced with defaults.

FO76 CRC flag translation notes:

```text
3744563888 -> SF1 bit 1  SKINNED -> Skinned
2333069810 -> SF1 bit 3  VERTEX_ALPHA -> Vertex_Alpha
442246519  -> SF1 bit 4  GRAYSCALE_TO_PALETTE_COLOR -> GreyscaleToPalette_Color
2901038324 -> SF1 bit 5  GRAYSCALE_TO_PALETTE_ALPHA -> GreyscaleToPalette_Alpha
3980660124 -> SF1 bit 6  FALLOFF -> Use_Falloff
2893749418 -> SF1 bit 7  ENVMAP -> Environment_Mapping
3448946507 -> SF1 bit 8  RGB_FALLOFF -> RGB_Falloff
1563274220 -> SF1 bit 9  CAST_SHADOWS -> Cast_Shadows
314919375  -> SF1 bit 10 FACE -> Face
2548465567 -> SF1 bit 12 MODELSPACENORMALS -> Model_Space_Normals
1957349758 -> SF1 bit 15 REFRACTION -> Refraction
1264105798 -> SF1 bit 18 HAIRTINT -> Hair
1483897208 -> SF1 bit 21 SKIN_TINT -> Skin_Tint
2262553490 -> SF1 bit 22 EMIT_ENABLED -> Own_Emit
3849131744 -> SF1 bit 26 DECAL -> Decal
1576614759 -> SF1 bit 27 DYNAMIC_DECAL -> Dynamic_Decal
2150459555 -> SF1 bit 29 EXTERNAL_EMITTANCE -> External_Emittance
3503164976 -> SF1 bit 30 SOFT_EFFECT -> Soft_Effect
1740048692 -> SF1 bit 31 ZBUFFER_TEST -> ZBuffer_Test
3166356979 -> SF2 bit 0  ZBUFFER_WRITE -> ZBuffer_Write
2896726515 -> SF2 bit 2  LOD_OBJECTS -> LOD_Objects
2994043788 -> SF2 bit 3  NOFADE -> No_Fade
759557230  -> SF2 bit 4  TWO_SIDED -> Double_Sided
348504749  -> SF2 bit 5  VERTEXCOLORS -> Vertex_Colors
2399422528 -> SF2 bit 6  GLOWMAP -> Glow_Map
3196772338 -> SF2 bit 7  TRANSFORM_CHANGED -> Transform_Changed
2078326675 -> SF2 bit 17 WEAPON_BLOOD -> Weapon_Blood
3473438218 -> SF2 bit 30 EFFECT_LIGHTING -> Effect_Lighting
```

FO76-only CRCs intentionally had no FO4 flag: `731263983` PBR,
`902349195` REFRACTION_FALLOFF, `3030867718` INVERTED_FADE_PATTERN, and
`3707406987` NO_EXPOSURE. The PBR CRC is still semantically important because
it drives the scalar reweight described above.

`inject_inline_shader_cubemaps` notes:

- Only runs for FO4 targets.
- Skip `BSEffectShaderProperty` blocks with a non-empty `Name`, because a BGEM
  reference owns the cubemap.
- Skip `BSLightingShaderProperty` blocks with a non-empty `Name`, because a
  BGSM reference owns the cubemap.
- Effect shaders use `Env Map Texture` directly and set environment map scale
  if the existing value is missing or zero.
- Lighting shaders store the cubemap in `BSShaderTextureSet.Textures[4]`;
  pad to at least 10 slots before writing.
- FO4 environment mapping is `Shader Flags 1` bit 7.
- The cubemap heuristic source path is the NIF directory, stripped to a
  Data-relative `meshes/...` path and translated to `materials/...`.
- Skip actor character assets in `meshes/actors/.../characterassets`.
- Lighting shader proxy slot map was FO4 slot 0 diffuse, slot 1 normal, slot 7
  specular/back lighting. Shader type 4 meant skin tint and 5 meant hair tint.

### Texture Path And Slot Notes

`convert_texture_sets` contained four separate passes:

- Resize texture sets only. Target count was 15 for FO76 and 10 for other
  targets.
- Remap FO76 texture slots only for `fo76 -> fo4`.
- Normalize texture paths to backslashes with a leading `textures\` prefix.
- Rename texture filenames by detected texture role through
  `creation_lib.textures.naming`.

FO76 slot remap behavior:

- Slot 9, reflectivity/specular, moved to FO4 slot 7 if that slot was empty.
- Slot 10, lighting, moved to FO4 glow slot 2 only for emissive shader sets.
- Emissive detection accepted `Own_Emit`, `External_Emittance`, `Glow_Map`, or
  a positive emissive color with missing or positive emissive multiplier.

`normalize_shader_paths` stripped absolute and build-server prefixes from
shader material references. Recognized Data-relative roots were `materials`,
`textures`, `meshes`, `sound`, `music`, and `interface`. Preserve original slash
style when the source used only backslashes.

`rewire_orphan_texture_sets` attached orphan `BSShaderTextureSet` blocks to the
nearest preceding unlinked `BSLightingShaderProperty`. This was generic and was
especially useful for FO76 meshes where `Texture Set == -1` because the BGSM
carried textures; the explicit texture set gives FO4 a fallback if the BGSM
does not load.

### Havok And Collision Notes

`adjust_havok_scale` used this invariant:

- `src_scale` means `nif_coords = havok_coords * src_scale`.
- Convert with `new_havok = old_havok * (src_scale / tgt_scale)`.
- FO4 uses 69.99125. Legacy games use 1:1 scale.
- Source scaled and target unscaled means multiply by source scale.
- Source unscaled and target scaled means divide by target scale.

`upgrade_havok_blob` converted FO76 embedded Havok:

- Active only for `fo76 -> fo4`.
- Target block types were `bhkPhysicsSystem` and `bhkRagdollSystem`.
- Source data lived at `Binary Data.Data` and was FO76 TAG0/hk_2015.
- Target data was FO4 hk_2014 packfile bytes.
- Converted bytes had to be written back as `list[int]` with `Data Size` set
  to the converted length so the serializer round-tripped.
- On conversion failure, leave the original FO76 data in place and warn that
  collision will crash at runtime.

`downgrade_havok_blob` was a stub. Required native work:

- Wrap `havok_native.havok_convert_bytes` with target `fo76`.
- Apply it to `bhkPhysicsSystem` and `bhkRagdollSystem` binary data.

`regenerate_fo4_collision` stripped legacy `bhkCollisionObject` chains and
rebuilt FO4 collision from `BSTriShape` geometry:

- Candidate parent node types were `NiNode`, `BSFadeNode`, `BSLeafAnimNode`,
  `BSOrderedNode`, and `NiBillboardNode`.
- Metadata was taken from the legacy body: layer from `Havok Filter.Layer`,
  plus mass, friction, and restitution from `Rigid Body Info`.
- Defaults were layer `STATIC`, mass `0.0`, friction `0.5`, restitution `0.4`.
- Legacy Fallout layer values mapped as:
  `1 STATIC`, `2 ANIMSTATIC`, `3 TRANSPARENT`, `4 CLUTTER`, `5 WEAPON`,
  `6 PROJECTILE`, `7 NPC`, `13 TERRAIN`, `14 BIPED`, `15 TREES`,
  `17 DEADBIP`, `30 CHARCONTROLLER`.
- Clear the parent node's `Collision Object` ref, remove the collected
  collision subtree, rediscover the parent by name and type after removal,
  collect transformed child geometry, and call the FO4 collision generator with
  `replace=True`.

`regenerate_legacy_collision` was a stub. Required native work:

- Strip `bhkNPCollisionObject` and `bhkPhysicsSystem`.
- Capture a vertex AABB from `BSTriShape`.
- Rebuild `bhkConvexVerticesShape`.
- Wrap it in `bhkRigidBody` and `bhkCollisionObject` with `Fallout3Layer`
  mapping.

`regenerate_starfield_collision` was a stub. Required native work:

- Strip FO4 `bhkNP*` blocks and packfile payloads.
- Build Starfield Havok 2019 TAG0 `hknpPhysicsSystemData`.
- Reuse the Starfield collision path from operations/collision if available.

`remap_collision_layers` was a reporting pass, not a numeric conversion:

- Collision layer values are often numerically identical across games; the enum
  type name changes with schema version.
- Preserve numeric values and report the source and target collision enum names.
- Do the same for physics material enum names on known shape block types.

### Skinning And Dismember Notes

`legacy_skin_to_fo4_skin` is now implemented in the native `skin/` module.
Python threads `translation_maps_dir`, `auto_skin_reference_body`,
`emit_first_person`, `first_person_reference`, and `morph_weight_cap` through
`ConversionOrchestrator` -> `_convert_single_nif` ->
`creation_lib.nif.native_runtime.convert_nif_file_raw`. Use the notes below as
parity and maintenance guidance for that native route.

Legacy source layout:

```text
NiTriStrips or NiTriShape
  -> NiSkinInstance or BSDismemberSkinInstance
     -> Skeleton Root
     -> Bones
     -> NiSkinData
        -> Skin Transform
        -> Bone List with per-bone bounds and vertex weights
        -> Has Vertex Weights
     -> NiSkinPartition
        -> partition blocks with triangles/strips, vertex map, weights,
           and bone indices
```

FO4 non-armor target layout:

```text
BSTriShape with VF_VERTEX | VF_UVS | VF_NORMALS | VF_TANGENTS |
  VF_SKINNED | VF_FULLPRECISION
  -> BSSkin::Instance
     -> Skeleton Root
     -> NiSkinPartition, FO4 flavored
     -> BSSkin::BoneData with per-bone bounds only
     -> Bones
```

FO4 armor target layout:

```text
BSTriShape
  -> BSDismemberSkinInstance, FO4 form_version
     -> NiSkinInstance fields, version conditional
     -> Partitions using Fallout4BodyPart values
     -> BSSkin::BoneData, not NiSkinData
```

Required native work:

- Detect armor vs non-armor. A source `BSDismemberSkinInstance` preserves
  dismember semantics; plain `NiSkinInstance` can become `BSSkin::Instance`.
- Fold legacy per-partition weights back into the global vertex array, dedupe,
  and pack up to four weights and bone indices into the FO4 vertex stream.
- Convert `NiSkinData` per-bone bounds to `BSSkin::BoneData`.
- Remap dismember partition flags.
- Remap legacy bone names such as `Bip01_*` to FO4 skeleton names such as
  `Pelvis` and `Spine1`. This likely needs per-source-skeleton tables and may
  require a user-provided rig.
- Recompute tangent space for legacy skinned meshes that lack tangents.
- Recompute the new `BSTriShape` bounding sphere.
- Run after `strips_to_tri_shape` and before FO4 collision regeneration.
- Good fixtures would be a small FNV NPC outfit NIF and a small FO4 armor
  reference NIF.

`fo4_skin_to_legacy_skin` was the inverse stub:

- Detect FO4 armor vs non-armor. `BSDismemberSkinInstance` with FO4 form
  version demotes to the target form; plain `BSSkin::Instance` promotes to
  legacy `NiSkinInstance`.
- Group vertices by dominant bone, build legacy partitions, and emit
  `NiSkinPartition` vertex maps and weight tables.
- Convert `BSSkin::BoneData` back to `NiSkinData`.
- Remap FO4 bone names back to legacy target names.
- Preserve existing tangent space.
- Run after `tri_shape_to_strips` and before legacy collision regeneration.

`skin_data_to_bs_skin_bone_data` notes:

- Legacy `NiSkinData` stores skin transform, per-bone transform, bounding
  sphere, vertex weights, and `Has Vertex Weights`.
- FO4 `BSSkin::BoneData` stores per-bone bounding spheres only.
- Apply `NiSkinData.Skin Transform` to `BSTriShape` vertices before vertex
  packing; FO4 assumes vertices are already in skin space.
- Vertex weights are not this transform's job. They belong to
  `legacy_skin_to_fo4_skin`.

`bs_skin_bone_data_to_skin_data` notes:

- Copy per-bone bounds as-is.
- Reconstruct `Skin Transform` from the parent shape transform chain because
  `BSSkin` does not store it explicitly.
- Leave vertex weight migration to `fo4_skin_to_legacy_skin`.

`dismember_partition_remap` was a policy stub:

- Build remap tables from `nif.xml` for `Fallout3BodyPart`, `SkyrimBodyPart`,
  and `Fallout4BodyPart`.
- Lossy cases need an explicit policy enum:
  `DROP_ON_LOSS`, `MERGE_TO_PARENT`, or `FAIL_ON_LOSS`.
- Known lossy cases:
  - FO3 single `UPPER_BODY` -> FO4 split `CHEST + ARMS`. Without geometry
    hints, fallback was to drop `ARMS` and assign to `CHEST`.
  - FO4 `NECK` has no FNV/FO3 equivalent. Policy decision is drop or merge
    into `HEAD`.
  - Skyrim `FOREARMS` -> FO4 `ARMS` is 1:1.
- Test fixtures needed: a Skyrim armor NIF and a FO3 NPC outfit NIF.

### Attachment And Root Notes

`legacy_attach_to_connect_points` was a stub:

- Walk `NiStringExtraData` blocks for `Prn` hints.
- Collect `##`-prefixed `NiNode` names.
- Map legacy names to FO4 `P-*` names.
- Emit `BSConnectPoint::Parents` and `BSConnectPoint::Children` blocks.

`connect_points_to_legacy_attach` was the inverse stub:

- Walk `BSConnectPoint::Parents` and `BSConnectPoint::Children`.
- Map FO4 `P-*` names back to `##`-prefixed `NiNode` names and `Prn` hints.
- Drop attachments with no legacy equivalent.

`normalize_fo4_root_node` converted footer-root `BSFadeNode` blocks to `NiNode`
for FO4 exports and reported failures from the block-type conversion operation.

### Deleted Test Intent

The deleted Python tests carried these expectations that should be mirrored by
native tests when those features move:

- Dispatcher coverage required every unsupported route/stub to report either
  `Required transforms:` or `Required work:` and kept the FO76 -> FNV dependency
  on completing FO4 -> FNV first.
- FNV -> FO4 had real-fixture end-to-end tests.
- Inline shader cubemap tests covered both effect and lighting shader branches,
  named BGSM/BGEM references being skipped, non-FO4 targets being no-op, decal
  paths that return no cubemap, FO4 slot 4 population, and environment-mapping
  flag bit 7.
- Transform tests existed for texture slot/path conversion, legacy shader to
  lighting shader, BGSM emission, FO4 collision regeneration, and
  `NiTriStrips` to `BSTriShape`.

### Audit Coverage

The notes above were audited from the staged-deleted Python conversion files,
transform files, and their tests under:

- `py_creation_lib/python/creation_lib/nif/convert.py`
- `py_creation_lib/python/creation_lib/nif/convert_header.py`
- `py_creation_lib/python/creation_lib/nif/conversions/*.py`
- `py_creation_lib/python/creation_lib/nif/conversions/tests/*.py`
- `py_creation_lib/python/creation_lib/nif/transforms/*.py`
- `py_creation_lib/python/creation_lib/nif/transforms/tests/*.py`
- `py_creation_lib/python/creation_lib/nif/tests/test_inline_shader_envmap.py`

Exact deleted conversion files covered:

- `fnv_to_fo4.py`
- `fo4_to_fo4.py`
- `fo4_to_fnv.py`
- `fo4_to_fo76.py`
- `fo4_to_skyrimse.py`
- `fo4_to_starfield.py`
- `fo76_to_fo4.py`
- `fo76_to_fnv.py`
- `skyrimse_to_fo4.py`
- `starfield_to_fo4.py`

Exact deleted transform files covered:

- `adjust_havok_scale.py`
- `bs_geometry_to_tri_shape.py`
- `bs_skin_bone_data_to_skin_data.py`
- `connect_points_to_legacy_attach.py`
- `convert_texture_sets.py`
- `dismember_partition_remap.py`
- `downgrade_havok_blob.py`
- `fix_fo76_float_controllers.py`
- `flatten_fo76_effect_shader.py`
- `flatten_fo76_lighting_shader.py`
- `fo4_skin_to_legacy_skin.py`
- `inject_inline_shader_cubemaps.py`
- `legacy_attach_to_connect_points.py`
- `legacy_shader_to_lighting.py`
- `legacy_skin_to_fo4_skin.py`
- `lighting_to_legacy_shader.py`
- `normalize_fo4_root_node.py`
- `normalize_shader_paths.py`
- `regenerate_fo4_collision.py`
- `regenerate_legacy_collision.py`
- `regenerate_starfield_collision.py`
- `remap_collision_layers.py`
- `rewire_orphan_texture_sets.py`
- `skin_data_to_bs_skin_bone_data.py`
- `strips_to_tri_shape.py`
- `translate_fo76_crc_flags.py`
- `tri_shape_to_bs_geometry.py`
- `tri_shape_to_strips.py`
- `upgrade_havok_blob.py`
