from __future__ import annotations
import io
from dataclasses import asdict, dataclass

from . import native_runtime
from .base import BaseHeader

BGEM_SIGNATURE = 0x4D454742  # 'BGEM'


def _header_from_payload(payload: dict) -> BaseHeader:
    return BaseHeader(**payload)


def _tuple3(value: list[float] | tuple[float, float, float] | None):
    if value is None:
        return None
    return (float(value[0]), float(value[1]), float(value[2]))


def _bgem_from_payload(payload: dict) -> "BGEMData":
    payload = dict(payload)
    payload["header"] = _header_from_payload(payload["header"])
    for name in ("GlassFresnelColor", "BaseColor", "EmittanceColor"):
        payload[name] = _tuple3(payload.get(name))
    return BGEMData(**payload)


def _payload_from_bgem(data: "BGEMData") -> dict:
    return asdict(data)


@dataclass
class BGEMData:
    header: BaseHeader
    BaseTexture: str
    GrayscaleTexture: str
    EnvmapTexture: str
    NormalTexture: str
    EnvmapMaskTexture: str
    SpecularTexture: str | None
    LightingTexture: str | None
    GlowTexture: str | None
    GlassRoughnessScratch: str | None
    GlassDirtOverlay: str | None
    GlassEnabled: bool | None
    GlassFresnelColor: tuple[float, float, float] | None
    GlassBlurScaleBase: float | None
    GlassBlurScaleFactor: float | None
    GlassRefractionScaleBase: float | None
    # version >= 10
    EnvironmentMapping: bool | None
    EnvironmentMappingMaskScale: float | None
    # flags and floats
    BloodEnabled: bool
    EffectLightingEnabled: bool
    FalloffEnabled: bool
    FalloffColorEnabled: bool
    GrayscaleToPaletteAlpha: bool
    SoftEnabled: bool
    BaseColor: tuple[float, float, float]
    BaseColorScale: float
    FalloffStartAngle: float
    FalloffStopAngle: float
    FalloffStartOpacity: float
    FalloffStopOpacity: float
    LightingInfluence: float
    EnvmapMinLOD: int
    SoftDepth: float
    EmittanceColor: tuple[float, float, float] | None
    AdaptativeEmissive_ExposureOffset: float | None
    AdaptativeEmissive_FinalExposureMin: float | None
    AdaptativeEmissive_FinalExposureMax: float | None
    Glowmap: bool | None
    EffectPbrSpecular: bool | None

    def write(self, bw: io.BufferedWriter) -> None:
        bw.write(native_runtime.write_bgem(_payload_from_bgem(self)))


def read_bgem(br: io.BufferedReader) -> BGEMData:
    return _bgem_from_payload(native_runtime.parse_bgem(br.read()))
