//! Authored compiled-movie rendering. Never executes or rewrites ActionScript.
use crate::render::{Draw, bitmap_image, shader, shape_draws, transform};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use swf::{ColorTransform, Matrix, PlaceObjectAction, Shape, ShapeRecord, Tag, Twips};
use tiny_skia::{FillRule, Mask, MaskType, Paint, Pixmap, Transform};

fn swf_string(value: &swf::SwfStr) -> String {
    String::from_utf8_lossy(value.as_bytes()).into_owned()
}

fn html_attributes(value: &str, element: &str) -> HashMap<String, String> {
    let lower = value.to_ascii_lowercase();
    let Some(start) = lower.find(&format!("<{element}")) else {
        return HashMap::new();
    };
    let Some(end) = value[start..].find('>') else {
        return HashMap::new();
    };
    let mut rest = &value[start + element.len() + 1..start + end];
    let mut values = HashMap::new();
    while !rest.is_empty() {
        rest = rest.trim_start();
        let key_end = rest
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .unwrap_or(rest.len());
        if key_end == 0 {
            break;
        }
        let key = rest[..key_end].to_ascii_lowercase();
        rest = rest[key_end..].trim_start();
        if !rest.starts_with('=') {
            break;
        }
        rest = rest[1..].trim_start();
        let quote = rest.chars().next().unwrap_or(' ');
        let quoted = quote == '\'' || quote == '"';
        if quoted {
            rest = &rest[1..];
        }
        let end = if quoted {
            rest.find(quote)
        } else {
            rest.find(char::is_whitespace)
        }
        .unwrap_or(rest.len());
        values.insert(key, rest[..end].to_string());
        rest = &rest[end..];
        if quoted && !rest.is_empty() {
            rest = &rest[1..];
        }
    }
    values
}

pub struct StageImage {
    pub png: Vec<u8>,
    pub bounds: [f32; 4],
    pub warnings: Vec<String>,
}

fn validate_bounds(bounds: [f32; 4]) -> Result<(u32, u32), String> {
    let width = (bounds[2] - bounds[0]).ceil();
    let height = (bounds[3] - bounds[1]).ceil();
    if width < 1. || height < 1. || width * height > 16_000_000. || width > 16384. || height > 16384. {
        return Err("Authored stage dimensions exceed preview limits".into());
    }
    Ok((width as u32, height as u32))
}

#[derive(Clone)]
struct Placement {
    id: u16,
    matrix: Matrix,
    color: ColorTransform,
    born: u16,
    clip: Option<u16>,
    visible: bool,
}

fn read_tags<'a>(
    data: &'a [u8],
    version: u8,
    warnings: &mut BTreeSet<String>,
) -> Result<Vec<Tag<'a>>, String> {
    let mut tags = Vec::new();
    let mut rest = data;
    while !rest.is_empty() {
        let mut reader = swf::read::Reader::new(rest, version);
        let (code, length) = reader
            .read_tag_code_and_length()
            .map_err(|e| e.to_string())?;
        let prefix = rest.len() - reader.get_ref().len();
        let body = rest
            .get(prefix..prefix + length)
            .ok_or("Truncated SWF tag")?;
        let whole = &rest[..prefix + length];
        rest = &rest[prefix + length..];
        if code == 0 {
            break;
        }
        if code == 39 && body.len() >= 4 {
            tags.push(Tag::DefineSprite(swf::Sprite {
                id: u16::from_le_bytes([body[0], body[1]]),
                num_frames: u16::from_le_bytes([body[2], body[3]]),
                tags: read_tags(&body[4..], version, warnings)?,
            }));
        } else {
            match swf::read::Reader::new(whole, version).read_tag() {
                Ok(tag) => tags.push(tag),
                Err(_) => {
                    warnings.insert(format!(
                        "Tag {code} cannot be decoded for preview; its original bytes are preserved"
                    ));
                }
            }
        }
    }
    Ok(tags)
}

