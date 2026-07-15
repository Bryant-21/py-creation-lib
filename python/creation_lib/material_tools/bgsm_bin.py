from __future__ import annotations
import io
from dataclasses import asdict, dataclass

from . import native_runtime
from .base import BaseHeader

BGSM_SIGNATURE = 0x4D534742  # 'BGSM'


def _header_from_payload(payload: dict) -> BaseHeader:
    return BaseHeader(**payload)


def _tuple3(value: list[float] | tuple[float, float, float] | None):
    if value is None:
        return None
    return (float(value[0]), float(value[1]), float(value[2]))


def _bgsm_from_payload(payload: dict) -> "BGSMData":
    payload = dict(payload)
    payload["header"] = _header_from_payload(payload["header"])
    for name in (
        "TranslucencySubsurfaceColor",
        "SpecularColor",
        "EmittanceColor",
        "HairTintColor",
    ):
        payload[name] = _tuple3(payload.get(name))
    return BGSMData(**payload)


def _payload_from_bgsm(data: "BGSMData") -> dict:
    return asdict(data)


@dataclass
class BGSMData:
    header: BaseHeader
    # strings
    DiffuseTexture: str
    NormalTexture: str
    SmoothSpecTexture: str
    GreyscaleTexture: str
    EnvmapTexture: str | None
    GlowTexture: str | None
    InnerLayerTexture: str | None
    WrinklesTexture: str | None
    DisplacementTexture: str | None
    SpecularTexture: str | None
    LightingTexture: str | None
    FlowTexture: str | None
    DistanceFieldAlphaTexture: str | None
    # rest of fields (subset sufficient for round-trip)
    EnableEditorAlphaRef: bool
    # v>=8 block or v<8 block
    RimLighting: bool | None
    RimPower: float | None
    BackLightPower: float | None
    SubsurfaceLighting: bool | None
    SubsurfaceLightingRolloff: float | None
    Translucency: bool | None
    TranslucencyThickObject: bool | None
    TranslucencyMixAlbedoWithSubsurfaceColor: bool | None
    TranslucencySubsurfaceColor: tuple[float, float, float] | None
    TranslucencyTransmissiveScale: float | None
    TranslucencyTurbulence: float | None
    # spec / smoothness etc.
    SpecularEnabled: bool
    SpecularColor: tuple[float, float, float]
    SpecularMult: float
    Smoothness: float
    FresnelPower: float
    WetnessControlSpecScale: float
    WetnessControlSpecPowerScale: float
    WetnessControlSpecMinvar: float
    WetnessControlEnvMapScale: float | None
    WetnessControlFresnelPower: float
    WetnessControlMetalness: float
    # PBR and porosity
    PBR: bool | None
    CustomPorosity: bool | None
    PorosityValue: float | None
    RootMaterialPath: str
    AnisoLighting: bool
    EmitEnabled: bool
    EmittanceColor: tuple[float, float, float] | None
    EmittanceMult: float
    ModelSpaceNormals: bool
    ExternalEmittance: bool
    LumEmittance: float | None
    UseAdaptativeEmissive: bool | None
    AdaptativeEmissive_ExposureOffset: float | None
    AdaptativeEmissive_FinalExposureMin: float | None
    AdaptativeEmissive_FinalExposureMax: float | None
    BackLighting: bool | None
    ReceiveShadows: bool
    HideSecret: bool
    CastShadows: bool
    DissolveFade: bool
    AssumeShadowmask: bool
    Glowmap: bool
    EnvironmentMappingWindow: bool | None
    EnvironmentMappingEye: bool | None
    Hair: bool
    HairTintColor: tuple[float, float, float]
    Tree: bool
    Facegen: bool
    SkinTint: bool
    Tessellate: bool
    DisplacementTextureBias: float | None
    DisplacementTextureScale: float | None
    TessellationPnScale: float | None
    TessellationBaseFactor: float | None
    TessellationFadeDistance: float | None
    GrayscaleToPaletteScale: float
    SkewSpecularAlpha: bool | None
    Terrain: bool | None
    UnkInt1: int | None
    TerrainThresholdFalloff: float | None
    TerrainTilingDistance: float | None
    TerrainRotationAngle: float | None

    def write(self, bw: io.BufferedWriter) -> None:
        bw.write(native_runtime.write_bgsm(_payload_from_bgsm(self)))


def read_bgsm(br: io.BufferedReader) -> BGSMData:
    return _bgsm_from_payload(native_runtime.parse_bgsm(br.read()))
