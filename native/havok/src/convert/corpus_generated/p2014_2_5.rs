// Native Havok conversion corpus. Hand-maintained.

use super::super::manager::PatchManager;
use super::super::ops::{ClassVersion, Patch, PatchOperation, PatchValue};

pub(super) fn register(manager: &mut PatchManager) {
    // version_id = 55
    // common.py
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkaSkeleton", 5),
            ClassVersion::new("hkaSkeleton", 6),
        )
        .with_custom_hook("_noop_type_change")
        .with_operation(PatchOperation::Depends {
            class_name: "hkQsTransform".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkaSkeletonPartition", 1),
            ClassVersion::new("hkaSkeleton::Partition", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkaSkeletonLocalFrameOnBone", 0),
            ClassVersion::new("hkaSkeleton::LocalFrameOnBone", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkSweptTransformf", 0),
            ClassVersion::new("hkSweptTransform", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkPackfileHeader", 2),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "magic".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "userTag".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "fileVersion".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "layoutRules".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "numSections".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "contentsSectionIndex".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "contentsSectionOffset".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "contentsClassNameSectionIndex".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "contentsClassNameSectionOffset".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "contentsVersion".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "flags".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "maxpredicate".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "predicateArraySizePlusPadding".to_string(),
            type_name: "int".to_string(),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkPackfileSectionHeader", 1),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "sectionTag".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "nullByte".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "absoluteDataStart".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "localFixupsOffset".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "globalFixupsOffset".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "virtualFixupsOffset".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "exportsOffset".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "importsOffset".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "endOffset".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "pad".to_string(),
            type_name: "int".to_string(),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkDataObjectTypeAttribute", 0),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "typeName".to_string(),
            type_name: "string".to_string(),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkMonitorStreamStringMap", 0),
            ClassVersion::new("hkMonitorStreamStringMap", 1),
        )
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkRefCountedProperties", 1),
            ClassVersion::new("hkRefCountedProperties", 2),
        )
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkRefCountedProperties::Entry".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkClassEnum", 0),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "name".to_string(),
            type_name: "string".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "items".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "flags".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkClassEnumItem".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkClassEnumItem", 0),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "value".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "name".to_string(),
            type_name: "string".to_string(),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkRefCountedPropertiesEntry", 0),
            ClassVersion::new("hkRefCountedProperties::Entry", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkGeometryTriangle", 0),
            ClassVersion::new("hkGeometry::Triangle", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkMemoryResourceHandleExternalLink", 1),
            ClassVersion::new("hkMemoryResourceHandle::ExternalLink", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkMonitorStreamColorTableColorPair", 0),
            ClassVersion::new("hkMonitorStreamColorTable::ColorPair", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkMonitorStreamStringMapStringMap", 0),
            ClassVersion::new("hkMonitorStreamStringMap::StringMap", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkxSplineControlPoint", 0),
            ClassVersion::new("hkxSpline::ControlPoint", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkxVertexDescriptionElementDecl", 4),
            ClassVersion::new("hkxVertexDescription::ElementDecl", 4),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkxVertexBufferVertexData", 2),
            ClassVersion::new("hkxVertexBuffer::VertexData", 2),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkxVertexAnimationUsageMap", 0),
            ClassVersion::new("hkxVertexAnimation::UsageMap", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkxMeshUserChannelInfo", 0),
            ClassVersion::new("hkxMesh::UserChannelInfo", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkxMaterialProperty", 0),
            ClassVersion::new("hkxMaterial::Property", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkxMaterialTextureStage", 1),
            ClassVersion::new("hkxMaterial::TextureStage", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkxNodeAnnotationData", 0),
            ClassVersion::new("hkxNode::AnnotationData", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkxEnvironmentVariable", 0),
            ClassVersion::new("hkxEnvironment::Variable", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkxEnumItem", 0),
            ClassVersion::new("hkxEnum::Item", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkRootLevelContainerNamedVariant", 1),
            ClassVersion::new("hkRootLevelContainer::NamedVariant", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkVertexFormatElement", 0),
            ClassVersion::new("hkVertexFormat::Element", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkMeshTextureRawBufferDescriptor", 0),
            ClassVersion::new("hkMeshTexture::RawBufferDescriptor", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkSkinnedMeshShapePart", 1),
            ClassVersion::new("hkSkinnedMeshShape::Part", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkSkinnedMeshShapeBoneSection", 1),
            ClassVersion::new("hkSkinnedMeshShape::BoneSection", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkSkinnedMeshShapeBoneSet", 0),
            ClassVersion::new("hkSkinnedMeshShape::BoneSet", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkMultipleVertexBufferLockedElement", 0),
            ClassVersion::new("hkMultipleVertexBuffer::LockedElement", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkMultipleVertexBufferElementInfo", 0),
            ClassVersion::new("hkMultipleVertexBuffer::ElementInfo", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkMultipleVertexBufferVertexBufferInfo", 0),
            ClassVersion::new("hkMultipleVertexBuffer::VertexBufferInfo", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkMemoryMeshShapeSection", 0),
            ClassVersion::new("hkMemoryMeshShape::Section", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkSethkIntRealPairhkContainerHeapAllocatorhkMapOperationshkIntRealPair", 0),
            ClassVersion::new("hkSet< hkIntRealPair, hkContainerHeapAllocator, hkMapOperations< hkIntRealPair > >", 0),
        )
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new(
                "hkSetunsignedinthkContainerHeapAllocatorhkMapOperationsunsignedint",
                0,
            ),
            ClassVersion::new(
                "hkSet< hkUint32, hkContainerHeapAllocator, hkMapOperations< hkUint32 > >",
                0,
            ),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new(
                "hkSetunsignedlonglonghkContainerHeapAllocatorhkMapOperationsunsignedlonglong",
                0,
            ),
            ClassVersion::new(
                "hkSet< hkUint64, hkContainerHeapAllocator, hkMapOperations< hkUint64 > >",
                0,
            ),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkBitFieldBasehkOffsetBitFieldStoragehkArrayunsignedinthkContainerHeapAllocator", 0),
            ClassVersion::new("hkBitFieldBase< hkOffsetBitFieldStorage< hkArray< hkUint32, hkContainerHeapAllocator > > >", 0),
        )
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkBitFieldBasehkBitFieldStoragehkArrayunsignedinthkContainerHeapAllocator", 0),
            ClassVersion::new("hkBitFieldBase< hkBitFieldStorage< hkArray< hkUint32, hkContainerHeapAllocator > > >", 0),
        )
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new(
                "hkBitFieldStoragehkArrayunsignedinthkContainerHeapAllocator",
                0,
            ),
            ClassVersion::new(
                "hkBitFieldStorage< hkArray< hkUint32, hkContainerHeapAllocator > >",
                0,
            ),
        ),
    );
    manager.register(
        55,
        Patch::new(ClassVersion::new("", -1), ClassVersion::new("hkResult", 0)).with_operation(
            PatchOperation::MemberAdd {
                name: "enum".to_string(),
                type_name: "int".to_string(),
                ctype: None,
                default: None,
            },
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkContainerTempAllocator", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkContainerHeapAllocator", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkContainerDebugAllocator", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkTraceStream", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "counter".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "titles".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hkTraceStream::Title".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkTraceStream::Title".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkTraceStream::Title", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "value".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkPackedVector4_6", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "values".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkVector2f", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "x".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "y".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkVector2d", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "x".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "y".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(ClassVersion::new("", -1), ClassVersion::new("hkVector2", 0))
            .with_operation(PatchOperation::ParentSet {
                old_parent: None,
                new_parent: Some("hkVector2f".to_string()),
            })
            .with_operation(PatchOperation::Depends {
                class_name: "hkVector2f".to_string(),
                version: 0,
            }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkGpuTraceResult", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "id".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "gpuTimeBegin".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "gpuTimeEnd".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "numPixelsTouched".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "type".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "threadId".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "meta".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkReferencedObject".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkSerialize::Note::Import", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "name".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkSerialize::Note::Export", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "name".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkSerialize::Detail::IdFromPointer", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "id".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(ClassVersion::new("", -1), ClassVersion::new("hkTask", 0)),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkReferencedTask", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkMeshSystem", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkCameraData", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "from".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "to".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "up".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "fovyDegrees".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "near".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "far".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "isOrthographic".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "handedness".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkCamera3d", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkDiagonalizedMassProperties", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "volume".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "mass".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "centerOfMass".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "inertiaTensor".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "majorAxisSpace".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkDefaultCompoundMeshShape", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkMeshShape".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkMeshShape".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "shapes".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hkMeshShape".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "defaultChildTransforms".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "sections".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hkDefaultCompoundMeshShape::MeshSection".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkDefaultCompoundMeshShape::MeshSection".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkDefaultCompoundMeshShape::MeshSection", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "shapeIndex".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "sectionIndex".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkDefaultCompoundMeshBody", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkMeshBody".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkMeshBody".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "bodies".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hkMeshBody".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "transform".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "shape".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkDefaultCompoundMeshShape".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkDefaultCompoundMeshShape".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "transformSet".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkIndexedTransformSet".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkIndexedTransformSet".to_string(),
            version: 2,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "transformIsDirty".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "transformSetUpdated".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkMemoryMeshSystem", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkMeshSystem".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkMeshSystem".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkAxialRotation", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "tag".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkAxialTransform", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "rotation".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkAxialRotation".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "translation".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkAxialRotation".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkQsTransformd", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "translation".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "rotation".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "scale".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkQsTransformf", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "translation".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "rotation".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "scale".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkCompressedMassProperties", 0),
            ClassVersion::new("hkCompressedMassProperties", 1),
        )
        .with_custom_hook("_noop_type_change")
        .with_operation(PatchOperation::Depends {
            class_name: "hkPackedVector3".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkcdPlanarGeometryPrimitives::Plane::Int64Vector4", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "vec".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkcdPlanarGeometryPrimitives::Plane", 0),
            ClassVersion::new("hkcdPlanarGeometryPrimitives::Plane", 1),
        )
        .with_custom_hook("_noop_type_change")
        .with_operation(PatchOperation::Depends {
            class_name: "hkcdPlanarGeometryPrimitives::Plane::Int64Vector4".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(ClassVersion::new("hkClass", 1), ClassVersion::new("", -2))
            .with_operation(PatchOperation::MemberRemove {
                name: "declaredEnums".to_string(),
                type_name: "array".to_string(),
            })
            .with_operation(PatchOperation::MemberRemove {
                name: "describedVersion".to_string(),
                type_name: "int".to_string(),
            })
            .with_operation(PatchOperation::MemberRemove {
                name: "flags".to_string(),
                type_name: "int".to_string(),
            })
            .with_operation(PatchOperation::MemberRemove {
                name: "declaredMembers".to_string(),
                type_name: "array".to_string(),
            })
            .with_operation(PatchOperation::MemberRemove {
                name: "numImplementedInterfaces".to_string(),
                type_name: "int".to_string(),
            })
            .with_operation(PatchOperation::MemberRemove {
                name: "objectSize".to_string(),
                type_name: "int".to_string(),
            })
            .with_operation(PatchOperation::MemberRemove {
                name: "parent".to_string(),
                type_name: "struct".to_string(),
            })
            .with_operation(PatchOperation::MemberRemove {
                name: "name".to_string(),
                type_name: "string".to_string(),
            })
            .with_operation(PatchOperation::Depends {
                class_name: "hkClassEnum".to_string(),
                version: 0,
            })
            .with_operation(PatchOperation::Depends {
                class_name: "hkClassMember".to_string(),
                version: 1,
            }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkClassMember", 0),
            ClassVersion::new("hkClassMember", 1),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "enum".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "class".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkClassEnum".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkClass".to_string(),
            version: 1,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkClassMember", 1),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "offset".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "flags".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "cArraySize".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "subtype".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "type".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "name".to_string(),
            type_name: "string".to_string(),
        }),
    );
    // animation.py
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkaKeyFrameHierarchyUtilityControlData", 0),
            ClassVersion::new("hkaKeyFrameHierarchyUtility::ControlData", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkaSkeletonMapperDataPartitionMappingRange", 0),
            ClassVersion::new("hkaSkeletonMapperData::PartitionMappingRange", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkaSkeletonMapperDataChainMapping", 0),
            ClassVersion::new("hkaSkeletonMapperData::ChainMapping", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkaSkeletonMapperDataSimpleMapping", 0),
            ClassVersion::new("hkaSkeletonMapperData::SimpleMapping", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkaMeshBindingMapping", 0),
            ClassVersion::new("hkaMeshBinding::Mapping", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkaAnnotationTrackAnnotation", 0),
            ClassVersion::new("hkaAnnotationTrack::Annotation", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkaSplineCompressedAnimationAnimationCompressionParams", 0),
            ClassVersion::new(
                "hkaSplineCompressedAnimation::AnimationCompressionParams",
                0,
            ),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkaSplineCompressedAnimationTrackCompressionParams", 0),
            ClassVersion::new("hkaSplineCompressedAnimation::TrackCompressionParams", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkaQuantizedAnimationTrackCompressionParams", 0),
            ClassVersion::new("hkaQuantizedAnimation::TrackCompressionParams", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkaPredictiveCompressedAnimationTrackCompressionParams", 0),
            ClassVersion::new(
                "hkaPredictiveCompressedAnimation::TrackCompressionParams",
                0,
            ),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkaSkeletonMapperData::SimpleMapping", 0),
            ClassVersion::new("hkaSkeletonMapperData::SimpleMapping", 1),
        )
        .with_custom_hook("_noop_type_change")
        .with_operation(PatchOperation::Depends {
            class_name: "hkQsTransform".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkaSkeletonMapperData::ChainMapping", 0),
            ClassVersion::new("hkaSkeletonMapperData::ChainMapping", 1),
        )
        .with_custom_hook("_noop_type_change")
        .with_operation(PatchOperation::Depends {
            class_name: "hkQsTransform".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkaSkeletonMapperData", 2),
            ClassVersion::new("hkaSkeletonMapperData", 3),
        )
        .with_custom_hook("_noop_type_change")
        .with_operation(PatchOperation::Depends {
            class_name: "hkQsTransform".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkaInterleavedUncompressedAnimation", 0),
            ClassVersion::new("hkaInterleavedUncompressedAnimation", 1),
        )
        .with_custom_hook("_noop_type_change")
        .with_operation(PatchOperation::Depends {
            class_name: "hkQsTransform".to_string(),
            version: 0,
        }),
    );
    // behavior.py
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbBehaviorReferenceGenerator", 0),
            ClassVersion::new("hkbBehaviorReferenceGenerator", 1),
        )
        .with_custom_hook("_hkbBehaviorReferenceGenerator_0_to_1"),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbCharacterStringData", 9),
            ClassVersion::new("hkbCharacterStringData", 10),
        )
        .with_custom_hook("_hkbCharacterStringData_9_to_10"),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbBehaviorGraphStringData", 1),
            ClassVersion::new("hkbBehaviorGraphStringData", 2),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "animationNames".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbClipGenerator", 4),
            ClassVersion::new("hkbClipGenerator", 5),
        )
        .with_custom_hook("_hkbClipGenerator_4_to_5")
        .with_operation(PatchOperation::MemberAdd {
            name: "animationInternalId".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(-1)),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "animationBindingIndex".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "animationBundleName".to_string(),
            type_name: "string".to_string(),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbProjectStringData", 2),
            ClassVersion::new("hkbProjectStringData", 3),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "animationFilenames".to_string(),
            type_name: "array".to_string(),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbEventDrivenBlendingObject", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "weight".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(1.0_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "fadeInDuration".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(0.0_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "fadeOutDuration".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(0.0_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "onEventId".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(-1)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "offEventId".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(-1)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "onByDefault".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "forceFullFadeDurations".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "fadeInOutCurve".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(1)),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbEventDrivenBlendingObject::InternalState", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "weight".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "timeElapsed".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "onFraction".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "onFractionOffset".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "fadingState".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbBodyIkControlPriority", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbBodyIkControlPoint", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkbBodyIkControl".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "name".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "taskInfluenceDistance".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(255)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "animationInfluenceDistance".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(3)),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbBodyIkControl".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbBodyIkControlsModifierInternalState", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "controlDataInternalStates".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hkbEventDrivenBlendingObject::InternalState".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbEventDrivenBlendingObject::InternalState".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbBodyIkControllerSetup", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "skeleton".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkaSkeleton".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "controllerCinfo".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkbBodyIkControllerCinfo".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbBodyIkControllerCinfo".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomPropertySheet".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkaSkeleton".to_string(),
            version: 5,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbBodyIkControllerCinfo", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkbCustomPropertySheet".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "profiles".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hkbBodyIkControllerProfile".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomPropertySheet".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbBodyIkControllerProfile".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbNullBodyIkInterface", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkbBodyIkInterface".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbBodyIkInterface".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbBodyIkControllerProfile", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "name".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "pins".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hkbBodyIkControlPin".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "controlPoints".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hkbBodyIkControlPoint".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "animationInfluences".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkbBoneWeightArray".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbBodyIkControlPoint".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbBindable".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbBodyIkControl".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbBodyIkControlPin".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbBoneWeightArray".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbBodyIkControlPin", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkbBodyIkControl".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbBodyIkControl".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbBodyIkTask", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "boneIdx".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "priority".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "taskInfluenceDistance".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "targetPositionMS".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "targetPositionWeight".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "targetRotationMS".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "targetRotationWeight".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "effectors".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "effectorsOffsetLS".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbBodyIkTaskList", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "tasks".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hkbBodyIkTask".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "animationInfluences".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbBodyIkTask".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbBodyIkControlBits", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbBodyIkInterface", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbBodyIkControlsModifier::ControlData", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkbEventDrivenBlendingObject".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "controlPointName".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "effectors".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(3)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "animationInfluence".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(0.25_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "targetPosition".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "targetRotation".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "targetHandle".to_string(),
            type_name: "pointer".to_string(),
            ctype: Some("hkbHandle".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "targetTransitionDuration".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(0.0_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "blendAnimationDuringTargetTransition".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(1)),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbHandle".to_string(),
            version: 2,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbEventDrivenBlendingObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbBodyIkControlsModifier", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkbModifier".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "profileName".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "controlDatas".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hkbBodyIkControlsModifier::ControlData".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "posePredictionMode".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbNode".to_string(),
            version: 1,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbModifier".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbBindable".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbBodyIkControlsModifier::ControlData".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbEventDrivenBlendingObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbBodyIkControl", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "boneIdx".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(-1)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "effectorsOffsetLS".to_string(),
            type_name: "vec4".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "effectors".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(3)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "priority".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbLayer", 1),
            ClassVersion::new("hkbLayer", 2),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "blendingControlData".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkbEventDrivenBlendingObject".to_string()),
            default: None,
        })
        .with_custom_hook("_hkbLayer_1_to_2")
        .with_operation(PatchOperation::MemberRemove {
            name: "weight".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "fadeInDuration".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "fadeOutDuration".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "onEventId".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "offEventId".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "onByDefault".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "forceFullFadeDurations".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbEventDrivenBlendingObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbLayerGeneratorInternalState", 0),
            ClassVersion::new("hkbLayerGeneratorInternalState", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "layerBlendingInternalStates".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hkbEventDrivenBlendingObject::InternalState".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbEventDrivenBlendingObject::InternalState".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbLayerGeneratorLayerInternalState", 0),
            ClassVersion::new("hkbLayerGenerator::LayerInternalState", 1),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "weight".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "timeElapsed".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "onFraction".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "fadingState".to_string(),
            type_name: "int".to_string(),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbCustomPropertySheet", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbMirroredSkeletonInfo", 1),
            ClassVersion::new("hkbMirroredSkeletonInfo", 2),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: Some("hkReferencedObject".to_string()),
            new_parent: Some("hkbCustomPropertySheet".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomPropertySheet".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbHandIkDriverInfo", 0),
            ClassVersion::new("hkbHandIkDriverInfo", 1),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: Some("hkReferencedObject".to_string()),
            new_parent: Some("hkbCustomPropertySheet".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomPropertySheet".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbFootIkDriverInfo", 1),
            ClassVersion::new("hkbFootIkDriverInfo", 2),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: Some("hkReferencedObject".to_string()),
            new_parent: Some("hkbCustomPropertySheet".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomPropertySheet".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbFootIkDriverInfo::Leg", 1),
            ClassVersion::new("hkbFootIkDriverInfo::Leg", 2),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "maxFootPitchDegrees".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(45.0_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "minFootPitchDegrees".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(-45.0_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "maxFootRollDegrees".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(20.0_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "minFootRollDegrees".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(-20.0_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "heelOffsetFromAnkle".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(0.0_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "favorToeInterpenetrationOverSteepSlope".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "favorHeelInterpenetrationOverSteepSlope".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbFootIkGains", 1),
            ClassVersion::new("hkbFootIkGains", 2),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "ankleRotationGain".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(0.2_f32)),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbCharacterData", 10),
            ClassVersion::new("hkbCharacterData", 11),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "propertySheets".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hkbCustomPropertySheet".to_string()),
            default: None,
        })
        .with_reversible_custom_hook("_hkbCharacterData_10_to_11", "_hkbCharacterData_11_to_10")
        .with_operation(PatchOperation::MemberRemove {
            name: "mirroredSkeletonInfo".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "footIkDriverInfo".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "handIkDriverInfo".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "aiControlDriverInfo".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbFootIkDriverInfo".to_string(),
            version: 1,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbHandIkDriverInfo".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbMirroredSkeletonInfo".to_string(),
            version: 1,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomPropertySheet".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbAiDriverInfo", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkbCustomPropertySheet".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomPropertySheet".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbAiInterface", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbAiDriverSetup", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "character".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkbCharacter".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "driverInfo".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkbAiDriverInfo".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCharacter".to_string(),
            version: 4,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbCustomPropertySheet".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbAiDriverInfo".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkbAiDriver", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkBaseObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbBehaviorInfoIdToNamePair", 1),
            ClassVersion::new("hkbBehaviorInfo::IdToNamePair", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkVariableTweakingHelperVector4VariableInfo", 0),
            ClassVersion::new("hkVariableTweakingHelper::Vector4VariableInfo", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkVariableTweakingHelperRealVariableInfo", 0),
            ClassVersion::new("hkVariableTweakingHelper::RealVariableInfo", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkVariableTweakingHelperIntVariableInfo", 0),
            ClassVersion::new("hkVariableTweakingHelper::IntVariableInfo", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkVariableTweakingHelperBoolVariableInfo", 0),
            ClassVersion::new("hkVariableTweakingHelper::BoolVariableInfo", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbRadialSelectorGeneratorGeneratorPair", 0),
            ClassVersion::new("hkbRadialSelectorGenerator::GeneratorPair", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbRadialSelectorGeneratorGeneratorInfo", 0),
            ClassVersion::new("hkbRadialSelectorGenerator::GeneratorInfo", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbPoseStoringGeneratorOutputListenerStoredPose", 0),
            ClassVersion::new("hkbPoseStoringGeneratorOutputListener::StoredPose", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbVariableBindingSetBinding", 1),
            ClassVersion::new("hkbVariableBindingSet::Binding", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbCompiledExpressionSetToken", 0),
            ClassVersion::new("hkbCompiledExpressionSet::Token", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbStateMachineDelayedTransitionInfo", 1),
            ClassVersion::new("hkbStateMachine::DelayedTransitionInfo", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbStateMachineNestedStateMachineData", 0),
            ClassVersion::new("hkbStateMachine::NestedStateMachineData", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbStateMachineStateInfo", 4),
            ClassVersion::new("hkbStateMachine::StateInfo", 4),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbStateMachineEventPropertyArray", 0),
            ClassVersion::new("hkbStateMachine::EventPropertyArray", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbStateMachineTransitionInfoArray", 0),
            ClassVersion::new("hkbStateMachine::TransitionInfoArray", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbStateMachineProspectiveTransitionInfo", 2),
            ClassVersion::new("hkbStateMachine::ProspectiveTransitionInfo", 2),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbStateMachineActiveTransitionInfo", 1),
            ClassVersion::new("hkbStateMachine::ActiveTransitionInfo", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbStateMachineTransitionInfoReference", 1),
            ClassVersion::new("hkbStateMachine::TransitionInfoReference", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbStateMachineTransitionInfo", 1),
            ClassVersion::new("hkbStateMachine::TransitionInfo", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbStateMachineTimeInterval", 0),
            ClassVersion::new("hkbStateMachine::TimeInterval", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbIntVariableSequencedDataSample", 0),
            ClassVersion::new("hkbIntVariableSequencedData::Sample", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbBoolVariableSequencedDataSample", 0),
            ClassVersion::new("hkbBoolVariableSequencedData::Sample", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbRealVariableSequencedDataSample", 0),
            ClassVersion::new("hkbRealVariableSequencedData::Sample", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbEventSequencedDataSequencedEvent", 0),
            ClassVersion::new("hkbEventSequencedData::SequencedEvent", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbSenseHandleModifierRange", 0),
            ClassVersion::new("hkbSenseHandleModifier::Range", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbKeyframeBonesModifierKeyframeInfo", 0),
            ClassVersion::new("hkbKeyframeBonesModifier::KeyframeInfo", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbHandIkModifierHand", 4),
            ClassVersion::new("hkbHandIkModifier::Hand", 4),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbHandIkControlsModifierHand", 0),
            ClassVersion::new("hkbHandIkControlsModifier::Hand", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbFootIkModifierInternalLegData", 1),
            ClassVersion::new("hkbFootIkModifier::InternalLegData", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbFootIkModifierLeg", 2),
            ClassVersion::new("hkbFootIkModifier::Leg", 2),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbFootIkControlsModifierLeg", 1),
            ClassVersion::new("hkbFootIkControlsModifier::Leg", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbEvaluateExpressionModifierInternalExpressionData", 0),
            ClassVersion::new("hkbEvaluateExpressionModifier::InternalExpressionData", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbAttributeModifierAssignment", 0),
            ClassVersion::new("hkbAttributeModifier::Assignment", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbGeneratorSyncInfoActiveInterval", 0),
            ClassVersion::new("hkbGeneratorSyncInfo::ActiveInterval", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbGeneratorSyncInfoSyncPoint", 0),
            ClassVersion::new("hkbGeneratorSyncInfo::SyncPoint", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbClipGeneratorEcho", 0),
            ClassVersion::new("hkbClipGenerator::Echo", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbBlenderGeneratorChildInternalState", 0),
            ClassVersion::new("hkbBlenderGenerator::ChildInternalState", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbLayerGeneratorLayerInternalState", 1),
            ClassVersion::new("hkbLayerGenerator::LayerInternalState", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbHandIkDriverInfoHand", 1),
            ClassVersion::new("hkbHandIkDriverInfo::Hand", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbFootIkDriverInfoLeg", 1),
            ClassVersion::new("hkbFootIkDriverInfo::Leg", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbCharacterStringDataFileNameMeshNamePair", 0),
            ClassVersion::new("hkbCharacterStringData::FileNameMeshNamePair", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbCharacterControllerModifier", 2),
            ClassVersion::new("hkbCharacterControllerModifier", 3),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "collisionShapeProfileIdx".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbRigidBodySetup", 0),
            ClassVersion::new("hkbRigidBodySetup", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "collisionShapeProfiles".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hkbShapeSetup".to_string()),
            default: None,
        })
        .with_custom_hook("_hkbRigidBodySetup_0_to_1")
        .with_operation(PatchOperation::MemberRemove {
            name: "shapeSetup".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkbShapeSetup".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbShapeSetup", 0),
            ClassVersion::new("hkbShapeSetup", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "customPivotEnabled".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "customPivotIdx".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbBlendingTransitionEffectInternalState", 2),
            ClassVersion::new("hkbBlendingTransitionEffectInternalState", 3),
        )
        .with_custom_hook("_noop_type_change")
        .with_operation(PatchOperation::Depends {
            class_name: "hkQsTransform".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbClipGeneratorInternalState", 0),
            ClassVersion::new("hkbClipGeneratorInternalState", 1),
        )
        .with_custom_hook("_noop_type_change")
        .with_operation(PatchOperation::Depends {
            class_name: "hkQsTransform".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbExtrapolatingTransitionEffectInternalState", 1),
            ClassVersion::new("hkbExtrapolatingTransitionEffectInternalState", 2),
        )
        .with_custom_hook("_noop_type_change")
        .with_operation(PatchOperation::Depends {
            class_name: "hkQsTransform".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbFootIkModifier::Leg", 2),
            ClassVersion::new("hkbFootIkModifier::Leg", 3),
        )
        .with_custom_hook("_noop_type_change")
        .with_operation(PatchOperation::Depends {
            class_name: "hkQsTransform".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "maxFootPitchDegrees".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(45.0_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "minFootPitchDegrees".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(-45.0_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "maxFootRollDegrees".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(20.0_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "minFootRollDegrees".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(-20.0_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "heelOffsetFromAnkle".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(0.0_f32)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "favorToeInterpenetrationOverSteepSlope".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "favorHeelInterpenetrationOverSteepSlope".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbClientCharacterState", 2),
            ClassVersion::new("hkbClientCharacterState", 3),
        )
        .with_custom_hook("_noop_type_change")
        .with_operation(PatchOperation::Depends {
            class_name: "hkQsTransform".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbPoseStoringGeneratorOutputListener::StoredPose", 0),
            ClassVersion::new("hkbPoseStoringGeneratorOutputListener::StoredPose", 1),
        )
        .with_custom_hook("_noop_type_change")
        .with_operation(PatchOperation::Depends {
            class_name: "hkQsTransform".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbCharacterAddedInfo", 1),
            ClassVersion::new("hkbCharacterAddedInfo", 2),
        )
        .with_custom_hook("_noop_type_change")
        .with_operation(PatchOperation::Depends {
            class_name: "hkQsTransform".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkbCharacterSteppedInfo", 2),
            ClassVersion::new("hkbCharacterSteppedInfo", 3),
        )
        .with_custom_hook("_noop_type_change")
        .with_operation(PatchOperation::Depends {
            class_name: "hkQsTransform".to_string(),
            version: 0,
        }),
    );
    // physics.py
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpCdBody", 1),
            ClassVersion::new("hkpCdBody", 2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "parent".to_string(),
            type_name: "struct".to_string(),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpEntity", 3),
            ClassVersion::new("hkpEntity", 4),
        )
        .with_operation(PatchOperation::MemberRename {
            old_name: "motion".to_string(),
            new_name: "motion_old".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "motion".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkpMaxSizeMotion".to_string()),
            default: None,
        })
        .with_custom_hook("_hkpEntity_3_to_4")
        .with_operation(PatchOperation::MemberRemove {
            name: "motion_old".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpMaxSizeMotion".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpMotion".to_string(),
            version: 3,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpEntity", 4),
            ClassVersion::new("hkpEntity", 5),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "npData".to_string(),
            type_name: "int".to_string(),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpStaticCompoundShapeInstance", 0),
            ClassVersion::new("hkpStaticCompoundShape::Instance", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpShapeKeyTableBlock", 0),
            ClassVersion::new("hkpShapeKeyTable::Block", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpMoppCodeCodeInfo", 0),
            ClassVersion::new("hkpMoppCode::CodeInfo", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpSimpleMeshShapeTriangle", 0),
            ClassVersion::new("hkpSimpleMeshShape::Triangle", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpStorageMeshShapeSubpartStorage", 0),
            ClassVersion::new("hkpStorageMeshShape::SubpartStorage", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpMeshShapeSubpart", 0),
            ClassVersion::new("hkpMeshShape::Subpart", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpSampledHeightFieldShapeCoarseMinMaxLevel", 0),
            ClassVersion::new("hkpSampledHeightFieldShape::CoarseMinMaxLevel", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpStorageExtendedMeshShapeShapeSubpartStorage", 2),
            ClassVersion::new("hkpStorageExtendedMeshShape::ShapeSubpartStorage", 2),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpStorageExtendedMeshShapeMeshSubpartStorage", 3),
            ClassVersion::new("hkpStorageExtendedMeshShape::MeshSubpartStorage", 3),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpStorageExtendedMeshShapeMaterial", 1),
            ClassVersion::new("hkpStorageExtendedMeshShape::Material", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpExtendedMeshShapeShapesSubpart", 1),
            ClassVersion::new("hkpExtendedMeshShape::ShapesSubpart", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpExtendedMeshShapeTrianglesSubpart", 3),
            ClassVersion::new("hkpExtendedMeshShape::TrianglesSubpart", 3),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpExtendedMeshShapeSubpart", 3),
            ClassVersion::new("hkpExtendedMeshShape::Subpart", 3),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpCompressedMeshShapeConvexPiece", 4),
            ClassVersion::new("hkpCompressedMeshShape::ConvexPiece", 4),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpCompressedMeshShapeBigTriangle", 2),
            ClassVersion::new("hkpCompressedMeshShape::BigTriangle", 2),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpCompressedMeshShapeChunk", 4),
            ClassVersion::new("hkpCompressedMeshShape::Chunk", 4),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpMultiRayShapeRay", 0),
            ClassVersion::new("hkpMultiRayShape::Ray", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpListShapeChildInfo", 1),
            ClassVersion::new("hkpListShape::ChildInfo", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpCollidableBoundingVolumeData", 1),
            ClassVersion::new("hkpCollidable::BoundingVolumeData", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpPoweredChainDataConstraintInfo", 0),
            ClassVersion::new("hkpPoweredChainData::ConstraintInfo", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpEntityExtendedListeners", 0),
            ClassVersion::new("hkpEntity::ExtendedListeners", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpEntitySpuCollisionCallback", 0),
            ClassVersion::new("hkpEntity::SpuCollisionCallback", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpEntitySmallArraySerializeOverrideType", 1),
            ClassVersion::new("hkpEntity::SmallArraySerializeOverrideType", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpBreakableMultiMaterialInverseMapping", 0),
            ClassVersion::new("hkpBreakableMultiMaterial::InverseMapping", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpBreakableMultiMaterialInverseMappingDescriptor", 0),
            ClassVersion::new("hkpBreakableMultiMaterial::InverseMappingDescriptor", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpBreakableBodyController", 0),
            ClassVersion::new("hkpBreakableBody::Controller", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpConstraintInstanceSmallArraySerializeOverrideType", 1),
            ClassVersion::new("hkpConstraintInstance::SmallArraySerializeOverrideType", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpGenericConstraintDataSchemeConstraintInfo", 0),
            ClassVersion::new("hkpGenericConstraintDataScheme::ConstraintInfo", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpStiffSpringChainDataConstraintInfo", 0),
            ClassVersion::new("hkpStiffSpringChainData::ConstraintInfo", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpBallSocketChainDataConstraintInfo", 1),
            ClassVersion::new("hkpBallSocketChainData::ConstraintInfo", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpPairCollisionFilterMapPairFilterKeyOverrideType", 0),
            ClassVersion::new("hkpPairCollisionFilter::MapPairFilterKeyOverrideType", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpVehicleInstanceWheelInfo", 2),
            ClassVersion::new("hkpVehicleInstance::WheelInfo", 2),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpVehicleDataWheelComponentParams", 0),
            ClassVersion::new("hkpVehicleData::WheelComponentParams", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpVehicleLinearCastWheelCollideWheelState", 0),
            ClassVersion::new("hkpVehicleLinearCastWheelCollide::WheelState", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpVehicleSuspensionSuspensionWheelParameters", 0),
            ClassVersion::new("hkpVehicleSuspension::SuspensionWheelParameters", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new(
                "hkpVehicleDefaultSuspensionWheelSpringSuspensionParameters",
                0,
            ),
            ClassVersion::new(
                "hkpVehicleDefaultSuspension::WheelSpringSuspensionParameters",
                0,
            ),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpVehiclePerWheelSimulationWheelData", 0),
            ClassVersion::new("hkpVehiclePerWheelSimulation::WheelData", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpVehicleDefaultBrakeWheelBrakingProperties", 0),
            ClassVersion::new("hkpVehicleDefaultBrake::WheelBrakingProperties", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpDisplayBindingDataPhysicsSystem", 1),
            ClassVersion::new("hkpDisplayBindingData::PhysicsSystem", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpDisplayBindingDataRigidBody", 2),
            ClassVersion::new("hkpDisplayBindingData::RigidBody", 2),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpSerializedDisplayRbTransformsDisplayTransformPair", 0),
            ClassVersion::new("hkpSerializedDisplayRbTransforms::DisplayTransformPair", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpPoweredChainMapperLinkInfo", 0),
            ClassVersion::new("hkpPoweredChainMapper::LinkInfo", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpPoweredChainMapperTarget", 0),
            ClassVersion::new("hkpPoweredChainMapper::Target", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpTriggerVolumeEventInfo", 0),
            ClassVersion::new("hkpTriggerVolume::EventInfo", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpCompressedMeshShape", 11),
            ClassVersion::new("hkpCompressedMeshShape", 12),
        )
        .with_custom_hook("_noop_type_change")
        .with_operation(PatchOperation::Depends {
            class_name: "hkQsTransform".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpConvexTransformShape", 2),
            ClassVersion::new("hkpConvexTransformShape", 3),
        )
        .with_custom_hook("_noop_type_change")
        .with_operation(PatchOperation::Depends {
            class_name: "hkQsTransform".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpExtendedMeshShape::TrianglesSubpart", 3),
            ClassVersion::new("hkpExtendedMeshShape::TrianglesSubpart", 4),
        )
        .with_custom_hook("_noop_type_change")
        .with_operation(PatchOperation::Depends {
            class_name: "hkQsTransform".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpStaticCompoundShape::Instance", 0),
            ClassVersion::new("hkpStaticCompoundShape::Instance", 1),
        )
        .with_custom_hook("_noop_type_change")
        .with_operation(PatchOperation::Depends {
            class_name: "hkQsTransform".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpVehicleFrictionStatusAxisStatus", 0),
            ClassVersion::new("hkpVehicleFrictionStatus::AxisStatus", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpVehicleFrictionDescriptionAxisDescription", 0),
            ClassVersion::new("hkpVehicleFrictionDescription::AxisDescription", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpCharacterRigidBodyCinfo", 0),
            ClassVersion::new("hkpCharacterRigidBodyCinfo", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "maxSupportSlope".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(1.49071075_f32)),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkpSerializedTrack1nInfo::Agent1nSector", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "bytesAllocated".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "rawData".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpSerializedTrack1nInfo", 0),
            ClassVersion::new("hkpSerializedTrack1nInfo", 1),
        )
        .with_custom_hook("_noop_type_change")
        .with_operation(PatchOperation::Depends {
            class_name: "hkpAgent1nSector".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkpSerializedTrack1nInfo::Agent1nSector".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpAgent1nSector", 0),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "bytesAllocated".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "pad0".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "pad1".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "pad2".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "data".to_string(),
            type_name: "int8".to_string(),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkcdStaticMeshTreehkcdStaticMeshTreeCommonConfigunsignedintunsignedlonglong1121hkpBvCompressedMeshShapeTreeDataRun", 0),
            ClassVersion::new("hkcdStaticMeshTree< hkcdStaticMeshTreeCommonConfig< hkUint32, hkUint64, 11, 21 >, hkpBvCompressedMeshShapeTreeDataRun >", 0),
        )
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkcdStaticMeshTreeBasePrimitiveDataRunBaseunsignedint", 0),
            ClassVersion::new(
                "hkcdStaticMeshTreeBase::PrimitiveDataRunBase< hkUint32 >",
                0,
            ),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpWheelFrictionConstraintDataAtoms", 0),
            ClassVersion::new("hkpWheelFrictionConstraintData::Atoms", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpWheelFrictionConstraintDataRuntime", 0),
            ClassVersion::new("hkpWheelFrictionConstraintData::Runtime", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpWheelConstraintDataAtoms", 0),
            ClassVersion::new("hkpWheelConstraintData::Atoms", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpStiffSpringConstraintDataAtoms", 1),
            ClassVersion::new("hkpStiffSpringConstraintData::Atoms", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpRotationalConstraintDataAtoms", 0),
            ClassVersion::new("hkpRotationalConstraintData::Atoms", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpRagdollLimitsDataAtoms", 0),
            ClassVersion::new("hkpRagdollLimitsData::Atoms", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpRagdollConstraintDataAtoms", 1),
            ClassVersion::new("hkpRagdollConstraintData::Atoms", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpRackAndPinionConstraintDataAtoms", 0),
            ClassVersion::new("hkpRackAndPinionConstraintData::Atoms", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpPulleyConstraintDataAtoms", 0),
            ClassVersion::new("hkpPulleyConstraintData::Atoms", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpPrismaticConstraintDataAtoms", 0),
            ClassVersion::new("hkpPrismaticConstraintData::Atoms", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpPointToPlaneConstraintDataAtoms", 0),
            ClassVersion::new("hkpPointToPlaneConstraintData::Atoms", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpLimitedHingeConstraintDataAtoms", 1),
            ClassVersion::new("hkpLimitedHingeConstraintData::Atoms", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpHingeLimitsDataAtoms", 0),
            ClassVersion::new("hkpHingeLimitsData::Atoms", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpHingeConstraintDataAtoms", 1),
            ClassVersion::new("hkpHingeConstraintData::Atoms", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpFixedConstraintDataAtoms", 0),
            ClassVersion::new("hkpFixedConstraintData::Atoms", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpDeformableFixedConstraintDataAtoms", 0),
            ClassVersion::new("hkpDeformableFixedConstraintData::Atoms", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpCogWheelConstraintDataAtoms", 0),
            ClassVersion::new("hkpCogWheelConstraintData::Atoms", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpLinearClearanceConstraintDataAtoms", 0),
            ClassVersion::new("hkpLinearClearanceConstraintData::Atoms", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpBallAndSocketConstraintDataAtoms", 1),
            ClassVersion::new("hkpBallAndSocketConstraintData::Atoms", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkp6DofConstraintDataBlueprints", 0),
            ClassVersion::new("hkp6DofConstraintData::Blueprints", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkpWheelFrictionConstraintAtomAxle", 0),
            ClassVersion::new("hkpWheelFrictionConstraintAtom::Axle", 0),
        ),
    );
    // physics_np.py
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpRagdollKeyFrameHierarchyUtilityControlData", 0),
            ClassVersion::new("hknpRagdollKeyFrameHierarchyUtility::ControlData", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new(
                "hkFreeListArrayhknpMaterialhknpMaterialId8hknpMaterialFreeListArrayOperations",
                0,
            ),
            ClassVersion::new("hkFreeListArray< hknpMaterial, 8 >", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkFreeListArrayhknpShapeInstancehkHandleshort32767hknpShapeInstanceIdDiscriminant8hknpShapeInstance", 0),
            ClassVersion::new("hkFreeListArray< hknpShapeInstance, 8 >", 0),
        )
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkFreeListArrayhknpMotionPropertieshknpMotionPropertiesId8hknpMotionPropertiesFreeListArrayOperations", 0),
            ClassVersion::new("hkFreeListArray< hknpMotionProperties, 8 >", 0),
        )
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkFreeListArrayhknpMotionPropertieshknpMotionPropertiesId8hknpMotionPropertiesLibraryFreeListArrayOperations", 0),
            ClassVersion::new("hkFreeListArray< hknpMotionProperties, 8 >", 0),
        )
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkFreeListArrayElementhknpMaterial", 2),
            ClassVersion::new("hkFreeListArrayElement< hknpMaterial >", 2),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpGroupCollisionFilterBasehknpGroupCollisionFilterTypesConfig55516", 0),
            ClassVersion::new("hknpGroupCollisionFilterBase< hknpGroupCollisionFilterTypes::Config< 5, 5, 5, 16 > >", 0),
        )
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpSparseCompactMapunsignedshort", 0),
            ClassVersion::new("hknpSparseCompactMap< hkUint16 >", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpPhysicsSystemDatabodyCinfoWithAttachment", 0),
            ClassVersion::new("hknpPhysicsSystemData::bodyCinfoWithAttachment", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpCompressedMeshShapeInternalsKeyMask", 0),
            ClassVersion::new("hknpCompressedMeshShapeInternals::KeyMask", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpVehicleInstanceWheelInfo", 0),
            ClassVersion::new("hknpVehicleInstance::WheelInfo", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpVehicleDataWheelComponentParams", 0),
            ClassVersion::new("hknpVehicleData::WheelComponentParams", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpVehicleLinearCastWheelCollideWheelState", 0),
            ClassVersion::new("hknpVehicleLinearCastWheelCollide::WheelState", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpVehicleSuspensionSuspensionWheelParameters", 0),
            ClassVersion::new("hknpVehicleSuspension::SuspensionWheelParameters", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new(
                "hknpVehicleDefaultSuspensionWheelSpringSuspensionParameters",
                0,
            ),
            ClassVersion::new(
                "hknpVehicleDefaultSuspension::WheelSpringSuspensionParameters",
                0,
            ),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpVehicleDefaultBrakeWheelBrakingProperties", 0),
            ClassVersion::new("hknpVehicleDefaultBrake::WheelBrakingProperties", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpMaskedShapeMaskWrapper", 0),
            ClassVersion::new("hknpMaskedShape::MaskWrapper", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpConvexPolytopeShapeConnectivityEdge", 0),
            ClassVersion::new("hknpConvexPolytopeShape::Connectivity::Edge", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpConvexPolytopeShapeConnectivity", 0),
            ClassVersion::new("hknpConvexPolytopeShape::Connectivity", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpConvexPolytopeShapeFace", 0),
            ClassVersion::new("hknpConvexPolytopeShape::Face", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpLodShapeLevelOfDetailInfo", 0),
            ClassVersion::new("hknpLodShape::LevelOfDetailInfo", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpMinMaxQuadTreeMinMaxLevel", 0),
            ClassVersion::new("hknpMinMaxQuadTree::MinMaxLevel", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpPairCollisionFilterMapPairFilterKeyOverrideType", 0),
            ClassVersion::new("hknpPairCollisionFilter::MapPairFilterKeyOverrideType", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpBroadPhaseConfigLayer", 0),
            ClassVersion::new("hknpBroadPhaseConfig::Layer", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpShape", 3),
            ClassVersion::new("hknpShape", 4),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "type".to_string(),
            type_name: "uint8".to_string(),
            ctype: None,
            default: None,
        })
        .with_custom_hook("hknpShape_3_to_4"),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpCompositeShape", 0),
            ClassVersion::new("hknpCompositeShape", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "materialTable".to_string(),
            type_name: "pointer".to_string(),
            ctype: Some("hkReferencedObject".to_string()),
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hknpHeightFieldGeometry", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hknpDefaultHeightFieldGeometry", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hknpHeightFieldGeometry".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "storage".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "shapeTags".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "triangleFlip".to_string(),
            type_name: "uint8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "resX".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "resZ".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "heightScale".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "heightOffset".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hknpHeightFieldBoundingVolume", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "minMaxTree".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hknpMinMaxQuadTree".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "minLevel".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpHeightFieldShape", 3),
            ClassVersion::new("hknpHeightFieldShape", 4),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "geometry".to_string(),
            type_name: "pointer".to_string(),
            ctype: Some("hknpHeightFieldGeometry".to_string()),
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpHeightFieldShape", 4),
            ClassVersion::new("hknpHeightFieldShape", 5),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "boundingVolumeData".to_string(),
            type_name: "pointer".to_string(),
            ctype: Some("hknpHeightFieldBoundingVolume".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "markBorderEdgesForWelding".to_string(),
            type_name: "uint8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "minMaxTreeCoarseness".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "minMaxTree".to_string(),
            type_name: "struct".to_string(),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpCompressedMeshShape", 5),
            ClassVersion::new("hknpCompressedMeshShape", 6),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "quadIsFlat".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::MemberRename {
            old_name: "data".to_string(),
            new_name: "data_old".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "data".to_string(),
            type_name: "pointer".to_string(),
            ctype: Some("hkReferencedObject".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "data_old".to_string(),
            type_name: "pointer".to_string(),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hknpDebrisShape", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hknpConvexPolytopeShape".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "obb".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkcdObb".to_string()),
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hknpBoxShape", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hknpConvexPolytopeShape".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "obb".to_string(),
            type_name: "vec16".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hknpLodShape", 4),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hknpShape".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "variants".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hknpShape".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "lodTypeToVariant".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hknpLodShapeIndex".to_string()),
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hknpLodShapeIndex", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "data".to_string(),
            type_name: "uint8".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpLodShape::LevelOfDetailInfo", 0),
            ClassVersion::new("hknpLodMeshShape::LevelOfDetailInfo", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "levelOfDetail".to_string(),
            type_name: "uint8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(2)),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpLodShape", 2),
            ClassVersion::new("hknpLodMeshShape", 3),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "numLevelsOfDetail".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "shapes".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberRename {
            old_name: "infos".to_string(),
            new_name: "infos_old".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "infos".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hknpLodShape::LevelOfDetailInfo".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "infos_old".to_string(),
            type_name: "array".to_string(),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpLodMeshShape", 3),
            ClassVersion::new("hknpLodMeshShape", 4),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: Some("hknpCompositeShape".to_string()),
            new_parent: Some("hknpLodShape".to_string()),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpShapeTagCodec", 1),
            ClassVersion::new("hknpShapeTagCodec", 2),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "hints".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpStaticCompoundShape", 1),
            ClassVersion::new("hknpStaticCompoundShape", 2),
        )
        .with_operation(PatchOperation::MemberRename {
            old_name: "boundingVolumeData".to_string(),
            new_name: "boundingVolumeData_old".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "boundingVolumeData".to_string(),
            type_name: "pointer".to_string(),
            ctype: Some("hknpDynamicCompoundShapeData".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "boundingVolumeData_old".to_string(),
            type_name: "pointer".to_string(),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpStaticCompoundShape", 2),
            ClassVersion::new("hknpDynamicCompoundShape", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpStaticCompoundShapeData", 0),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: Some("hkReferencedObject".to_string()),
            new_parent: None,
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "aabbTree".to_string(),
            type_name: "struct".to_string(),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpStaticCompoundShapeTree", 0),
            ClassVersion::new("", -2),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: Some("hkcdStaticTreeDefaultTreeStorage6".to_string()),
            new_parent: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpStaticCompoundShapeKeyMask", 0),
            ClassVersion::new("hknpDynamicCompoundShapeKeyMask", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpCompoundShape", 2),
            ClassVersion::new("hknpCompoundShapeBase", 3),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "estimatedNumShapeKeys".to_string(),
            type_name: "uint8".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpExternMeshShape", 1),
            ClassVersion::new("hknpExternMeshShape", 2),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpExternMeshShapeData", 0),
            ClassVersion::new("hknpExternMeshShapeData", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "hasBuildContext".to_string(),
            type_name: "uint8".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpDynamicCompoundShape", 1),
            ClassVersion::new("hknpCompoundShape", 3),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpCompoundShapeInternalsKeyMask", 0),
            ClassVersion::new("hknpCompoundShapeInternalsKeyMask", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpCompoundShape", 3),
            ClassVersion::new("hknpCompoundShape", 4),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hknpBodyId", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "serialAndIndex".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpBodyReference", 0),
            ClassVersion::new("hknpBodyReference", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpBodyCinfo", 4),
            ClassVersion::new("hknpBodyCinfo", 5),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "collisionCntrl".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpBody", 3),
            ClassVersion::new("hknpBody", 4),
        )
        .with_operation(PatchOperation::MemberRename {
            old_name: "aabb".to_string(),
            new_name: "aabb_old".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "aabb".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkAabb24_16_24".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "maxTimDistanceFromRotation".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "lodFlags".to_string(),
            type_name: "uint8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "avgSurfaceVelocity".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_custom_hook("hknpBody_3_to_4")
        .with_operation(PatchOperation::MemberRemove {
            name: "aabb_old".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "shapeSizeDiv16".to_string(),
            type_name: "uint8".to_string(),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpBody", 4),
            ClassVersion::new("hknpBody", 5),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "collisionControl".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpBody", 5),
            ClassVersion::new("hknpBody", 6),
        )
        .with_operation(PatchOperation::MemberRename {
            old_name: "motionToBodyRotation".to_string(),
            new_name: "motionToBodyRotation_old".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "motionToBodyRotation".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_custom_hook("hknpBody_5_to_6")
        .with_operation(PatchOperation::MemberRemove {
            name: "motionToBodyRotation_old".to_string(),
            type_name: "array".to_string(),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hknpPhysicsSystemDatabodyCinfoWithAttachment", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hknpBodyCinfo".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "attachedBody".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(-1)),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpPhysicsSystemData", 0),
            ClassVersion::new("hknpPhysicsSystemData", 1),
        )
        .with_operation(PatchOperation::MemberRename {
            old_name: "bodyCinfos".to_string(),
            new_name: "bodyCinfos_old".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "bodyCinfos".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hknpPhysicsSystemDatabodyCinfoWithAttachment".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "bodyCinfos_old".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "motionCinfos".to_string(),
            type_name: "array".to_string(),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpWorldCinfo", 6),
            ClassVersion::new("hknpWorldCinfo", 7),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "broadPhaseType".to_string(),
            type_name: "uint8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(1)),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpWorldCinfo", 7),
            ClassVersion::new("hknpWorldCinfo", 8),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "aabbMargin".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(0.01_f32)),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpWorldCinfo", 8),
            ClassVersion::new("hknpWorldCinfo", 9),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "defaultSolverTimestep".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "lodManagerCinfo".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hknpLodManagerCinfo".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "enablePenetrationRecovery".to_string(),
            type_name: "uint8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(1)),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hknpLodManagerInfo", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "lodEnabled".to_string(),
            type_name: "uint8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "autoBuildLodOnDynamicBodyAdded".to_string(),
            type_name: "uint8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "autoBuildLodOnMeshBodyAdded".to_string(),
            type_name: "uint8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "lodAccuray".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "slowToFastThreshold".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "fastToSlowThreshold".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "bodyIsBigThreshold".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "avgVelocityGain".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpLodManagerInfo", 0),
            ClassVersion::new("hknpLodManagerCinfo", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpLodManagerCinfo", 0),
            ClassVersion::new("hknpLodManagerCinfo", 1),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "lodEnabled".to_string(),
            type_name: "uint8".to_string(),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "registerDefaultConfig".to_string(),
            type_name: "uint8".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpConstraintCinfo", 3),
            ClassVersion::new("hknpConstraintCinfo", 4),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "desiredConstraintId".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hknpConstraintId".to_string()),
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hknpConstraintId", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "value".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpConstraint", 2),
            ClassVersion::new("hknpConstraint", 3),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "type".to_string(),
            type_name: "uint8".to_string(),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpConstraint", 3),
            ClassVersion::new("hknpConstraint", 4),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpMotion", 3),
            ClassVersion::new("hknpMotion", 4),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hknpMotionProperties::FreeListArrayOperations", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hknpMaterial::FreeListArrayOperations", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpMaterial", 1),
            ClassVersion::new("hknpMaterial", 2),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpBodyQuality", 1),
            ClassVersion::new("hknpBodyQuality", 2),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hkFreeListArrayElementhknpMaterial", 1),
            ClassVersion::new("hkFreeListArrayElementhknpMaterial", 2),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hkFreeListArrayhknpMotionPropertieshknpMotionPropertiesId8hknpMotionPropertiesLibraryFreeListArrayOperations", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "elements".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hknpMotionProperties".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "firstFree".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hknpBodyData", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "shape".to_string(),
            type_name: "pointer".to_string(),
            ctype: Some("hknpShape".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "bodyQuality".to_string(),
            type_name: "pointer".to_string(),
            ctype: Some("hknpBodyQuality".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "motionPropertiesData".to_string(),
            type_name: "pointer".to_string(),
            ctype: Some("hknpMotionPropertiesData".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "material".to_string(),
            type_name: "pointer".to_string(),
            ctype: Some("hknpMaterial".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "initialPosition".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "initialOrientation".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "massProperties".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hkMassProperties".to_string()),
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hknpMaterialData", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "density".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "inertiaFactor".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "dynamicFriction".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "staticFriction".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "restitution".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "frictionCombinePolicy".to_string(),
            type_name: "uint8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "restitutionCombinePolicy".to_string(),
            type_name: "uint8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "weldingTolerance".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "triggerVolumeType".to_string(),
            type_name: "uint8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "triggerVolumeTolerance".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "maxContactImpulse".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "fractionOfClippedImpulseToApply".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "massChangerCategory".to_string(),
            type_name: "uint8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "massChangerHeavyObjectFactor".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "softContactForceFactor".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "softContactDampFactor".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "softContactSeperationVelocity".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "surfaceVelocity".to_string(),
            type_name: "pointer".to_string(),
            ctype: Some("hknpSurfaceVelocity".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "disablingCollisionsBetweenCvxCvxDynamicObjectsDistance".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hknpMotionPropertiesData", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "flags".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "gravityFactor".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "maxLinearSpeed".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "maxAngularSpeed".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "linearDamping".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "angularDamping".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "solverStabilization".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "deactivationReferenceDistance".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "deactivationReferenceRotation".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "deactivationStrategy".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpCharacterRigidBodyCinfo", 3),
            ClassVersion::new("hknpCharacterRigidBodyCinfo", 4),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "maxSupportSlope".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: Some(PatchValue::Real(1.49071075_f32)),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpCharacterRigidBodyCinfo", 4),
            ClassVersion::new("hknpCharacterRigidBodyCinfo", 5),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpVehicleData", 0),
            ClassVersion::new("hknpVehicleData", 1),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "alreadyInitialised".to_string(),
            type_name: "uint8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "chassisFrictionInertiaInvDiag".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "frictionDescription".to_string(),
            type_name: "struct".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "numWheelsPerAxle".to_string(),
            type_name: "array".to_string(),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpVehicleLinearCastWheelCollide", 0),
            ClassVersion::new("hknpVehicleLinearCastWheelCollide", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpUnaryAction", 1),
            ClassVersion::new("hknpUnaryAction", 2),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hknpBinaryAction", 1),
            ClassVersion::new("hknpBinaryAction", 2),
        ),
    );
    // cloth.py
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclSimClothSetupObjectTransferMotionSetupData", 0),
            ClassVersion::new("hclSimClothSetupObject::TransferMotionSetupData", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclSetupMeshSectionTriangle", 0),
            ClassVersion::new("hclSetupMeshSection::Triangle", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclStorageSetupMeshSectionBoneInfluences", 0),
            ClassVersion::new("hclStorageSetupMeshSection::BoneInfluences", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new(
                "hclStorageSetupMeshSectionSectionTriangleSelectionChannel",
                0,
            ),
            ClassVersion::new(
                "hclStorageSetupMeshSection::SectionTriangleSelectionChannel",
                0,
            ),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclStorageSetupMeshSectionSectionEdgeSelectionChannel", 0),
            ClassVersion::new("hclStorageSetupMeshSection::SectionEdgeSelectionChannel", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclStorageSetupMeshSectionSectionVertexFloatChannel", 0),
            ClassVersion::new("hclStorageSetupMeshSection::SectionVertexFloatChannel", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclStorageSetupMeshSectionSectionVertexSelectionChannel", 0),
            ClassVersion::new(
                "hclStorageSetupMeshSection::SectionVertexSelectionChannel",
                0,
            ),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclStorageSetupMeshSectionSectionVertexChannel", 0),
            ClassVersion::new("hclStorageSetupMeshSection::SectionVertexChannel", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclStorageSetupMeshBone", 0),
            ClassVersion::new("hclStorageSetupMesh::Bone", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclStorageSetupMeshTriangleChannel", 0),
            ClassVersion::new("hclStorageSetupMesh::TriangleChannel", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclStorageSetupMeshEdgeChannel", 0),
            ClassVersion::new("hclStorageSetupMesh::EdgeChannel", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclStorageSetupMeshVertexChannel", 0),
            ClassVersion::new("hclStorageSetupMesh::VertexChannel", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBonePlanesSetupObjectPerParticleAngle", 0),
            ClassVersion::new("hclBonePlanesSetupObject::PerParticleAngle", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBonePlanesSetupObjectGlobalPlane", 0),
            ClassVersion::new("hclBonePlanesSetupObject::GlobalPlane", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBonePlanesSetupObjectPerParticlePlane", 0),
            ClassVersion::new("hclBonePlanesSetupObject::PerParticlePlane", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclTransformSetUsageTransformTracker", 0),
            ClassVersion::new("hclTransformSetUsage::TransformTracker", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclClothStateTransformSetAccess", 0),
            ClassVersion::new("hclClothState::TransformSetAccess", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclClothStateBufferAccess", 2),
            ClassVersion::new("hclClothState::BufferAccess", 2),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclSimClothDataTransferMotionData", 0),
            ClassVersion::new("hclSimClothData::TransferMotionData", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclSimClothDataCollidablePinchingData", 1),
            ClassVersion::new("hclSimClothData::CollidablePinchingData", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclSimClothDataCollidableTransformMap", 0),
            ClassVersion::new("hclSimClothData::CollidableTransformMap", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclSimClothDataParticleData", 2),
            ClassVersion::new("hclSimClothData::ParticleData", 2),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclSimClothDataOverridableSimulationInfo", 2),
            ClassVersion::new("hclSimClothData::OverridableSimulationInfo", 2),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclSimClothDataLandscapeCollisionData", 0),
            ClassVersion::new("hclSimClothData::LandscapeCollisionData", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclGatherSomeVerticesOperatorVertexPair", 0),
            ClassVersion::new("hclGatherSomeVerticesOperator::VertexPair", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclUpdateSomeVertexFramesOperatorTriangle", 0),
            ClassVersion::new("hclUpdateSomeVertexFramesOperator::Triangle", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclSkinOperatorBoneInfluence", 2),
            ClassVersion::new("hclSkinOperator::BoneInfluence", 2),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclMoveParticlesOperatorVertexParticlePair", 0),
            ClassVersion::new("hclMoveParticlesOperator::VertexParticlePair", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclMeshMeshDeformOperatorTriangleVertexPair", 0),
            ClassVersion::new("hclMeshMeshDeformOperator::TriangleVertexPair", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclSimpleMeshBoneDeformOperatorTriangleBonePair", 0),
            ClassVersion::new("hclSimpleMeshBoneDeformOperator::TriangleBonePair", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclMeshBoneDeformOperatorTriangleBonePair", 0),
            ClassVersion::new("hclMeshBoneDeformOperator::TriangleBonePair", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclObjectSpaceDeformerLocalBlockUnpackedPNTB", 0),
            ClassVersion::new("hclObjectSpaceDeformer::LocalBlockUnpackedPNTB", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclObjectSpaceDeformerLocalBlockUnpackedPNT", 0),
            ClassVersion::new("hclObjectSpaceDeformer::LocalBlockUnpackedPNT", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclObjectSpaceDeformerLocalBlockUnpackedPN", 0),
            ClassVersion::new("hclObjectSpaceDeformer::LocalBlockUnpackedPN", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclObjectSpaceDeformerLocalBlockUnpackedP", 0),
            ClassVersion::new("hclObjectSpaceDeformer::LocalBlockUnpackedP", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclObjectSpaceDeformerLocalBlockPNTB", 0),
            ClassVersion::new("hclObjectSpaceDeformer::LocalBlockPNTB", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclObjectSpaceDeformerLocalBlockPNT", 0),
            ClassVersion::new("hclObjectSpaceDeformer::LocalBlockPNT", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclObjectSpaceDeformerLocalBlockPN", 0),
            ClassVersion::new("hclObjectSpaceDeformer::LocalBlockPN", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclObjectSpaceDeformerLocalBlockP", 0),
            ClassVersion::new("hclObjectSpaceDeformer::LocalBlockP", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclObjectSpaceDeformerOneBlendEntryBlock", 0),
            ClassVersion::new("hclObjectSpaceDeformer::OneBlendEntryBlock", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclObjectSpaceDeformerTwoBlendEntryBlock", 0),
            ClassVersion::new("hclObjectSpaceDeformer::TwoBlendEntryBlock", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclObjectSpaceDeformerThreeBlendEntryBlock", 0),
            ClassVersion::new("hclObjectSpaceDeformer::ThreeBlendEntryBlock", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclObjectSpaceDeformerFourBlendEntryBlock", 0),
            ClassVersion::new("hclObjectSpaceDeformer::FourBlendEntryBlock", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclObjectSpaceDeformerFiveBlendEntryBlock", 0),
            ClassVersion::new("hclObjectSpaceDeformer::FiveBlendEntryBlock", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclObjectSpaceDeformerSixBlendEntryBlock", 0),
            ClassVersion::new("hclObjectSpaceDeformer::SixBlendEntryBlock", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclObjectSpaceDeformerSevenBlendEntryBlock", 0),
            ClassVersion::new("hclObjectSpaceDeformer::SevenBlendEntryBlock", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclObjectSpaceDeformerEightBlendEntryBlock", 0),
            ClassVersion::new("hclObjectSpaceDeformer::EightBlendEntryBlock", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBoneSpaceDeformerLocalBlockUnpackedPNTB", 0),
            ClassVersion::new("hclBoneSpaceDeformer::LocalBlockUnpackedPNTB", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBoneSpaceDeformerLocalBlockUnpackedPNT", 0),
            ClassVersion::new("hclBoneSpaceDeformer::LocalBlockUnpackedPNT", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBoneSpaceDeformerLocalBlockUnpackedPN", 0),
            ClassVersion::new("hclBoneSpaceDeformer::LocalBlockUnpackedPN", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBoneSpaceDeformerLocalBlockUnpackedP", 0),
            ClassVersion::new("hclBoneSpaceDeformer::LocalBlockUnpackedP", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBoneSpaceDeformerLocalBlockPNTB", 0),
            ClassVersion::new("hclBoneSpaceDeformer::LocalBlockPNTB", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBoneSpaceDeformerLocalBlockPNT", 0),
            ClassVersion::new("hclBoneSpaceDeformer::LocalBlockPNT", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBoneSpaceDeformerLocalBlockPN", 0),
            ClassVersion::new("hclBoneSpaceDeformer::LocalBlockPN", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBoneSpaceDeformerLocalBlockP", 0),
            ClassVersion::new("hclBoneSpaceDeformer::LocalBlockP", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBoneSpaceDeformerOneBlendEntryBlock", 0),
            ClassVersion::new("hclBoneSpaceDeformer::OneBlendEntryBlock", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBoneSpaceDeformerTwoBlendEntryBlock", 0),
            ClassVersion::new("hclBoneSpaceDeformer::TwoBlendEntryBlock", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBoneSpaceDeformerThreeBlendEntryBlock", 1),
            ClassVersion::new("hclBoneSpaceDeformer::ThreeBlendEntryBlock", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBoneSpaceDeformerFourBlendEntryBlock", 1),
            ClassVersion::new("hclBoneSpaceDeformer::FourBlendEntryBlock", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclRuntimeConversionInfoElementConversion", 0),
            ClassVersion::new("hclRuntimeConversionInfo::ElementConversion", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclRuntimeConversionInfoSlotConversion", 0),
            ClassVersion::new("hclRuntimeConversionInfo::SlotConversion", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBlendSomeVerticesOperatorBlendEntry", 0),
            ClassVersion::new("hclBlendSomeVerticesOperator::BlendEntry", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclVolumeConstraintMxApplySingleData", 0),
            ClassVersion::new("hclVolumeConstraintMx::ApplySingleData", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclVolumeConstraintMxApplyBatchData", 0),
            ClassVersion::new("hclVolumeConstraintMx::ApplyBatchData", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclVolumeConstraintMxFrameSingleData", 0),
            ClassVersion::new("hclVolumeConstraintMx::FrameSingleData", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclVolumeConstraintMxFrameBatchData", 0),
            ClassVersion::new("hclVolumeConstraintMx::FrameBatchData", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclVolumeConstraintApplyData", 0),
            ClassVersion::new("hclVolumeConstraint::ApplyData", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclVolumeConstraintFrameData", 0),
            ClassVersion::new("hclVolumeConstraint::FrameData", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclTransitionConstraintSetPerParticle", 1),
            ClassVersion::new("hclTransitionConstraintSet::PerParticle", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclStretchLinkConstraintSetMxSingle", 0),
            ClassVersion::new("hclStretchLinkConstraintSetMx::Single", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclStretchLinkConstraintSetMxBatch", 0),
            ClassVersion::new("hclStretchLinkConstraintSetMx::Batch", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclStretchLinkConstraintSetLink", 0),
            ClassVersion::new("hclStretchLinkConstraintSet::Link", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclStandardLinkConstraintSetMxSingle", 0),
            ClassVersion::new("hclStandardLinkConstraintSetMx::Single", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclStandardLinkConstraintSetMxBatch", 0),
            ClassVersion::new("hclStandardLinkConstraintSetMx::Batch", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclStandardLinkConstraintSetLink", 0),
            ClassVersion::new("hclStandardLinkConstraintSet::Link", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclLocalRangeConstraintSetLocalConstraint", 0),
            ClassVersion::new("hclLocalRangeConstraintSet::LocalConstraint", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclCompressibleLinkConstraintSetMxSingle", 0),
            ClassVersion::new("hclCompressibleLinkConstraintSetMx::Single", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclCompressibleLinkConstraintSetMxBatch", 0),
            ClassVersion::new("hclCompressibleLinkConstraintSetMx::Batch", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclCompressibleLinkConstraintSetLink", 0),
            ClassVersion::new("hclCompressibleLinkConstraintSet::Link", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBonePlanesConstraintSetBonePlane", 0),
            ClassVersion::new("hclBonePlanesConstraintSet::BonePlane", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBendStiffnessConstraintSetMxSingle", 0),
            ClassVersion::new("hclBendStiffnessConstraintSetMx::Single", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBendStiffnessConstraintSetMxBatch", 0),
            ClassVersion::new("hclBendStiffnessConstraintSetMx::Batch", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBendStiffnessConstraintSetLink", 1),
            ClassVersion::new("hclBendStiffnessConstraintSet::Link", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBendLinkConstraintSetMxSingle", 0),
            ClassVersion::new("hclBendLinkConstraintSetMx::Single", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBendLinkConstraintSetMxBatch", 0),
            ClassVersion::new("hclBendLinkConstraintSetMx::Batch", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBendLinkConstraintSetLink", 0),
            ClassVersion::new("hclBendLinkConstraintSet::Link", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclAntiPinchConstraintSetPerParticle", 1),
            ClassVersion::new("hclAntiPinchConstraintSet::PerParticle", 1),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBufferLayoutSlot", 0),
            ClassVersion::new("hclBufferLayout::Slot", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBufferLayoutBufferElement", 0),
            ClassVersion::new("hclBufferLayout::BufferElement", 0),
        ),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hclStateDependencyGraph::Branch", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "branchId".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "stateOperatorIndices".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "parentBranches".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "childBranches".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hclStateDependencyGraph", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "branches".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclStateDependencyGraph::Branch".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "rootBranchIds".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "children".to_string(),
            type_name: "struct".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "parents".to_string(),
            type_name: "struct".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "multiThreadable".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclStateDependencyGraph::Branch".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclClothState", 1),
            ClassVersion::new("hclClothState", 2),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "dependencyGraph".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hclStateDependencyGraph".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclStateDependencyGraph".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclOperator", 0),
            ClassVersion::new("hclOperator", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "usedTransformSets".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclClothState::TransformSetAccess".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclClothState::TransformSetAccess".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "usedBuffers".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclClothState::BufferAccess".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclClothState::BufferAccess".to_string(),
            version: 2,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hclStateTransition", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "name".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "stateIds".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "stateTransitionData".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclStateTransition::StateTransitionData".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclStateTransition::StateTransitionData".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simClothTransitionConstraints".to_string(),
            type_name: "struct".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hclStateTransition::SimClothTransitionData", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "isSimulated".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "transitionType".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "transitionConstraints".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hclStateTransition::BlendOpTransitionData", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "bufferASimCloths".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "bufferBSimCloths".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "transitionType".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "blendOperatorId".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "blendWeightType".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hclStateTransition::StateTransitionData", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "simClothTransitionData".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclStateTransition::SimClothTransitionData".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclStateTransition::SimClothTransitionData".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclStateTransition::BlendOpTransitionData".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "simulatedState".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "emptyState".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "blendOpTransitionData".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclStateTransition::BlendOpTransitionData".to_string()),
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclClothData", 1),
            ClassVersion::new("hclClothData", 2),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "stateTransitions".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclStateTransition".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclStateTransition".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclSimClothSetupObject", 5),
            ClassVersion::new("hclSimClothSetupObject", 6),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "virtualCollisionPointDensities".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hclVertexFloatInput".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "landscapeVirtualCollisionPoints".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hclVertexSelectionInput".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "virtualCollisionPoints".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hclVertexSelectionInput".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "virtualCollisionPointCollidables".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "landscapeVirtualCollisionPointDensities".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hclVertexFloatInput".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "virtualCollisionPointUseAllCollidables".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(1)),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclVertexFloatInput".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclVertexSelectionInput".to_string(),
            version: 0,
        })
        .with_custom_hook("_hclSimClothSetupObject_5_to_6"),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclSimClothSetupObjectPerInstanceCollidable", 3),
            ClassVersion::new("hclSimClothSetupObject::PerInstanceCollidable", 4),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "vcpCollisionEnabled".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hclBoneSpaceTransferSimulationOperator", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hclBoneSpaceMeshMeshDeformPOperator".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclBoneSpaceMeshMeshDeformPOperator".to_string(),
            version: 1,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "inputBufferPrevIdx".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "outputBufferPrevIdx".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hclObjectSpaceTransferSimulationOperator", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hclObjectSpaceMeshMeshDeformPOperator".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclObjectSpaceMeshMeshDeformPOperator".to_string(),
            version: 1,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "inputBufferPrevIdx".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "outputBufferPrevIdx".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclSimulateOperator", 3),
            ClassVersion::new("hclSimulateOperator", 4),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "simulateOpConfigs".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclSimulateOperator::Config".to_string()),
            default: None,
        })
        .with_custom_hook("_hclSimulateOperator_3_to_4")
        .with_operation(PatchOperation::MemberRemove {
            name: "adaptConstraintStiffness".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "subSteps".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "numberOfSolveIterations".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "constraintExecution".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclSimulateOperator::Config".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hclSimulateOperator::Config", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "name".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "constraintExecution".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "instanceCollidablesUsed".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "subSteps".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "numberOfSolveIterations".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "useAllInstanceCollidables".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(1)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "adaptConstraintStiffness".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hclStateOperatorMask", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "operatorStepMask".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "usedBuffers".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclClothState::BufferAccess".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclClothState::BufferAccess".to_string(),
            version: 2,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "usedTransformSets".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclClothState::TransformSetAccess".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclClothState::TransformSetAccess".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hclStateTransitionSetupObject", 0),
        )
        .with_operation(PatchOperation::ParentSet {
            old_parent: None,
            new_parent: Some("hkReferencedObject".to_string()),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hkReferencedObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "name".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "stateSetupObjects".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclClothStateSetupObject".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "useDynamicBlendTransitions".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "useTransitionConstraints".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclClothStateSetupObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclClothSetupObject", 0),
            ClassVersion::new("hclClothSetupObject", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "stateTransitionSetupObjects".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclStateTransitionSetupObject".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclStateTransitionSetupObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclMeshMeshDeformSetupObject", 2),
            ClassVersion::new("hclMeshMeshDeformSetupObject", 3),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "outputBufferPrevSetupObject".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hclBufferSetupObject".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclBufferSetupObject".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "inputBufferPrevSetupObject".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hclBufferSetupObject".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclBufferSetupObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hclSimulateSetupObject::Config", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "name".to_string(),
            type_name: "string".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "numberOfSubsteps".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "adaptConstraintStiffness".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "numberOfSolveIterations".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "constraintSetExecutionOrder".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclConstraintSetSetupObject".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "explicitConstraintOrder".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "specificCollidables".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "useAllCollidables".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclConstraintSetSetupObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclSimulateSetupObject", 3),
            ClassVersion::new("hclSimulateSetupObject", 4),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "simulateConfigs".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclSimulateSetupObject::Config".to_string()),
            default: None,
        })
        .with_custom_hook("_hclSimulateSetupObject_3_to_4")
        .with_operation(PatchOperation::MemberRemove {
            name: "adaptConstraintStiffness".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "constraintSetExecutionOrder".to_string(),
            type_name: "array".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "numberOfSubsteps".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "numberOfSolveIterations".to_string(),
            type_name: "int".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "explicitConstraintOrder".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclSimulateSetupObject::Config".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclConstraintSetSetupObject".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclSimClothData::LandscapeCollisionData", 0),
            ClassVersion::new("hclSimClothData::LandscapeCollisionData", 1),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "collisionTolerance".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclSimClothData", 12),
            ClassVersion::new("hclSimClothData", 13),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "simOpIds".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "pinchDetectionEnabled".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "landscapeCollisionEnabled".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "virtualCollisionPointsData".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hclVirtualCollisionPointsData".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclVirtualCollisionPointsData".to_string(),
            version: 0,
        })
        .with_custom_hook("_hclSimClothData_12_to_13"),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclSimClothData::OverridableSimulationInfo", 2),
            ClassVersion::new("hclSimClothData::OverridableSimulationInfo", 3),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "landscapeCollisionEnabled".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "pinchDetectionEnabled".to_string(),
            type_name: "int8".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "collisionTolerance".to_string(),
            type_name: "real".to_string(),
        })
        .with_operation(PatchOperation::MemberRemove {
            name: "subSteps".to_string(),
            type_name: "int".to_string(),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclOperator", 1),
            ClassVersion::new("hclOperator", 2),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "operatorID".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclClothData", 2),
            ClassVersion::new("hclClothData", 3),
        )
        .with_custom_hook("_hclClothData_2_to_3")
        .with_operation(PatchOperation::Depends {
            class_name: "hclOperator".to_string(),
            version: 2,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBlendSomeVerticesOperator", 1),
            ClassVersion::new("hclBlendSomeVerticesOperator", 2),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "blendVertices".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hclBlendSomeVerticesOperator::BlendVertices".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "dynamicBlend".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "dynamicBlendData".to_string(),
            type_name: "struct".to_string(),
            ctype: Some("hclBlendSomeVerticesOperator::DynamicBlendData".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclBlendSomeVerticesOperator::DynamicBlendData".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclBlendSomeVerticesOperator::BlendVertices".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hclBlendSomeVerticesOperator::DynamicBlendData", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "defaultWeight".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "transitionPeriod".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "mapToSCurve".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hclBlendSomeVerticesOperator::BlendVertices", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "vertexIndices".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "constBlendWeight".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBlendSetupObject", 1),
            ClassVersion::new("hclBlendSetupObject", 2),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "dynamicBlend".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "dynamicBlendTransitionPeriod".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "dynamicBlendDefaultWeight".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hclVirtualCollisionPointsData", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "blocks".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclVirtualCollisionPointsData::Block".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclVirtualCollisionPointsData::Block".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "numVCPoints".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "landscapeParticlesBlockIndex".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "numLandscapeVCPoints".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "edgeBarycentricsDictionary".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "edgeDictionaryEntries".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclVirtualCollisionPointsData::BarycentricDictionaryEntry".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclVirtualCollisionPointsData::BarycentricDictionaryEntry".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "triangleBarycentricsDictionary".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclVirtualCollisionPointsData::BarycentricPair".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclVirtualCollisionPointsData::BarycentricPair".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "triangleDictionaryEntries".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclVirtualCollisionPointsData::BarycentricDictionaryEntry".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "edges".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclVirtualCollisionPointsData::EdgeFanSection".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclVirtualCollisionPointsData::EdgeFanSection".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "edgeFans".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclVirtualCollisionPointsData::EdgeFan".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclVirtualCollisionPointsData::EdgeFan".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "triangles".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclVirtualCollisionPointsData::TriangleFanSection".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclVirtualCollisionPointsData::TriangleFanSection".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "triangleFans".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclVirtualCollisionPointsData::TriangleFan".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclVirtualCollisionPointsData::TriangleFan".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "edgesLandscape".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclVirtualCollisionPointsData::EdgeFanSection".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "edgeFansLandscape".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclVirtualCollisionPointsData::EdgeFanLandscape".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclVirtualCollisionPointsData::EdgeFanLandscape".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "trianglesLandscape".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclVirtualCollisionPointsData::TriangleFanSection".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "triangleFansLandscape".to_string(),
            type_name: "array".to_string(),
            ctype: Some("hclVirtualCollisionPointsData::TriangleFanLandscape".to_string()),
            default: None,
        })
        .with_operation(PatchOperation::Depends {
            class_name: "hclVirtualCollisionPointsData::TriangleFanLandscape".to_string(),
            version: 0,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "edgeFanIndices".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "triangleFanIndices".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "edgeFanIndicesLandscape".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "triangleFanIndicesLandscape".to_string(),
            type_name: "array".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hclVirtualCollisionPointsData::Block", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "safeDisplacementRadius".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "startingVCPIndex".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "numVCPs".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new(
                "hclVirtualCollisionPointsData::BarycentricDictionaryEntry",
                0,
            ),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "startingBarycentricIndex".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "numBarycentrics".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hclVirtualCollisionPointsData::TriangleFanSection", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "oppositeRealParticleIndices".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "barycentricDictionaryIndex".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hclVirtualCollisionPointsData::TriangleFan", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "realParticleIndex".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "vcpStartIndex".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "numTriangles".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hclVirtualCollisionPointsData::TriangleFanLandscape", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "realParticleIndex".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "triangleStartIndex".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "vcpStartIndex".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "numTriangles".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hclVirtualCollisionPointsData::EdgeFanSection", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "oppositeRealParticleIndex".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "barycentricDictionaryIndex".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hclVirtualCollisionPointsData::EdgeFan", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "realParticleIndex".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "edgeStartIndex".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "numEdges".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hclVirtualCollisionPointsData::EdgeFanLandscape", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "realParticleIndex".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "edgeStartIndex".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "vcpStartIndex".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "numEdges".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("", -1),
            ClassVersion::new("hclVirtualCollisionPointsData::BarycentricPair", 0),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "u".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "v".to_string(),
            type_name: "real".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclCollidable", 3),
            ClassVersion::new("hclCollidable", 4),
        )
        .with_operation(PatchOperation::MemberAdd {
            name: "virtualCollisionPointCollisionEnabled".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(0)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "enabled".to_string(),
            type_name: "int8".to_string(),
            ctype: None,
            default: Some(PatchValue::Int(1)),
        })
        .with_operation(PatchOperation::MemberAdd {
            name: "userData".to_string(),
            type_name: "int".to_string(),
            ctype: None,
            default: None,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBoneSpaceDeformer", 0),
            ClassVersion::new("hclBoneSpaceDeformer", 1),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "batchSizeSpu".to_string(),
            type_name: "int".to_string(),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclObjectSpaceDeformer", 1),
            ClassVersion::new("hclObjectSpaceDeformer", 2),
        )
        .with_operation(PatchOperation::MemberRemove {
            name: "batchSizeSpu".to_string(),
            type_name: "int".to_string(),
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBoneSpaceDeformer::LocalBlockPN", 0),
            ClassVersion::new("hclBoneSpaceDeformer::LocalBlockPN", 1),
        )
        .with_custom_hook("_noop_type_change")
        .with_operation(PatchOperation::Depends {
            class_name: "hkPackedVector3".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBoneSpaceDeformer::LocalBlockPNT", 0),
            ClassVersion::new("hclBoneSpaceDeformer::LocalBlockPNT", 1),
        )
        .with_custom_hook("_noop_type_change")
        .with_operation(PatchOperation::Depends {
            class_name: "hkPackedVector3".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclBoneSpaceDeformer::LocalBlockPNTB", 0),
            ClassVersion::new("hclBoneSpaceDeformer::LocalBlockPNTB", 1),
        )
        .with_custom_hook("_noop_type_change")
        .with_operation(PatchOperation::Depends {
            class_name: "hkPackedVector3".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclObjectSpaceDeformer::LocalBlockP", 0),
            ClassVersion::new("hclObjectSpaceDeformer::LocalBlockP", 1),
        )
        .with_custom_hook("_noop_type_change")
        .with_operation(PatchOperation::Depends {
            class_name: "hkPackedVector3".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclObjectSpaceDeformer::LocalBlockPN", 0),
            ClassVersion::new("hclObjectSpaceDeformer::LocalBlockPN", 1),
        )
        .with_custom_hook("_noop_type_change")
        .with_operation(PatchOperation::Depends {
            class_name: "hkPackedVector3".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclObjectSpaceDeformer::LocalBlockPNT", 0),
            ClassVersion::new("hclObjectSpaceDeformer::LocalBlockPNT", 1),
        )
        .with_custom_hook("_noop_type_change")
        .with_operation(PatchOperation::Depends {
            class_name: "hkPackedVector3".to_string(),
            version: 0,
        }),
    );
    manager.register(
        55,
        Patch::new(
            ClassVersion::new("hclObjectSpaceDeformer::LocalBlockPNTB", 0),
            ClassVersion::new("hclObjectSpaceDeformer::LocalBlockPNTB", 1),
        )
        .with_custom_hook("_noop_type_change")
        .with_operation(PatchOperation::Depends {
            class_name: "hkPackedVector3".to_string(),
            version: 0,
        }),
    );
}