fn id(tag: &Tag<'_>) -> Option<u16> {
    match tag {
        Tag::DefineShape(s) => Some(s.id),
        Tag::DefineSprite(s) => Some(s.id),
        Tag::DefineFont(s) => Some(s.id),
        Tag::DefineFont2(s) => Some(s.id),
        Tag::DefineFont4(s) => Some(s.id),
        Tag::DefineText(s) | Tag::DefineText2(s) => Some(s.id),
        Tag::DefineEditText(s) => Some(s.id()),
        Tag::DefineBitsLossless(s) => Some(s.id),
        Tag::DefineBits { id, .. } | Tag::DefineBitsJpeg2 { id, .. } => Some(*id),
        Tag::DefineBitsJpeg3(s) => Some(s.id),
        Tag::DefineButton(s) | Tag::DefineButton2(s) => Some(s.id),
        _ => None,
    }
}

fn index<'a>(tags: &'a [Tag<'a>], definitions: &mut HashMap<u16, &'a Tag<'a>>) {
    for tag in tags {
        if let Some(id) = id(tag) {
            definitions.insert(id, tag);
        }
        if let Tag::DefineSprite(sprite) = tag {
            index(&sprite.tags, definitions);
        }
    }
}

fn display_list(
    tags: &[Tag<'_>],
    frame: u16,
    warnings: &mut BTreeSet<String>,
) -> Result<BTreeMap<u16, Placement>, String> {
    let mut live = BTreeMap::new();
    let mut current = 1;
    for tag in tags {
        match tag {
            Tag::ShowFrame => {
                if current == frame {
                    return Ok(live);
                }
                current += 1;
            }
            Tag::PlaceObject(p) => {
                match p.action {
                    PlaceObjectAction::Place(id) => {
                        live.insert(
                            p.depth,
                            Placement {
                                id,
                                matrix: Matrix::IDENTITY,
                                color: ColorTransform::IDENTITY,
                                born: current,
                                clip: None,
                                visible: true,
                            },
                        );
                    }
                    PlaceObjectAction::Replace(id) => {
                        if let Some(old) = live.get_mut(&p.depth) {
                            old.id = id;
                            old.born = current;
                        }
                    }
                    PlaceObjectAction::Modify => {}
                }
                if let Some(item) = live.get_mut(&p.depth) {
                    if let Some(m) = p.matrix {
                        item.matrix = m;
                    }
                    if let Some(c) = p.color_transform {
                        item.color = c;
                    }
                    if let Some(c) = p.clip_depth {
                        item.clip = Some(c);
                    }
                    if let Some(v) = p.is_visible {
                        item.visible = v;
                    }
                    if p.filters.as_ref().is_some_and(|f| !f.is_empty()) {
                        warnings
                            .insert("Flash filters are not evaluated in authored preview".into());
                    }
                    if p.blend_mode
                        .is_some_and(|b| b != swf::BlendMode::Normal && b != swf::BlendMode::Layer)
                    {
                        warnings.insert(
                            "Non-normal blend modes are approximated in authored preview".into(),
                        );
                    }
                } else {
                    warnings.insert(format!("Placement at absent depth {} was ignored", p.depth));
                }
            }
            Tag::RemoveObject(r) => {
                live.remove(&r.depth);
            }
            _ => {}
        }
    }
    Err(format!("Frame {frame} has no ShowFrame tag"))
}

struct Renderer<'a> {
    definitions: HashMap<u16, &'a Tag<'a>>,
    font_names: HashMap<String, &'a Tag<'a>>,
    warnings: BTreeSet<String>,
    viewport: Transform,
    bounds: Option<tiny_skia::Rect>,
    budget: usize,
    measure: bool,
    stencil: bool,
}

