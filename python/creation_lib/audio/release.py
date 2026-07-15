"""Release audio processing: LIP/FUZ generation, WAV->XWM conversion.

Pipeline for preparing audio files for mod distribution:
- Voice WAVs -> LIP (lip-sync) + XWM (compressed) -> FUZ (combined)
- SFX WAVs -> XWM (compressed), skipping files with loop/cue markers

Tool binaries live in creation_lib resources and are invoked via subprocess.
"""

from __future__ import annotations

import glob
import logging
import os
import subprocess
import yaml

from creation_lib.audio import has_loop_or_cue_markers

_log = logging.getLogger("creation_lib.audio.release")

_CREATE_NO_WINDOW = 0x08000000 if os.name == "nt" else 0


def _tool_paths(resource_dir: str | os.PathLike[str]) -> dict[str, str]:
    resource = os.fspath(resource_dir)
    if not os.path.isfile(os.path.join(resource, "xWMAEncode.exe")):
        from creation_lib.paths import get_resource_dir

        resource = os.fspath(get_resource_dir())
    lipgen_dir = os.path.join(resource, "fo4_lipgen")
    return {
        "facefx": os.path.join(lipgen_dir, "FaceFXWrapper.exe"),
        "fonix_cdf": os.path.join(lipgen_dir, "FonixData.cdf"),
        "xwma": os.path.join(resource, "xWMAEncode.exe"),
        "bmlfuz": os.path.join(resource, "BmlFuzEncode.exe"),
    }


# ---------------------------------------------------------------------------
# Low-level tool wrappers
# ---------------------------------------------------------------------------

def create_lip(wav_path: str, lip_path: str, transcript: str,
               ffmpeg_path: str = "ffmpeg", game: str = "Fallout4",
               language: str = "USEnglish",
               resource_dir: str | os.PathLike[str] | None = None) -> bool:
    """Generate LIP from WAV + transcript text via FaceFXWrapper.

    Resamples WAV to 16kHz mono PCM16 (required by FaceFX), then invokes
    FaceFXWrapper with the transcript text. Returns True on success.
    """
    if not transcript or not transcript.strip():
        _log.warning("Empty transcript for %s, skipping LIP", os.path.basename(wav_path))
        return False
    if resource_dir is None:
        raise ValueError("resource_dir is required")
    tools = _tool_paths(resource_dir)

    rs_wav = None
    try:
        # Resample to 16kHz mono for FaceFX
        rs_wav = wav_path.replace(".wav", "_16000.wav")
        cmd = [
            ffmpeg_path, "-y", "-i", wav_path,
            "-ar", "16000", "-ac", "1", "-sample_fmt", "s16",
            rs_wav,
        ]
        proc = subprocess.run(cmd, capture_output=True, creationflags=_CREATE_NO_WINDOW)
        if proc.returncode != 0:
            _log.error("ffmpeg resample failed for %s: %s",
                       os.path.basename(wav_path), proc.stderr.decode(errors="replace"))
            return False

        # FaceFXWrapper.exe <game> <language> <cdf> <wav> <lip> <transcript>
        cmd = [
            tools["facefx"], game, language,
            os.path.abspath(tools["fonix_cdf"]),
            os.path.abspath(rs_wav),
            os.path.abspath(lip_path),
            transcript,
        ]
        proc = subprocess.run(cmd, capture_output=True, creationflags=_CREATE_NO_WINDOW)
        if proc.returncode != 0:
            _log.error("FaceFXWrapper failed for %s: %s",
                       os.path.basename(wav_path), proc.stderr.decode(errors="replace"))
            return False

        _log.debug("LIP created: %s", lip_path)
        return True

    except Exception as e:
        _log.error("create_lip error for %s: %s", os.path.basename(wav_path), e)
        return False
    finally:
        if rs_wav and os.path.exists(rs_wav):
            os.remove(rs_wav)


def create_xwm(
    wav_path: str,
    xwm_path: str,
    *,
    resource_dir: str | os.PathLike[str] | None = None,
) -> bool:
    """Encode WAV to XWM via xWMAEncode. Returns True on success."""
    if resource_dir is None:
        raise ValueError("resource_dir is required")
    cmd = [_tool_paths(resource_dir)["xwma"], "-b", "32000", wav_path, xwm_path]
    try:
        proc = subprocess.run(cmd, capture_output=True, creationflags=_CREATE_NO_WINDOW)
        if proc.returncode != 0:
            _log.error("xWMAEncode failed for %s: %s",
                       os.path.basename(wav_path), proc.stderr.decode(errors="replace"))
            return False
        _log.debug("XWM created: %s", xwm_path)
        return True
    except Exception as e:
        _log.error("create_xwm error for %s: %s", os.path.basename(wav_path), e)
        return False


