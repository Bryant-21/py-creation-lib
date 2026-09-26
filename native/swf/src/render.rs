use std::collections::{BTreeMap, HashMap};
use std::io::Read;

use swf::{ColorTransform, FillStyle, Matrix, PlaceObjectAction, Shape, ShapeRecord, Sprite, Tag, TagCode};
use tiny_skia::{FillRule, Paint, Path, PathBuilder, Pixmap, Stroke, Transform};

#[derive(Clone)]
struct Placement {
    id: u16,
    matrix: Matrix,
    color: ColorTransform,
}

struct Draw {
    path: Path,
    fill: FillStyle,
    color: ColorTransform,
    stroke: Option<Stroke>,
    transform: Transform,
    bounds: tiny_skia::Rect,
}

#[derive(Clone, Copy)]
struct Edge {
    start: (i32, i32),
    end: (i32, i32),
    control: Option<(i32, i32)>,
}

impl Edge {
    fn reversed(self) -> Self {
        Self {
            start: self.end,
            end: self.start,
            ..self
        }
    }
}

fn paths(edges: &[Edge], close: bool) -> Result<Path, String> {
    let mut pending = edges.to_vec();
    let mut path = PathBuilder::new();
    while !pending.is_empty() {
        let first = pending.remove(0);
        path.move_to(first.start.0 as f32 / 20.0, first.start.1 as f32 / 20.0);
        let mut edge = first;
        loop {
            let (x, y) = (edge.end.0 as f32 / 20.0, edge.end.1 as f32 / 20.0);
            if let Some((cx, cy)) = edge.control {
                path.quad_to(cx as f32 / 20.0, cy as f32 / 20.0, x, y);
            } else {
                path.line_to(x, y);
            }
            if edge.end == first.start {
                if close {
                    path.close();
                }
                break;
            }
            if let Some(index) = pending
                .iter()
                .position(|candidate| candidate.start == edge.end)
            {
                edge = pending.remove(index);
            } else {
                if close {
                    return Err("Unclosed SWF fill contour".into());
                }
                break;
            }
        }
    }
    path.finish().ok_or_else(|| "Empty SWF path".into())
}

fn transform(matrix: Matrix) -> Transform {
    Transform::from_row(
        matrix.a.to_f32(),
        matrix.b.to_f32(),
        matrix.c.to_f32(),
        matrix.d.to_f32(),
        matrix.tx.to_pixels() as f32,
        matrix.ty.to_pixels() as f32,
    )
}

fn tint(value: swf::Color, color: ColorTransform) -> tiny_skia::Color {
    let apply =
        |v: u8, m: swf::Fixed8, a: i16| (v as f32 * m.to_f32() + a as f32).clamp(0.0, 255.0) as u8;
    tiny_skia::Color::from_rgba8(
        apply(value.r, color.r_multiply, color.r_add),
        apply(value.g, color.g_multiply, color.g_add),
        apply(value.b, color.b_multiply, color.b_add),
        apply(value.a, color.a_multiply, color.a_add),
    )
}