impl<'a> Renderer<'a> {
    fn draw(&mut self, draw: Draw, image: &mut Pixmap, mask: Option<&Mask>) -> Result<(), String> {
        self.bounds = Some(self.bounds.map_or(draw.bounds, |previous| {
            previous.join(&draw.bounds).unwrap_or(previous)
        }));
        if self.measure {
            return Ok(());
        }
        if self.stencil && draw.stroke.is_some() {
            return Ok(());
        }
        let bitmap = match &draw.fill {
            _ if self.stencil => None,
            swf::FillStyle::Bitmap { id, .. } => Some(bitmap_image(
                self.definitions
                    .get(id)
                    .ok_or("Missing bitmap definition")?,
                draw.color,
            )?),
            _ => None,
        };
        let mut paint = Paint::default();
        if self.stencil {
            paint.set_color_rgba8(255, 255, 255, 255);
        } else {
            paint.shader = shader(&draw.fill, draw.color, bitmap.as_ref())?;
        }
        let t = self.viewport.pre_concat(draw.transform);
        if let Some(stroke) = draw.stroke {
            image.stroke_path(&draw.path, &paint, &stroke, t, mask);
        } else {
            image.fill_path(&draw.path, &paint, draw.fill_rule, t, mask);
        }
        Ok(())
    }

    fn glyph(
        &mut self,
        records: &[ShapeRecord],
        matrix: Matrix,
        color: swf::Color,
        ct: ColorTransform,
        image: &mut Pixmap,
        mask: Option<&Mask>,
    ) -> Result<(), String> {
        if self.budget == 0 {
            return Err("Stage exceeds rendering operation limit".into());
        }
        self.budget -= 1;
        let shape = Shape {
            version: 3,
            id: 0,
            shape_bounds: swf::Rectangle {
                x_min: Twips::new(-65536),
                x_max: Twips::new(65536),
                y_min: Twips::new(-65536),
                y_max: Twips::new(65536),
            },
            edge_bounds: Default::default(),
            flags: swf::ShapeFlag::empty(),
            styles: swf::ShapeStyles {
                fill_styles: vec![swf::FillStyle::Color(color)],
                line_styles: vec![],
            },
            shape: records.to_vec(),
        };
        for mut draw in shape_draws(&shape, matrix, ct)? {
            if let Some(b) = draw.path.bounds().transform(draw.transform) {
                draw.bounds = b;
            }
            self.draw(draw, image, mask)?;
        }
        Ok(())
    }

    fn font(&self, id: u16, name: Option<&swf::SwfStr>) -> Option<&'a Tag<'a>> {
        self.definitions
            .get(&id)
            .copied()
            .or_else(|| name.and_then(|n| self.font_names.get(&swf_string(n)).copied()))
    }

    fn text(
        &mut self,
        text: &swf::Text,
        matrix: Matrix,
        ct: ColorTransform,
        image: &mut Pixmap,
        mask: Option<&Mask>,
    ) -> Result<(), String> {
        let mut font_id = 0;
        let mut height = Twips::new(240);
        let mut color = swf::Color::BLACK;
        let mut x = 0;
        let mut y = 0;
        for record in &text.records {
            if let Some(f) = record.font_id {
                font_id = f;
            }
            if let Some(h) = record.height {
                height = h;
            }
            if let Some(c) = record.color {
                color = c;
            }
            if let Some(v) = record.x_offset {
                x = v.get();
            }
            if let Some(v) = record.y_offset {
                y = v.get();
            }
            for glyph in &record.glyphs {
                let font = self.font(font_id, None);
                let (records, em) = match font {
                    Some(Tag::DefineFont2(font)) => (
                        font.glyphs
                            .get(glyph.index as usize)
                            .map(|g| g.shape_records.as_slice()),
                        if font.version == 3 { 20480.0 } else { 1024.0 },
                    ),
                    Some(Tag::DefineFont(font)) => (
                        font.glyphs.get(glyph.index as usize).map(Vec::as_slice),
                        1024.0,
                    ),
                    _ => (None, 1024.0),
                };
                if let Some(records) = records {
                    let mut local = Matrix::IDENTITY;
                    local.a = swf::Fixed16::from_f32(height.get() as f32 / em);
                    local.d = local.a;
                    local.tx = Twips::new(x);
                    local.ty = Twips::new(y);
                    self.glyph(
                        records,
                        matrix * text.matrix * local,
                        color,
                        ct,
                        image,
                        mask,
                    )?;
                } else {
                    self.warnings.insert(format!(
                        "Text {} needs unavailable font/glyph {font_id}:{}",
                        text.id, glyph.index
                    ));
                }
                x += glyph.advance;
            }
        }
        Ok(())
    }