def create_fuz(
    fuz_path: str,
    xwm_path: str,
    lip_path: str,
    *,
    resource_dir: str | os.PathLike[str] | None = None,
) -> bool:
    """Combine XWM + LIP into FUZ via BmlFuzEncode. Returns True on success."""
    if resource_dir is None:
        raise ValueError("resource_dir is required")
    cmd = [_tool_paths(resource_dir)["bmlfuz"], fuz_path, xwm_path, lip_path]
    try:
        proc = subprocess.run(cmd, capture_output=True, creationflags=_CREATE_NO_WINDOW)
        if proc.returncode != 0:
            _log.error("BmlFuzEncode failed for %s: %s",
                       os.path.basename(fuz_path), proc.stderr.decode(errors="replace"))
            return False
        _log.debug("FUZ created: %s", fuz_path)
        return True
    except Exception as e:
        _log.error("create_fuz error for %s: %s", os.path.basename(fuz_path), e)
        return False


# ---------------------------------------------------------------------------
# High-level pipelines
# ---------------------------------------------------------------------------

def process_voice_wav(wav_path: str, transcript: str,
                      ffmpeg_path: str = "ffmpeg",
                      cleanup: bool = True,
                      resource_dir: str | os.PathLike[str] | None = None) -> str | None:
    """Full voice pipeline: WAV -> LIP + XWM -> FUZ.

    Returns FUZ path on success, None on failure.
    Intermediate LIP/XWM files are cleaned up if cleanup=True.
    """
    base = wav_path.rsplit(".", 1)[0]
    lip_path = base + ".lip"
    xwm_path = base + ".xwm"
    fuz_path = base + ".fuz"
    if resource_dir is None:
        raise ValueError("resource_dir is required")

    try:
        # Step 1: Generate LIP from WAV + transcript
        if not create_lip(
            wav_path,
            lip_path,
            transcript,
            ffmpeg_path=ffmpeg_path,
            resource_dir=resource_dir,
        ):
            return None

        # Step 2: Encode WAV to XWM (needs 44.1kHz source — original WAV is fine)
        if not create_xwm(wav_path, xwm_path, resource_dir=resource_dir):
            return None

        # Step 3: Combine XWM + LIP -> FUZ
        if not create_fuz(fuz_path, xwm_path, lip_path, resource_dir=resource_dir):
            return None

        _log.info("FUZ created: %s", os.path.basename(fuz_path))
        return fuz_path

    finally:
        if cleanup:
            for f in (lip_path, xwm_path):
                if os.path.exists(f):
                    os.remove(f)


def process_sfx_wav(
    wav_path: str,
    *,
    resource_dir: str | os.PathLike[str] | None = None,
) -> str | None:
    """Convert non-voice WAV to XWM. Skips files with loop/cue markers.

    Returns XWM path on success, None if skipped or failed.
    """
    if has_loop_or_cue_markers(wav_path):
        _log.info("Skipping (has markers): %s", os.path.basename(wav_path))
        return None
    if resource_dir is None:
        raise ValueError("resource_dir is required")

    xwm_path = wav_path.rsplit(".", 1)[0] + ".xwm"
    if create_xwm(wav_path, xwm_path, resource_dir=resource_dir):
        _log.info("XWM created: %s", os.path.basename(xwm_path))
        return xwm_path
    return None


# ---------------------------------------------------------------------------
# Transcript map from YAML
# ---------------------------------------------------------------------------

def build_transcript_map(yaml_dir: str, plugin_name: str) -> dict[str, str]:
    """Walk Quest YAML tree to build {wav_filename_stem: transcript_text} map.

    Scans yaml_dir/Quests/*/DialogTopics/*/Responses/*.yaml for dialogue
    response records containing transcript text in the Responses[].Text field.

    The WAV filename stem is derived from the response record's FormKey hex
    and the response number: <FormKeyHex>_<ResponseNumber>

    Returns dict mapping lowercase filename stems to transcript text.
    """
    transcript_map: dict[str, str] = {}

    # Glob for all response YAML files under Quests/*/DialogTopics/*/Responses/
    pattern = os.path.join(yaml_dir, "Quests", "*", "DialogTopics", "*",
                           "Responses", "*.yaml")
    for yaml_path in glob.glob(pattern):
        try:
            with open(yaml_path, "r", encoding="utf-8") as f:
                data = yaml.safe_load(f)
            if not data or not isinstance(data, dict):
                continue

            form_key = data.get("FormKey", "")
            if not form_key:
                continue

            # Extract hex portion from FormKey (e.g. "10B686:Fallout4.esm" -> "10B686")
            fk_hex = form_key.split(":")[0] if ":" in form_key else form_key

            responses = data.get("Responses", [])
            if not responses or not isinstance(responses, list):
                continue

            for resp in responses:
                if not isinstance(resp, dict):
                    continue
                resp_num = resp.get("ResponseNumber", 1)
                text_field = resp.get("Text", "")

                # Text can be a plain string or a dict with TargetLanguage/String
                if isinstance(text_field, dict):
                    text = text_field.get("String", text_field.get("TargetLanguage", ""))
                elif isinstance(text_field, str):
                    text = text_field
                else:
                    text = ""

                if text:
                    stem = f"{fk_hex}_{resp_num}".lower()
                    transcript_map[stem] = text

        except Exception as e:
            _log.debug("Failed to parse %s: %s", yaml_path, e)

    _log.info("Built transcript map: %d entries from %s", len(transcript_map), yaml_dir)
    return transcript_map


