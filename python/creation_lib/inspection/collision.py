from __future__ import annotations

from collections import Counter

from creation_lib.nif.native_runtime import nif_from_bytes_raw


def _members(obj):
    result = {}
    for member in obj.members:
        if hasattr(member, "value"):
            result[member.name] = member.value
        elif hasattr(member, "contents"):
            result[member.name] = list(member.contents)
    return result


def _body(obj):
    values = _members(obj)
    keys = ("motionId", "motionType", "motionPropertiesId", "reservedMotionId", "qualityId", "materialId",
            "collisionFilterInfo", "flags", "mass", "inverseMass", "position", "orientation")
    row = {key: values[key] for key in keys if key in values}
    if isinstance(row.get("collisionFilterInfo"), int):
        row["collisionLayer"] = row["collisionFilterInfo"] & 0xFF
    return row


def collision_report(data):
    nif = nif_from_bytes_raw(data)
    rows, issues = [], []
    for block in nif.get("blocks", []):
        name = block["type_name"]
        if not name.startswith("bhk"):
            continue
        fields = block["fields"]
        row = {"block_id": block["block_id"], "type": name}
        for field in ("Target", "Body", "Flags", "Shape", "Havok Filter", "Havok Filter Copy", "Motion System", "Mass", "Layer", "Material"):
            if field in fields:
                row[field] = fields[field]
        if name in {"bhkPhysicsSystem", "bhkRagdollSystem"}:
            binary = fields.get("Binary Data") or {}
            blob = bytes(binary.get("Data") or [])
            row["blob_bytes"] = len(blob)
            try:
                from creation_lib.havok.native_runtime import load_native_module
                native = load_native_module()
                if native is None:
                    raise RuntimeError("Havok native backend unavailable")
                hkx, _registry = native.load_hkx_bytes(blob)
                objects = list(hkx.objects)
                row["classes"] = dict(sorted(Counter(obj.class_name for obj in objects).items()))
                row["shape_classes"] = sorted({obj.class_name for obj in objects if "Shape" in obj.class_name})
                systems = []
                for obj in objects:
                    if obj.class_name in {"hknpPhysicsSystemData", "hknpRagdollData"}:
                        values = _members(obj)
                        systems.append({"class": obj.class_name,
                                        "bodies": [_body(body) for body in values.get("bodyCinfos", []) if hasattr(body, "members")],
                                        "motions": [_body(motion) for motion in values.get("motionCinfos", []) if hasattr(motion, "members")],
                                        "materials": len(values.get("materials", [])),
                                        "constraints": len(values.get("constraintCinfos", []))})
                row["systems"] = systems
            except (RuntimeError, ValueError) as error:
                row["parse_error"] = str(error)
                issues.append({"block_id": block["block_id"], "message": str(error)})
        rows.append(row)
    return {"has_collision": bool(rows), "collision_block_count": len(rows), "blocks": rows,
            "issues": issues, "complete": not issues}