    fn edit_text(
        &mut self,
        text: &swf::EditText<'_>,
        matrix: Matrix,
        ct: ColorTransform,
        image: &mut Pixmap,
        mask: Option<&Mask>,
    ) -> Result<(), String> {
        let Some(initial) = text.initial_text() else {
            return Ok(());
        };
        let mut value = swf_string(initial);
        if value.is_empty() {
            return Ok(());
        }
        let font_style = if text.is_html() {
            html_attributes(&value, "font")
        } else {
            HashMap::new()
        };
        let paragraph = if text.is_html() {
            html_attributes(&value, "p")
        } else {
            HashMap::new()
        };
        if text.is_html() {
            if value.to_ascii_lowercase().matches("<font").count() > 1
                || value.to_ascii_lowercase().contains("<u>")
            {
                self.warnings.insert(
                    "Mixed HTML font runs/underline are approximated in authored preview".into(),
                );
            }
            let mut inside = false;
            value = value
                .chars()
                .filter(|c| {
                    if *c == '<' {
                        inside = true;
                        false
                    } else if *c == '>' {
                        inside = false;
                        false
                    } else {
                        !inside
                    }
                })
                .collect();
            value = value
                .replace("&amp;", "&")
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&quot;", "\"")
                .replace("&nbsp;", " ");
        }
        let font_id = text.font_id().unwrap_or_default();
        let font = self.font(font_id, text.font_class()).or_else(|| {
            font_style
                .get("face")
                .and_then(|name| self.font_names.get(name).copied())
        });
        let Some(Tag::DefineFont2(font)) = font else {
            self.warnings.insert(format!(
                "Text {} needs an embedded/imported font ({font_id}); no substitute was used",
                text.id()
            ));
            return Ok(());
        };
        let height = font_style
            .get("size")
            .and_then(|s| s.parse::<f32>().ok())
            .filter(|h| *h > 0. && *h < 4096.)
            .map_or(
                text.height().unwrap_or(Twips::new(240)).get() as f32,
                |size| size * 20.,
            );
        let em = if font.version == 3 { 20480.0 } else { 1024.0 };
        let scale = height / em;
        let bounds = text.bounds();
        let layout = text.layout();
        let left = bounds.x_min.get() as f32 + layout.map_or(0., |l| l.left_margin.get() as f32);
        let right = bounds.x_max.get() as f32 - layout.map_or(0., |l| l.right_margin.get() as f32);
        let ascent = font
            .layout
            .as_ref()
            .map_or(height * 0.8, |l| l.ascent as f32 * scale);
        let mut y = bounds.y_min.get() as f32 + ascent;
        let mut color = text.color().unwrap_or(swf::Color::BLACK);
        if let Some(rgb) = font_style
            .get("color")
            .and_then(|c| u32::from_str_radix(c.trim_start_matches('#'), 16).ok())
        {
            color = swf::Color {
                r: (rgb >> 16) as u8,
                g: (rgb >> 8) as u8,
                b: rgb as u8,
                a: 255,
            };
        }
        let spacing = font_style
            .get("letterspacing")
            .and_then(|s| s.parse::<f32>().ok())
            .filter(|s| s.is_finite())
            .unwrap_or(0.)
            * 20.;
        let kerning = font_style.get("kerning").is_some_and(|s| s != "0");
        let adjust = |previous: Option<u16>, code: u16| -> f32 {
            if kerning {
                previous
                    .and_then(|previous| {
                        font.layout
                            .as_ref()?
                            .kerning
                            .iter()
                            .find(|k| k.left_code == previous && k.right_code == code)
                    })
                    .map_or(0., |k| k.adjustment.get() as f32 * scale)
            } else {
                0.
            }
        };
        let mut lines: Vec<Vec<&swf::Glyph>> = vec![Vec::new()];
        let mut width = 0.;
        for c in value.chars() {
            if c == '\r' {
                continue;
            }
            if c == '\n' {
                lines.push(Vec::new());
                width = 0.;
                continue;
            }
            if let Some(glyph) = font.glyphs.iter().find(|g| g.code as u32 == c as u32) {
                let advance = glyph.advance as f32 * scale + spacing;
                if text.is_word_wrap()
                    && width + advance > right - left
                    && !lines.last().unwrap().is_empty()
                {
                    lines.push(Vec::new());
                    width = 0.;
                }
                lines.last_mut().unwrap().push(glyph);
                width += advance;
            } else {
                self.warnings.insert(format!(
                    "Font {} has no glyph for U+{:04X}",
                    swf_string(font.name),
                    c as u32
                ));
            }
        }
        // Text is clipped to its authored field, even inside another mask.
        let mut field_mask =
            Mask::new(image.width(), image.height()).ok_or("Cannot allocate text mask")?;
        let rectangle = tiny_skia::Rect::from_ltrb(
            bounds.x_min.to_pixels() as f32,
            bounds.y_min.to_pixels() as f32,
            bounds.x_max.to_pixels() as f32,
            bounds.y_max.to_pixels() as f32,
        );
        if let Some(rectangle) = rectangle {
            let mut path = tiny_skia::PathBuilder::new();
            path.push_rect(rectangle);
            field_mask.fill_path(
                &path.finish().ok_or("Invalid text rectangle")?,
                FillRule::Winding,
                true,
                self.viewport.pre_concat(transform(matrix)),
            );
            if let Some(parent) = mask {
                multiply(&mut field_mask, parent);
            }
        }
        for line in lines {
            let mut previous = None;
            let width: f32 = line
                .iter()
                .map(|g| {
                    let width = g.advance as f32 * scale + spacing + adjust(previous, g.code);
                    previous = Some(g.code);
                    width
                })
                .sum::<f32>()
                - spacing;
            let mut x = left + layout.map_or(0., |l| l.indent.get() as f32);
            let align = paragraph
                .get("align")
                .map(|s| match s.to_ascii_lowercase().as_str() {
                    "right" => swf::TextAlign::Right,
                    "center" => swf::TextAlign::Center,
                    _ => swf::TextAlign::Left,
                });
            match align.or_else(|| layout.map(|l| l.align)) {
                Some(swf::TextAlign::Right) => x = right - width,
                Some(swf::TextAlign::Center) => x = left + (right - left - width) / 2.,
                Some(swf::TextAlign::Justify) => {
                    self.warnings
                        .insert("Justified text uses left alignment in authored preview".into());
                }
                _ => {}
            }
            previous = None;
            for glyph in line {
                x += adjust(previous, glyph.code);
                previous = Some(glyph.code);
                let mut local = Matrix::IDENTITY;
                local.a = swf::Fixed16::from_f32(scale);
                local.d = local.a;
                local.tx = Twips::new(x.round() as i32);
                local.ty = Twips::new(y.round() as i32);
                if !glyph.shape_records.is_empty() {
                    self.glyph(
                        &glyph.shape_records,
                        matrix * local,
                        color,
                        ct,
                        image,
                        Some(&field_mask),
                    )?;
                }
                x += glyph.advance as f32 * scale + spacing;
            }
            y += height + layout.map_or(0., |l| l.leading.get() as f32);
        }
        Ok(())
    }

    fn character(
        &mut self,
        id: u16,
        frame: u16,
        matrix: Matrix,
        color: ColorTransform,
        image: &mut Pixmap,
        mask: Option<&Mask>,
        stack: &mut Vec<u16>,
    ) -> Result<(), String> {
        if self.budget == 0 {
            return Err("Stage exceeds 64000 rendering operations".into());
        }
        self.budget -= 1;
        if stack.contains(&id) || stack.len() >= 64 {
            return Err("Recursive/deep SWF sprite".into());
        }
        let Some(tag) = self.definitions.get(&id).copied() else {
            self.warnings
                .insert(format!("Character {id} is unavailable for preview"));
            return Ok(());
        };
        let result = match tag {
            Tag::DefineShape(shape) => {
                for draw in shape_draws(shape, matrix, color)? {
                    self.draw(draw, image, mask)?;
                }
                Ok(())
            }
            Tag::DefineSprite(sprite) => {
                stack.push(id);
                let result = self.timeline(
                    &sprite.tags,
                    (frame - 1) % sprite.num_frames.max(1) + 1,
                    matrix,
                    color,
                    image,
                    mask,
                    stack,
                );
                stack.pop();
                result
            }
            Tag::DefineText(text) | Tag::DefineText2(text) => {
                self.text(text, matrix, color, image, mask)
            }
            Tag::DefineEditText(text) => self.edit_text(text, matrix, color, image, mask),
            Tag::DefineButton(button) | Tag::DefineButton2(button) => {
                let mut records: Vec<_> = button
                    .records
                    .iter()
                    .filter(|r| r.states.contains(swf::ButtonState::UP))
                    .collect();
                records.sort_by_key(|r| r.depth);
                for r in records {
                    self.character(
                        r.id,
                        1,
                        matrix * r.matrix,
                        color * r.color_transform,
                        image,
                        mask,
                        stack,
                    )?;
                }
                Ok(())
            }
            _ => {
                self.warnings
                    .insert(format!("Character {id} has unsupported preview content"));
                Ok(())
            }
        };
        result
    }

    fn timeline(
        &mut self,
        tags: &[Tag<'a>],
        frame: u16,
        matrix: Matrix,
        color: ColorTransform,
        image: &mut Pixmap,
        parent: Option<&Mask>,
        stack: &mut Vec<u16>,
    ) -> Result<(), String> {
        let mut active: Vec<(u16, Mask)> = Vec::new();
        for (depth, p) in display_list(tags, frame, &mut self.warnings)? {
            active.retain(|(end, _)| *end >= depth);
            if !p.visible {
                continue;
            }
            let mut combined = parent.cloned();
            for (_, mask) in &active {
                if let Some(previous) = &mut combined {
                    multiply(previous, mask);
                } else {
                    combined = Some(mask.clone());
                }
            }
            if let Some(end) = p.clip {
                if active.len() >= 8 {
                    return Err("More than eight simultaneous clip masks".into());
                }
                let mut clip = Pixmap::new(image.width(), image.height())
                    .ok_or("Cannot allocate clip mask")?;
                let previous_stencil = self.stencil;
                self.stencil = true;
                let result = self.character(
                    p.id,
                    frame - p.born + 1,
                    matrix * p.matrix,
                    color * p.color,
                    &mut clip,
                    None,
                    stack,
                );
                self.stencil = previous_stencil;
                result?;
                active.push((end, Mask::from_pixmap(clip.as_ref(), MaskType::Alpha)));
            } else {
                self.character(
                    p.id,
                    frame - p.born + 1,
                    matrix * p.matrix,
                    color * p.color,
                    image,
                    combined.as_ref(),
                    stack,
                )?;
            }
        }
        Ok(())
    }
}