fn shader<'a>(
    style: &FillStyle,
    color: ColorTransform,
    bitmap: Option<&'a Pixmap>,
) -> Result<tiny_skia::Shader<'a>, String> {
    match style {
        FillStyle::Color(value) => Ok(tiny_skia::Shader::SolidColor(tint(*value, color))),
        FillStyle::LinearGradient(g)
        | FillStyle::RadialGradient(g)
        | FillStyle::FocalGradient { gradient: g, .. } => {
            if g.interpolation != swf::GradientInterpolation::Rgb {
                return Err("Linear RGB gradient is not supported".into());
            }
            let stops = g
                .records
                .iter()
                .map(|r| tiny_skia::GradientStop::new(r.ratio as f32 / 255.0, tint(r.color, color)))
                .collect();
            let spread = match g.spread {
                swf::GradientSpread::Pad => tiny_skia::SpreadMode::Pad,
                swf::GradientSpread::Reflect => tiny_skia::SpreadMode::Reflect,
                swf::GradientSpread::Repeat => tiny_skia::SpreadMode::Repeat,
            };
            let radius = 16384.0 / 20.0;
            let point = tiny_skia::Point::from_xy;
            if matches!(style, FillStyle::LinearGradient(_)) {
                tiny_skia::LinearGradient::new(
                    point(-radius, 0.0),
                    point(radius, 0.0),
                    stops,
                    spread,
                    transform(g.matrix),
                )
            } else {
                let focal = match style {
                    FillStyle::FocalGradient { focal_point, .. } => focal_point.to_f32(),
                    _ => 0.0,
                };
                tiny_skia::RadialGradient::new(
                    point(focal * radius, 0.0),
                    0.0,
                    point(0.0, 0.0),
                    radius,
                    stops,
                    spread,
                    transform(g.matrix),
                )
            }
            .ok_or_else(|| "Invalid SWF gradient".into())
        }
        FillStyle::Bitmap {
            matrix,
            is_smoothed,
            is_repeating,
            ..
        } => Ok(tiny_skia::Pattern::new(
            bitmap.ok_or("Missing sprite bitmap")?.as_ref(),
            if *is_repeating {
                tiny_skia::SpreadMode::Repeat
            } else {
                tiny_skia::SpreadMode::Pad
            },
            if *is_smoothed {
                tiny_skia::FilterQuality::Bilinear
            } else {
                tiny_skia::FilterQuality::Nearest
            },
            1.0,
            transform(*matrix).pre_scale(1.0 / 20.0, 1.0 / 20.0),
        )),
    }
}

fn shape_draws(shape: &Shape, matrix: Matrix, color: ColorTransform) -> Result<Vec<Draw>, String> {
    let b = &shape.shape_bounds;
    let bounds = tiny_skia::Rect::from_ltrb(
        b.x_min.to_pixels() as f32,
        b.y_min.to_pixels() as f32,
        b.x_max.to_pixels() as f32,
        b.y_max.to_pixels() as f32,
    )
    .ok_or("Empty shape bounds")?
    .transform(transform(matrix))
    .ok_or("Invalid shape transform")?;
    let mut fills = shape.styles.fill_styles.clone();
    let mut lines = shape.styles.line_styles.clone();
    let mut fill_edges: BTreeMap<usize, Vec<Edge>> = BTreeMap::new();
    let mut line_edges: BTreeMap<usize, Vec<Edge>> = BTreeMap::new();
    let (mut f0, mut f1, mut line) = (None, None, None);
    let (mut fill_offset, mut line_offset) = (0, 0);
    let mut at = (0, 0);
    for record in &shape.shape {
        let edge = match record {
            ShapeRecord::StyleChange(change) => {
                if let Some(p) = change.move_to {
                    at = (p.x.get(), p.y.get());
                }
                if let Some(i) = change.fill_style_0 {
                    f0 = (i != 0).then(|| fill_offset + i as usize - 1);
                }
                if let Some(i) = change.fill_style_1 {
                    f1 = (i != 0).then(|| fill_offset + i as usize - 1);
                }
                if let Some(i) = change.line_style {
                    line = (i != 0).then(|| line_offset + i as usize - 1);
                }
                if let Some(styles) = &change.new_styles {
                    fill_offset = fills.len();
                    line_offset = lines.len();
                    fills.extend_from_slice(&styles.fill_styles);
                    lines.extend_from_slice(&styles.line_styles);
                }
                continue;
            }
            ShapeRecord::StraightEdge { delta } => Edge {
                start: at,
                end: (at.0 + delta.dx.get(), at.1 + delta.dy.get()),
                control: None,
            },
            ShapeRecord::CurvedEdge {
                control_delta,
                anchor_delta,
            } => {
                let control = (at.0 + control_delta.dx.get(), at.1 + control_delta.dy.get());
                Edge {
                    start: at,
                    end: (
                        control.0 + anchor_delta.dx.get(),
                        control.1 + anchor_delta.dy.get(),
                    ),
                    control: Some(control),
                }
            }
        };
        at = edge.end;
        if f0 != f1 {
            if let Some(i) = f0 {
                fill_edges.entry(i).or_default().push(edge.reversed());
            }
            if let Some(i) = f1 {
                fill_edges.entry(i).or_default().push(edge);
            }
        }
        if let Some(i) = line {
            line_edges.entry(i).or_default().push(edge);
        }
    }
    let mut draws = Vec::new();
    for (i, edges) in fill_edges {
        let style = fills.get(i).ok_or("Invalid SWF fill index")?;
        draws.push(Draw {
            path: paths(&edges, true)?,
            fill: style.clone(),
            color,
            stroke: None,
            transform: transform(matrix),
            bounds,
        });
    }
    for (i, edges) in line_edges {
        let style = lines.get(i).ok_or("Invalid SWF line index")?;
        let stroke = Stroke {
            width: style.width().to_pixels() as f32,
            line_cap: match style.start_cap() {
                swf::LineCapStyle::Round => tiny_skia::LineCap::Round,
                swf::LineCapStyle::None => tiny_skia::LineCap::Butt,
                swf::LineCapStyle::Square => tiny_skia::LineCap::Square,
            },
            line_join: match style.join_style() {
                swf::LineJoinStyle::Round => tiny_skia::LineJoin::Round,
                swf::LineJoinStyle::Bevel => tiny_skia::LineJoin::Bevel,
                swf::LineJoinStyle::Miter(_) => tiny_skia::LineJoin::Miter,
            },
            ..Stroke::default()
        };
        draws.push(Draw {
            path: paths(&edges, false)?,
            fill: style.fill_style().clone(),
            color,
            stroke: Some(stroke),
            transform: transform(matrix),
            bounds,
        });
    }
    Ok(draws)
}

