use std::error::Error;
use std::fmt;

const TYPE_TAG: u32 = 6;
const COLLECTION_TAG: &str = "CollectionAnimationSpeedContourName";
const INDIVIDUAL_TAG: &str = "IndividualClipAnimationSpeedContour";
const SPEED_SAMPLED_TAG: &str = "SpeedSampledAnimationSpeedContour";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum CenterMode {
    PiCentered = 0,
    ZeroCentered = 1,
}

impl TryFrom<u8> for CenterMode {
    type Error = ContourCodecError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::PiCentered),
            1 => Ok(Self::ZeroCentered),
            _ => Err(ContourCodecError::InvalidCenterMode(value)),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpeedInfoFile {
    pub roots: Vec<SpeedInfoRoot>,
}

impl SpeedInfoFile {
    pub fn stats(&self) -> ContourStats {
        let mut stats = ContourStats {
            roots: self.roots.len(),
            ..ContourStats::default()
        };
        for root in &self.roots {
            root.contour.add_stats(&mut stats);
        }
        stats
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpeedInfoRoot {
    pub state_machine_path: String,
    pub contour: Contour,
    pub metadata: RootMetadata,
}

impl SpeedInfoRoot {
    pub fn producer_metadata(&self) -> Option<&Entry> {
        match (&self.contour, &self.metadata) {
            (_, RootMetadata::Collection(metadata)) => Some(&metadata.producer),
            (Contour::Individual(individual), RootMetadata::DirectIndividual) => {
                Some(&individual.entry)
            }
            (Contour::SpeedSampled(sampled), RootMetadata::DirectSpeedSampled) => {
                Some(&sampled.entry)
            }
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RootMetadata {
    Collection(CollectionRootMetadata),
    DirectIndividual,
    DirectSpeedSampled,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CollectionRootMetadata {
    pub center_mode: CenterMode,
    pub producer: Entry,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Contour {
    Collection(CollectionContour),
    Individual(IndividualContour),
    SpeedSampled(SpeedSampledContour),
}

impl Contour {
    fn add_stats(&self, stats: &mut ContourStats) {
        match self {
            Self::Collection(collection) => {
                stats.collections += 1;
                for child in &collection.children {
                    child.add_stats(stats);
                }
            }
            Self::Individual(_) => stats.individuals += 1,
            Self::SpeedSampled(_) => stats.speed_sampled += 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CollectionContour {
    pub children: Vec<Contour>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IndividualContour {
    pub direction: [f32; 3],
    pub parameter: String,
    pub speed: f32,
    pub clip: String,
    pub condition: String,
    pub entry: Entry,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpeedSampledContour {
    pub capacity_hint: u32,
    pub angle_min: f32,
    pub angle_max: f32,
    pub speed_min: f32,
    pub speed_max: f32,
    pub curves: Vec<DirectionCurve>,
    pub directional_summary: Vec<SamplePair>,
    pub center_mode: CenterMode,
    pub clip: String,
    pub condition: String,
    pub entry: Entry,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DirectionCurve {
    pub angle: f32,
    pub samples: Vec<SamplePair>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SamplePair {
    pub input: f32,
    pub output: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub state_id: i32,
    pub value: f32,
    pub link: Option<EntryLink>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EntryLink {
    pub event: String,
    pub variable: String,
    pub next: Box<Entry>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ContourStats {
    pub roots: usize,
    pub collections: usize,
    pub individuals: usize,
    pub speed_sampled: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContourCodecError {
    UnexpectedEof { offset: usize, needed: usize },
    InvalidTypeTag(u32),
    InvalidUtf8 { offset: usize },
    MissingStringTerminator { offset: usize },
    StringTooLong(usize),
    UnknownContourTag(String),
    InvalidCenterMode(u8),
    InvalidEntryLink { offset: usize },
    InvalidRootMetadata { root: usize, reason: &'static str },
    CountOverflow(&'static str),
    TrailingBytes { offset: usize, count: usize },
}

impl fmt::Display for ContourCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof { offset, needed } => {
                write!(f, "unexpected EOF at {offset}, need {needed} bytes")
            }
            Self::InvalidTypeTag(tag) => write!(f, "invalid SpeedInfo type tag {tag}"),
            Self::InvalidUtf8 { offset } => write!(f, "invalid UTF-8 string at {offset}"),
            Self::MissingStringTerminator { offset } => {
                write!(f, "missing string terminator at {offset}")
            }
            Self::StringTooLong(len) => write!(f, "SpeedInfo string is too long: {len} bytes"),
            Self::UnknownContourTag(tag) => write!(f, "unknown contour tag {tag:?}"),
            Self::InvalidCenterMode(mode) => write!(f, "invalid center_mode {mode}"),
            Self::InvalidEntryLink { offset } => {
                write!(f, "entry link at {offset} must name an event and variable")
            }
            Self::InvalidRootMetadata { root, reason } => {
                write!(f, "invalid metadata for root {root}: {reason}")
            }
            Self::CountOverflow(kind) => write!(f, "too many {kind} to encode as u32"),
            Self::TrailingBytes { offset, count } => {
                write!(f, "{count} trailing bytes after offset {offset}")
            }
        }
    }
}

impl Error for ContourCodecError {}

pub fn decode_speed_info(bytes: &[u8]) -> Result<SpeedInfoFile, ContourCodecError> {
    let mut decoder = Decoder::new(bytes);
    let tag = decoder.u32()?;
    if tag != TYPE_TAG {
        return Err(ContourCodecError::InvalidTypeTag(tag));
    }
    let root_count = decoder.u32()? as usize;
    let mut roots = Vec::with_capacity(root_count);
    for root_index in 0..root_count {
        let state_machine_path = decoder.string()?;
        let contour = decoder.contour(EntryContext::RootMetadata)?;
        let metadata = match &contour {
            Contour::Collection(_) => {
                let center_mode = CenterMode::try_from(decoder.u8()?)?;
                if !decoder.string()?.is_empty() || !decoder.string()?.is_empty() {
                    return Err(ContourCodecError::InvalidRootMetadata {
                        root: root_index,
                        reason: "Collection root clip and condition must be empty",
                    });
                }
                let producer = decoder.entry(EntryContext::RootMetadata)?;
                validate_producer_entry(root_index, &producer)?;
                RootMetadata::Collection(CollectionRootMetadata {
                    center_mode,
                    producer,
                })
            }
            Contour::Individual(individual) => {
                validate_producer_entry(root_index, &individual.entry)?;
                RootMetadata::DirectIndividual
            }
            Contour::SpeedSampled(sampled) => {
                validate_producer_entry(root_index, &sampled.entry)?;
                RootMetadata::DirectSpeedSampled
            }
        };
        roots.push(SpeedInfoRoot {
            state_machine_path,
            contour,
            metadata,
        });
    }
    if decoder.position != bytes.len() {
        return Err(ContourCodecError::TrailingBytes {
            offset: decoder.position,
            count: bytes.len() - decoder.position,
        });
    }
    Ok(SpeedInfoFile { roots })
}

pub fn encode_speed_info(file: &SpeedInfoFile) -> Result<Vec<u8>, ContourCodecError> {
    let mut out = Vec::new();
    out.extend_from_slice(&TYPE_TAG.to_le_bytes());
    push_count(&mut out, file.roots.len(), "roots")?;
    for (root_index, root) in file.roots.iter().enumerate() {
        push_string(&mut out, &root.state_machine_path)?;
        encode_contour(&mut out, &root.contour)?;
        match (&root.contour, &root.metadata) {
            (Contour::Collection(_), RootMetadata::Collection(metadata)) => {
                validate_producer_entry(root_index, &metadata.producer)?;
                out.push(metadata.center_mode as u8);
                push_string(&mut out, "")?;
                push_string(&mut out, "")?;
                encode_entry(&mut out, &metadata.producer)?;
            }
            (Contour::Individual(individual), RootMetadata::DirectIndividual) => {
                validate_producer_entry(root_index, &individual.entry)?;
            }
            (Contour::SpeedSampled(sampled), RootMetadata::DirectSpeedSampled) => {
                validate_producer_entry(root_index, &sampled.entry)?;
            }
            _ => {
                return Err(ContourCodecError::InvalidRootMetadata {
                    root: root_index,
                    reason: "metadata context does not match the root contour class",
                });
            }
        }
    }
    Ok(out)
}

fn validate_producer_entry(root: usize, entry: &Entry) -> Result<(), ContourCodecError> {
    if entry.state_id != -1 {
        return Err(ContourCodecError::InvalidRootMetadata {
            root,
            reason: "producer state_id must be -1",
        });
    }
    if entry.link.is_some() {
        return Err(ContourCodecError::InvalidRootMetadata {
            root,
            reason: "producer entry must be terminal",
        });
    }
    Ok(())
}

fn encode_contour(out: &mut Vec<u8>, contour: &Contour) -> Result<(), ContourCodecError> {
    match contour {
        Contour::Collection(collection) => {
            push_string(out, COLLECTION_TAG)?;
            push_count(out, collection.children.len(), "Collection children")?;
            for child in &collection.children {
                encode_contour(out, child)?;
            }
        }
        Contour::Individual(individual) => {
            push_string(out, INDIVIDUAL_TAG)?;
            for value in individual.direction {
                out.extend_from_slice(&value.to_le_bytes());
            }
            push_string(out, &individual.parameter)?;
            out.extend_from_slice(&individual.speed.to_le_bytes());
            push_string(out, &individual.clip)?;
            push_string(out, &individual.condition)?;
            encode_entry(out, &individual.entry)?;
        }
        Contour::SpeedSampled(sampled) => {
            push_string(out, SPEED_SAMPLED_TAG)?;
            out.extend_from_slice(&sampled.capacity_hint.to_le_bytes());
            out.extend_from_slice(&sampled.angle_min.to_le_bytes());
            out.extend_from_slice(&sampled.angle_max.to_le_bytes());
            out.extend_from_slice(&sampled.speed_min.to_le_bytes());
            out.extend_from_slice(&sampled.speed_max.to_le_bytes());
            push_count(out, sampled.curves.len(), "SpeedSampled curves")?;
            for curve in &sampled.curves {
                out.extend_from_slice(&curve.angle.to_le_bytes());
                push_count(out, curve.samples.len(), "SpeedSampled samples")?;
                for sample in &curve.samples {
                    encode_sample(out, sample);
                }
            }
            push_count(
                out,
                sampled.directional_summary.len(),
                "SpeedSampled directional summary samples",
            )?;
            for sample in &sampled.directional_summary {
                encode_sample(out, sample);
            }
            out.push(sampled.center_mode as u8);
            push_string(out, &sampled.clip)?;
            push_string(out, &sampled.condition)?;
            encode_entry(out, &sampled.entry)?;
        }
    }
    Ok(())
}

fn encode_sample(out: &mut Vec<u8>, sample: &SamplePair) {
    out.extend_from_slice(&sample.input.to_le_bytes());
    out.extend_from_slice(&sample.output.to_le_bytes());
}

fn encode_entry(out: &mut Vec<u8>, entry: &Entry) -> Result<(), ContourCodecError> {
    out.extend_from_slice(&entry.state_id.to_le_bytes());
    out.extend_from_slice(&entry.value.to_le_bytes());
    if let Some(link) = &entry.link {
        if link.event.is_empty() || link.variable.is_empty() {
            return Err(ContourCodecError::InvalidEntryLink { offset: out.len() });
        }
        out.push(1);
        push_string(out, &link.event)?;
        push_string(out, &link.variable)?;
        encode_entry(out, &link.next)?;
    }
    Ok(())
}

fn push_count(
    out: &mut Vec<u8>,
    count: usize,
    kind: &'static str,
) -> Result<(), ContourCodecError> {
    let count = u32::try_from(count).map_err(|_| ContourCodecError::CountOverflow(kind))?;
    out.extend_from_slice(&count.to_le_bytes());
    Ok(())
}

fn push_string(out: &mut Vec<u8>, value: &str) -> Result<(), ContourCodecError> {
    if value.is_empty() {
        out.push(0);
        return Ok(());
    }
    let encoded_len = value
        .len()
        .checked_add(1)
        .filter(|length| *length <= u8::MAX as usize)
        .ok_or(ContourCodecError::StringTooLong(value.len()))?;
    out.push(encoded_len as u8);
    out.extend_from_slice(value.as_bytes());
    out.push(0);
    Ok(())
}

struct Decoder<'a> {
    bytes: &'a [u8],
    position: usize,
}

#[derive(Clone, Copy)]
enum EntryContext {
    Selector,
    RootMetadata,
}

impl<'a> Decoder<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], ContourCodecError> {
        let end = self
            .position
            .checked_add(count)
            .ok_or(ContourCodecError::UnexpectedEof {
                offset: self.position,
                needed: count,
            })?;
        let bytes = self
            .bytes
            .get(self.position..end)
            .ok_or(ContourCodecError::UnexpectedEof {
                offset: self.position,
                needed: count,
            })?;
        self.position = end;
        Ok(bytes)
    }

    fn u8(&mut self) -> Result<u8, ContourCodecError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, ContourCodecError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn i32(&mut self) -> Result<i32, ContourCodecError> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn f32(&mut self) -> Result<f32, ContourCodecError> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn string(&mut self) -> Result<String, ContourCodecError> {
        let start = self.position;
        let encoded_len = self.u8()? as usize;
        if encoded_len == 0 {
            return Ok(String::new());
        }
        let encoded = self.take(encoded_len)?;
        if encoded.last() != Some(&0) {
            return Err(ContourCodecError::MissingStringTerminator { offset: start });
        }
        std::str::from_utf8(&encoded[..encoded.len() - 1])
            .map(str::to_owned)
            .map_err(|_| ContourCodecError::InvalidUtf8 { offset: start })
    }

    fn contour(&mut self, entry_context: EntryContext) -> Result<Contour, ContourCodecError> {
        let tag = self.string()?;
        match tag.as_str() {
            COLLECTION_TAG => {
                let count = self.u32()? as usize;
                let mut children = Vec::with_capacity(count);
                for _ in 0..count {
                    children.push(self.contour(EntryContext::Selector)?);
                }
                Ok(Contour::Collection(CollectionContour { children }))
            }
            INDIVIDUAL_TAG => Ok(Contour::Individual(IndividualContour {
                direction: [self.f32()?, self.f32()?, self.f32()?],
                parameter: self.string()?,
                speed: self.f32()?,
                clip: self.string()?,
                condition: self.string()?,
                entry: self.entry(entry_context)?,
            })),
            SPEED_SAMPLED_TAG => {
                let capacity_hint = self.u32()?;
                let angle_min = self.f32()?;
                let angle_max = self.f32()?;
                let speed_min = self.f32()?;
                let speed_max = self.f32()?;
                let curve_count = self.u32()? as usize;
                let mut curves = Vec::with_capacity(curve_count);
                for _ in 0..curve_count {
                    let angle = self.f32()?;
                    let sample_count = self.u32()? as usize;
                    let mut samples = Vec::with_capacity(sample_count);
                    for _ in 0..sample_count {
                        samples.push(self.sample()?);
                    }
                    curves.push(DirectionCurve { angle, samples });
                }
                let summary_count = self.u32()? as usize;
                let mut directional_summary = Vec::with_capacity(summary_count);
                for _ in 0..summary_count {
                    directional_summary.push(self.sample()?);
                }
                Ok(Contour::SpeedSampled(SpeedSampledContour {
                    capacity_hint,
                    angle_min,
                    angle_max,
                    speed_min,
                    speed_max,
                    curves,
                    directional_summary,
                    center_mode: CenterMode::try_from(self.u8()?)?,
                    clip: self.string()?,
                    condition: self.string()?,
                    entry: self.entry(entry_context)?,
                }))
            }
            _ => Err(ContourCodecError::UnknownContourTag(tag)),
        }
    }

    fn sample(&mut self) -> Result<SamplePair, ContourCodecError> {
        Ok(SamplePair {
            input: self.f32()?,
            output: self.f32()?,
        })
    }

    fn entry(&mut self, context: EntryContext) -> Result<Entry, ContourCodecError> {
        let state_id = self.i32()?;
        let value = self.f32()?;
        let link = if matches!(context, EntryContext::Selector)
            && self.bytes.get(self.position) == Some(&1)
        {
            let checkpoint = self.position;
            self.position += 1;
            let event = self.string()?;
            let variable = self.string()?;
            if event.is_empty() && variable.is_empty() {
                self.position = checkpoint;
                None
            } else if event.is_empty() || variable.is_empty() {
                return Err(ContourCodecError::InvalidEntryLink { offset: checkpoint });
            } else {
                Some(EntryLink {
                    event,
                    variable,
                    next: Box::new(self.entry(EntryContext::Selector)?),
                })
            }
        } else {
            None
        };
        Ok(Entry {
            state_id,
            value,
            link,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn producer(value: f32) -> Entry {
        Entry {
            state_id: -1,
            value,
            link: None,
        }
    }

    fn individual(entry: Entry) -> IndividualContour {
        IndividualContour {
            direction: [0.0, 1.0, 0.0],
            parameter: String::new(),
            speed: 100.0,
            clip: String::new(),
            condition: String::new(),
            entry,
        }
    }

    #[test]
    fn mode_one_collection_metadata_is_not_an_entry_link() {
        let file = SpeedInfoFile {
            roots: vec![SpeedInfoRoot {
                state_machine_path: "WeaponBehavior.hkb/direct".to_string(),
                contour: Contour::Collection(CollectionContour {
                    children: vec![Contour::Individual(individual(producer(0.0)))],
                }),
                metadata: RootMetadata::Collection(CollectionRootMetadata {
                    center_mode: CenterMode::ZeroCentered,
                    producer: producer(0.108),
                }),
            }],
        };
        let encoded = encode_speed_info(&file).unwrap();
        let decoded = decode_speed_info(&encoded).unwrap();
        assert_eq!(decoded, file);
    }

    #[test]
    fn direct_individual_owns_its_producer_metadata_entry() {
        let file = SpeedInfoFile {
            roots: vec![SpeedInfoRoot {
                state_machine_path: "WeaponBehavior.hkb/WPNLanding_SM".to_string(),
                contour: Contour::Individual(individual(producer(1.0 / 30.0))),
                metadata: RootMetadata::DirectIndividual,
            }],
        };
        let encoded = encode_speed_info(&file).unwrap();
        let decoded = decode_speed_info(&encoded).unwrap();
        assert_eq!(decoded, file);
        assert_eq!(
            decoded.roots[0].producer_metadata(),
            Some(&producer(1.0 / 30.0))
        );
    }

    #[test]
    fn root_metadata_entries_are_terminal_at_file_boundaries() {
        let file = SpeedInfoFile {
            roots: vec![SpeedInfoRoot {
                state_machine_path: "root".to_string(),
                contour: Contour::Individual(individual(producer(f32::from_bits(1)))),
                metadata: RootMetadata::DirectIndividual,
            }],
        };
        let mut encoded = encode_speed_info(&file).unwrap();
        let boundary = encoded.len();
        encoded.push(1);
        assert_eq!(
            decode_speed_info(&encoded),
            Err(ContourCodecError::TrailingBytes {
                offset: boundary,
                count: 1,
            })
        );
    }

    #[test]
    fn root_metadata_value_bits_round_trip_without_link_lookahead() {
        let boundary_values = [
            0,
            1,
            u32::MAX,
            f32::INFINITY.to_bits(),
            f32::NEG_INFINITY.to_bits(),
            f32::NAN.to_bits(),
        ];
        for bits in boundary_values
            .into_iter()
            .chain((0..4096_u32).map(|value| value.wrapping_mul(0x9E37_79B9)))
        {
            let file = SpeedInfoFile {
                roots: vec![SpeedInfoRoot {
                    state_machine_path: "root".to_string(),
                    contour: Contour::Collection(CollectionContour {
                        children: vec![Contour::Individual(individual(producer(0.0)))],
                    }),
                    metadata: RootMetadata::Collection(CollectionRootMetadata {
                        center_mode: CenterMode::PiCentered,
                        producer: producer(f32::from_bits(bits)),
                    }),
                }],
            };
            let decoded = decode_speed_info(&encode_speed_info(&file).unwrap()).unwrap();
            assert_eq!(
                decoded.roots[0]
                    .producer_metadata()
                    .unwrap()
                    .value
                    .to_bits(),
                bits
            );
        }
    }

}