fn multiply(target: &mut Mask, other: &Mask) {
    for (a, b) in target.data_mut().iter_mut().zip(other.data()) {
        *a = ((*a as u16 * *b as u16 + 127) / 255) as u8;
    }
}

pub fn sprite_frame_bounds(data: &[u8], owner: u16, frames: &[u16]) -> Result<Vec<Option<[f32; 4]>>, String> {
    let buffer = swf::decompress_swf(data).map_err(|e| e.to_string())?;
    let mut warnings = BTreeSet::new();
    let tags = read_tags(&buffer.data, buffer.header.version(), &mut warnings)?;
    let mut definitions = HashMap::new();
    index(&tags, &mut definitions);
    let mut renderer = Renderer {
        definitions, font_names: HashMap::new(), warnings,
        viewport: Transform::identity(), bounds: None, budget: 64000, measure: true, stencil: false,
    };
    Ok(frames.iter().map(|&frame| renderer.sprite_bounds(owner, frame).ok()).collect())
}

impl Renderer<'_> {
    fn sprite_bounds(&mut self, owner: u16, frame: u16) -> Result<[f32; 4], String> {
        if frame == 0 {
            return Err("Frames are one-based".into());
        }
        self.bounds = None;
        self.budget = 64000;
        let mut image = Pixmap::new(1, 1).unwrap();
        self.character(owner, frame, Matrix::IDENTITY, ColorTransform::IDENTITY,
            &mut image, None, &mut Vec::new())?;
        let b = self.bounds.ok_or("Sprite has no authored drawing")?;
        let bounds = [b.left().floor(), b.top().floor(), b.right().ceil(), b.bottom().ceil()];
        validate_bounds(bounds)?;
        Ok(bounds)
    }
}