fn display_list(tags: &[Tag<'_>], frame: u16) -> Result<BTreeMap<u16, Placement>, String> {
    let mut list = BTreeMap::new();
    let mut current = 1;
    for tag in tags {
        match tag {
            Tag::ShowFrame => {
                if current == frame {
                    return Ok(list);
                }
                current += 1;
            }
            Tag::PlaceObject(place) => {
                if place.clip_depth.is_some()
                    || place.filters.as_ref().is_some_and(|v| !v.is_empty())
                    || place
                        .blend_mode
                        .is_some_and(|v| v != swf::BlendMode::Normal)
                    || place.is_visible == Some(false)
                {
                    return Err(
                        "Unsupported mask, filter, blend mode or visibility in sprite artwork"
                            .into(),
                    );
                }
                match place.action {
                    PlaceObjectAction::Place(id) => {
                        list.insert(
                            place.depth,
                            Placement {
                                id,
                                matrix: Matrix::IDENTITY,
                                color: ColorTransform::IDENTITY,
                            },
                        );
                    }
                    PlaceObjectAction::Replace(id) => {
                        list.get_mut(&place.depth)
                            .ok_or("Replacing absent SWF depth")?
                            .id = id;
                    }
                    PlaceObjectAction::Modify => {}
                }
                let item = list
                    .get_mut(&place.depth)
                    .ok_or("Modifying absent SWF depth")?;
                if let Some(m) = place.matrix {
                    item.matrix = m;
                }
                if let Some(c) = place.color_transform {
                    item.color = c;
                }
            }
            Tag::RemoveObject(remove) => {
                list.remove(&remove.depth);
            }
            _ => {}
        }
    }
    Err(format!("Sprite frame {frame} is missing"))
}

fn collect<'a>(
    id: u16,
    frame: u16,
    definitions: &HashMap<u16, &'a Tag<'a>>,
    matrix: Matrix,
    color: ColorTransform,
    stack: &mut Vec<u16>,
    draws: &mut Vec<Draw>,
) -> Result<(), String> {
    if stack.contains(&id) || stack.len() >= 64 {
        return Err("Recursive SWF sprite artwork".into());
    }
    match definitions
        .get(&id)
        .ok_or_else(|| format!("Missing SWF character {id}"))?
    {
        Tag::DefineShape(shape) => draws.extend(shape_draws(shape, matrix, color)?),
        Tag::DefineSprite(sprite) => {
            stack.push(id);
            for child in display_list(&sprite.tags, frame)?.values() {
                collect(
                    child.id,
                    1,
                    definitions,
                    matrix * child.matrix,
                    color * child.color,
                    stack,
                    draws,
                )?;
            }
            stack.pop();
        }
        _ => return Err(format!("SWF character {id} is not vector artwork")),
    }
    Ok(())
}