# ---------------------------------------------------------------------------
# File discovery
# ---------------------------------------------------------------------------

def find_voice_wavs(mod_dir: str, plugin_name: str) -> list[str]:
    """Find all WAV files under mods/<Mod>/data/Sound/Voice/<plugin>/"""
    voice_dir = os.path.join(mod_dir, "data", "Sound", "Voice", plugin_name)
    if not os.path.isdir(voice_dir):
        return []
    wavs = glob.glob(os.path.join(voice_dir, "**", "*.wav"), recursive=True)
    return sorted(wavs)


def find_sfx_wavs(mod_dir: str) -> list[str]:
    """Find all WAV files under mods/<Mod>/data/Sound/ EXCLUDING Voice/ subdir."""
    sound_dir = os.path.join(mod_dir, "data", "Sound")
    if not os.path.isdir(sound_dir):
        return []
    all_wavs = glob.glob(os.path.join(sound_dir, "**", "*.wav"), recursive=True)
    voice_prefix = os.path.normcase(os.path.join(sound_dir, "Voice"))
    return sorted(w for w in all_wavs if not os.path.normcase(w).startswith(voice_prefix))


# ---------------------------------------------------------------------------
# Transcription fallback
# ---------------------------------------------------------------------------

_transcription_model = None
_transcription_type = None


def transcribe_wav(wav_path: str, fallback: str = "none") -> str | None:
    """Transcribe a WAV file using the configured fallback model.

    Args:
        wav_path: Path to WAV file.
        fallback: "none", "parakeet", or "whisper".

    Returns:
        Transcript text or None if fallback is disabled or fails.
    """
    global _transcription_model, _transcription_type

    if fallback == "none":
        return None

    try:
        if fallback == "parakeet":
            return _transcribe_parakeet(wav_path)
        elif fallback == "whisper":
            return _transcribe_whisper(wav_path)
        else:
            _log.warning("Unknown transcription fallback: %s", fallback)
            return None
    except Exception as e:
        _log.error("Transcription failed for %s: %s", os.path.basename(wav_path), e)
        return None


def _transcribe_parakeet(wav_path: str) -> str | None:
    """Transcribe using nvidia/parakeet-tdt-0.6b-v3."""
    global _transcription_model, _transcription_type
    try:
        import nemo.collections.asr as nemo_asr
    except ImportError:
        _log.error("nemo_toolkit[asr] not installed. Run: uv add 'nemo_toolkit[asr]'")
        return None

    if _transcription_type != "parakeet" or _transcription_model is None:
        _log.info("Loading Parakeet model (first use)...")
        _transcription_model = nemo_asr.models.ASRModel.from_pretrained(
            "nvidia/parakeet-tdt-0.6b-v3"
        )
        _transcription_type = "parakeet"

    result = _transcription_model.transcribe([wav_path])
    if result and len(result) > 0:
        # NeMo returns list of transcriptions
        text = result[0] if isinstance(result[0], str) else str(result[0])
        return text.strip()
    return None


def _transcribe_whisper(wav_path: str) -> str | None:
    """Transcribe using OpenAI Whisper."""
    global _transcription_model, _transcription_type
    try:
        import whisper
    except ImportError:
        _log.error("openai-whisper not installed. Run: uv add openai-whisper")
        return None

    if _transcription_type != "whisper" or _transcription_model is None:
        _log.info("Loading Whisper model (first use)...")
        _transcription_model = whisper.load_model("base")
        _transcription_type = "whisper"

    result = _transcription_model.transcribe(wav_path)
    text = result.get("text", "")
    return text.strip() if text else None