pub fn render(
    data: &[u8],
    frame: u16,
    owner: u16,
    fonts: &[Vec<u8>],
) -> Result<StageImage, String> {
    if frame == 0 {
        return Err("Frames are one-based".into());
    }
    let buffer = swf::decompress_swf(data).map_err(|e| e.to_string())?;
    let mut warnings = BTreeSet::new();
    let tags = read_tags(&buffer.data, buffer.header.version(), &mut warnings)?;
    let font_buffers: Vec<_> = fonts
        .iter()
        .map(|d| swf::decompress_swf(d.as_slice()).map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()?;
    let font_tags: Vec<_> = font_buffers
        .iter()
        .map(|b| read_tags(&b.data, b.header.version(), &mut warnings))
        .collect::<Result<_, _>>()?;
    let mut definitions = HashMap::new();
    index(&tags, &mut definitions);
    let mut font_names = HashMap::new();
    for library in &font_tags {
        let mut local = HashMap::new();
        index(library, &mut local);
        for tag in library {
            if let Tag::ExportAssets(exports) = tag {
                for asset in exports {
                    if let Some(value) = local.get(&asset.id) {
                        font_names.insert(swf_string(asset.name), *value);
                    }
                }
            }
            if let Tag::DefineFont2(font) = tag {
                font_names.insert(swf_string(font.name).to_string(), tag);
            }
            if let Tag::SymbolClass(exports) = tag {
                for asset in exports {
                    if let Some(value) = local.get(&asset.id) {
                        font_names.insert(swf_string(asset.class_name), *value);
                    }
                }
            }
        }
    }
    for tag in &tags {
        if let Tag::ImportAssets { imports, .. } = tag {
            for asset in imports {
                if let Some(value) = font_names.get(&swf_string(asset.name)) {
                    definitions.insert(asset.id, *value);
                }
            }
        }
    }
    let mut renderer = Renderer {
        definitions,
        font_names,
        warnings,
        viewport: Transform::identity(),
        bounds: None,
        budget: 64000,
        measure: owner != 0,
        stencil: false,
    };
    let bounds = if owner == 0 {
        let b = buffer.header.stage_size();
        [
            b.x_min.to_pixels() as f32,
            b.y_min.to_pixels() as f32,
            b.x_max.to_pixels() as f32,
            b.y_max.to_pixels() as f32,
        ]
    } else {
        renderer.sprite_bounds(owner, frame)?
    };
    let (width, height) = validate_bounds(bounds)?;
    renderer.viewport = Transform::from_translate(-bounds[0], -bounds[1]);
    renderer.measure = false;
    renderer.budget = 64000;
    let mut image =
        Pixmap::new(width, height).ok_or("Cannot allocate authored stage")?;
    if owner == 0 {
        if frame > buffer.header.num_frames().max(1) {
            return Err("Stage frame is out of range".into());
        }
        renderer.timeline(
            &tags,
            frame,
            Matrix::IDENTITY,
            ColorTransform::IDENTITY,
            &mut image,
            None,
            &mut Vec::new(),
        )?;
    } else {
        renderer.character(
            owner,
            frame,
            Matrix::IDENTITY,
            ColorTransform::IDENTITY,
            &mut image,
            None,
            &mut Vec::new(),
        )?;
    }
    Ok(StageImage {
        png: image.encode_png().map_err(|e| e.to_string())?,
        bounds,
        warnings: renderer.warnings.into_iter().collect(),
    })
}