fn bitmap_image(tag: &Tag<'_>, color: ColorTransform) -> Result<Pixmap, String> {
    let Tag::DefineBitsLossless(bits) = tag else {
        return Err("Unsupported sprite bitmap encoding".into());
    };
    let (width, height) = (bits.width as usize, bits.height as usize);
    let mut bytes = Vec::new();
    flate2::read::ZlibDecoder::new(bits.data.as_ref())
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let mut pixmap =
        Pixmap::new(bits.width as u32, bits.height as u32).ok_or("Invalid bitmap dimensions")?;
    for y in 0..height {
        for x in 0..width {
            let rgba = match bits.format {
                swf::BitmapFormat::Rgb32 => {
                    let i = (y * width + x) * 4;
                    let p = bytes.get(i..i + 4).ok_or("Truncated ARGB bitmap")?;
                    [p[1], p[2], p[3], if bits.version == 2 { p[0] } else { 255 }]
                }
                swf::BitmapFormat::ColorMap8 { num_colors } => {
                    let stride = if bits.version == 2 { 4 } else { 3 };
                    let count = num_colors as usize + 1;
                    let i = *bytes
                        .get(count * stride + y * width.div_ceil(4) * 4 + x)
                        .ok_or("Truncated indexed bitmap")? as usize;
                    if i >= count {
                        return Err("Invalid bitmap palette index".into());
                    }
                    let p = bytes
                        .get(i * stride..(i + 1) * stride)
                        .ok_or("Truncated bitmap palette")?;
                    [p[0], p[1], p[2], if bits.version == 2 { p[3] } else { 255 }]
                }
                swf::BitmapFormat::Rgb15 => {
                    return Err("15-bit sprite bitmap is not supported".into());
                }
            };
            let unpremultiply = |v: u8| {
                if rgba[3] == 0 {
                    0
                } else {
                    ((v as u32 * 255 + rgba[3] as u32 / 2) / rgba[3] as u32).min(255) as u8
                }
            };
            let value = swf::Color {
                r: unpremultiply(rgba[0]),
                g: unpremultiply(rgba[1]),
                b: unpremultiply(rgba[2]),
                a: rgba[3],
            };
            pixmap.pixels_mut()[y * width + x] = tint(value, color).premultiply().to_color_u8();
        }
    }
    Ok(pixmap)
}

// Scaleform movies (FO76's mapmarkerlibrary.swf) carry tags the swf crate rejects, such as a
// PlaceObject with neither a character nor the move flag, and one of those fails a whole-movie parse.
// Tags are read one at a time, inside sprites too, and unreadable ones are dropped.
fn read_tags_lenient(input: &[u8], version: u8) -> Vec<Tag<'_>> {
    let mut rest = input;
    let mut tags = Vec::new();
    loop {
        let mut header = swf::read::Reader::new(rest, version);
        let Ok((code, length)) = header.read_tag_code_and_length() else {
            break;
        };
        let header_len = rest.len() - header.get_ref().len();
        let Some(body) = rest.get(header_len..header_len + length) else {
            break;
        };
        let whole = &rest[..header_len + length];
        rest = &rest[header_len + length..];
        if code == TagCode::End as u16 {
            break;
        }
        if code == TagCode::DefineSprite as u16 && body.len() >= 4 {
            tags.push(Tag::DefineSprite(Sprite {
                id: u16::from_le_bytes([body[0], body[1]]),
                num_frames: u16::from_le_bytes([body[2], body[3]]),
                tags: read_tags_lenient(&body[4..], version),
            }));
            continue;
        }
        if let Ok(tag) = swf::read::Reader::new(whole, version).read_tag() {
            tags.push(tag);
        }
    }
    tags
}

pub fn render_symbol_png(
    data: &[u8],
    name: &str,
    frame: u16,
    scale: f32,
) -> Result<Vec<u8>, String> {
    if frame == 0 || !scale.is_finite() || scale <= 0.0 {
        return Err("Invalid sprite frame or scale".into());
    }
    let buffer = swf::decompress_swf(data).map_err(|e| e.to_string())?;
    let tags = read_tags_lenient(&buffer.data, buffer.header.version());
    let mut definitions = HashMap::new();
    let mut symbols = Vec::new();
    for tag in &tags {
        match tag {
            Tag::DefineShape(shape) => {
                definitions.insert(shape.id, tag);
            }
            Tag::DefineSprite(sprite) => {
                definitions.insert(sprite.id, tag);
            }
            Tag::DefineBitsLossless(bits) => {
                definitions.insert(bits.id, tag);
            }
            Tag::SymbolClass(entries) => {
                for entry in entries {
                    if entry.class_name.as_bytes() == name.as_bytes() {
                        symbols.push(entry.id);
                    }
                }
            }
            _ => {}
        }
    }
    if symbols.len() != 1 {
        return Err(format!(
            "Expected one sprite named {name}, found {}",
            symbols.len()
        ));
    }
    let mut draws = Vec::new();
    collect(
        symbols[0],
        frame,
        &definitions,
        Matrix::IDENTITY,
        ColorTransform::IDENTITY,
        &mut Vec::new(),
        &mut draws,
    )?;
    let mut bounds: Option<tiny_skia::Rect> = None;
    let num_frames = match definitions[&symbols[0]] {
        Tag::DefineSprite(sprite) => sprite.num_frames,
        _ => 1,
    };
    // Every frame uses the same canvas so Expedition map overlays stay aligned.
    for index in 1..=num_frames {
        let mut frame_draws = Vec::new();
        collect(
            symbols[0],
            index,
            &definitions,
            Matrix::IDENTITY,
            ColorTransform::IDENTITY,
            &mut Vec::new(),
            &mut frame_draws,
        )?;
        for draw in frame_draws {
            bounds = Some(match bounds {
                Some(previous) => previous.join(&draw.bounds).ok_or("Invalid sprite bounds")?,
                None => draw.bounds,
            });
        }
    }
    let bounds = bounds.ok_or("Empty sprite artwork")?;
    let left = bounds.left() * scale;
    let top = bounds.top() * scale;
    let width = (bounds.width() * scale).ceil();
    let height = (bounds.height() * scale).ceil();
    if !(1.0..=16384.0).contains(&width) || !(1.0..=16384.0).contains(&height) {
        return Err("Sprite dimensions out of range".into());
    }
    let mut pixmap =
        Pixmap::new(width as u32, height as u32).ok_or("Cannot allocate sprite image")?;
    let viewport = Transform::from_row(scale, 0.0, 0.0, scale, -left, -top);
    for draw in draws {
        let mut paint = Paint::default();
        let bitmap = match &draw.fill {
            FillStyle::Bitmap { id, .. } => Some(bitmap_image(
                definitions
                    .get(id)
                    .ok_or("Missing sprite bitmap definition")?,
                draw.color,
            )?),
            _ => None,
        };
        paint.shader = shader(&draw.fill, draw.color, bitmap.as_ref())?;
        let transform = viewport.pre_concat(draw.transform);
        if let Some(stroke) = draw.stroke {
            pixmap.stroke_path(&draw.path, &paint, &stroke, transform, None);
        } else {
            pixmap.fill_path(&draw.path, &paint, FillRule::Winding, transform, None);
        }
    }
    if !pixmap.pixels().iter().any(|p| p.alpha() != 0) {
        return Err("Empty sprite artwork".into());
    }
    pixmap.encode_png().map_err(|e| e.to_string())
}
