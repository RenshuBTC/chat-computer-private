//! The canvas element's drawing (its 2D context, CanvasRenderingContext2D): paths (lines, Bézier curves, arcs,
//! ellipses, rectangles and rounded ones; Path2D, and SVG's path data), filled (non-zero or even-odd) and stroked
//! (widths, caps, joins, miters, dashes) through the context's transform, in colours, gradients (linear, radial,
//! conic) and patterns, clipped, at the global alpha, composited by any of the standard's operations and blend
//! modes; text in the page's fonts, filled or stroked (fillText, strokeText) and measured (measureText); pictures
//! and other canvases drawn on it (drawImage); its pixels read and written (getImageData, putImageData, ImageData)
//! and saved as PNG or JPEG (toDataURL, toBlob). Shapes are drawn with the picture crate's vector drawing (the one
//! SVG pictures are drawn with), into the canvas's own pixels, kept here: the page shows them as the element's
//! picture.

use super::{Host, arg, attribute, dom_exception, get, host, host_ref, interface, js, method, node_of, set_slot, slot, str_arg, string, this_element, this_host, wrap};
use crate::dom::NodeId;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use alloc::{format, vec};
use js::object::{Class, ElemKind, Key, ENUMERABLE, WRITABLE};
use js::value::{Gc, Value};
use js::vm::Vm;
use picture::draw::{self, Builder, Coverage, Matrix, Outline, Rgba};
use text::Face;

type R = Result<Value, Value>;

/// What a `Class::Host` object of the canvas's is: a context (its number is its canvas's node), a Path2D, a
/// gradient, a pattern, an ImageData (their state is in their private slot).
pub const CONTEXT: u32 = 7;
const PATH2D: u32 = 8;
const GRADIENT: u32 = 9;
const PATTERN: u32 = 10;
const IMAGE_DATA: u32 = 11;
/// Where a canvas's context object is kept (with its element's other objects).
const CONTEXT_OBJ: u32 = 0x104;

/// The most pixels a canvas may have (past it, it has none: nothing it draws is kept, as a browser's canvas too big
/// for it shows nothing); the most numbers a path may hold, and how deep save() may go.
const MOST_PIXELS: u64 = 1 << 24;
const MOST_PATH: usize = 1 << 22;
const MOST_STATES: usize = 512;

/// A picture the page has, decoded (for drawing on a canvas): its colours (straight RGB, a row after another), its
/// alpha if it has see-through parts, its size as decoded, and its own size (a picture decoded smaller than it is).
pub struct Pixels {
    pub rgb: Vec<u8>,
    pub alpha: Option<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub natural: (u32, u32),
}

// ------------------------------------------------------------------------------------------------- the state

/// How what's drawn goes onto what's there: Porter and Duff's operations, "lighter", and the blend modes.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Op {
    SourceOver,
    SourceIn,
    SourceOut,
    SourceAtop,
    DestOver,
    DestIn,
    DestOut,
    DestAtop,
    Lighter,
    Copy,
    Xor,
    Blend(u8),
}

const OPS: &[(&str, Op)] = &[
    ("source-over", Op::SourceOver),
    ("source-in", Op::SourceIn),
    ("source-out", Op::SourceOut),
    ("source-atop", Op::SourceAtop),
    ("destination-over", Op::DestOver),
    ("destination-in", Op::DestIn),
    ("destination-out", Op::DestOut),
    ("destination-atop", Op::DestAtop),
    ("lighter", Op::Lighter),
    ("copy", Op::Copy),
    ("xor", Op::Xor),
    ("multiply", Op::Blend(0)),
    ("screen", Op::Blend(1)),
    ("overlay", Op::Blend(2)),
    ("darken", Op::Blend(3)),
    ("lighten", Op::Blend(4)),
    ("color-dodge", Op::Blend(5)),
    ("color-burn", Op::Blend(6)),
    ("hard-light", Op::Blend(7)),
    ("soft-light", Op::Blend(8)),
    ("difference", Op::Blend(9)),
    ("exclusion", Op::Blend(10)),
    ("hue", Op::Blend(11)),
    ("saturation", Op::Blend(12)),
    ("color", Op::Blend(13)),
    ("luminosity", Op::Blend(14)),
];

impl Op {
    /// Whether it changes only where the shape is (the others change the whole canvas: outside the shape, as if
    /// what's drawn there were transparent).
    fn bounded(self) -> bool {
        !matches!(self, Op::SourceIn | Op::SourceOut | Op::DestIn | Op::DestAtop | Op::Copy)
    }

    /// Porter and Duff's fractions of the source and the destination kept, by their alphas.
    fn factors(self, sa: f32, da: f32) -> (f32, f32) {
        match self {
            Op::SourceIn => (da, 0.0),
            Op::SourceOut => (1.0 - da, 0.0),
            Op::SourceAtop => (da, 1.0 - sa),
            Op::DestOver => (1.0 - da, 1.0),
            Op::DestIn => (0.0, sa),
            Op::DestOut => (0.0, 1.0 - sa),
            Op::DestAtop => (1.0 - da, sa),
            Op::Copy => (1.0, 0.0),
            Op::Xor => (1.0 - da, 1.0 - sa),
            _ => (1.0, 1.0 - sa),
        }
    }
}

/// What fills or strokes: a colour, or a gradient or pattern object (its state in its slot).
#[derive(Clone)]
enum Paint {
    Color(Rgba),
    Object(Gc),
}

/// The context's font: as set (and given back), its size in pixels, and the face it's drawn in.
#[derive(Clone)]
struct Font {
    text: String,
    size: f32,
    face: Face,
}

impl Font {
    fn initial() -> Font {
        Font { text: String::from("10px sans-serif"), size: 10.0, face: Face::Sans }
    }
}

/// What save() keeps and restore() puts back.
#[derive(Clone)]
struct State {
    m: Matrix,
    fill: Paint,
    stroke: Paint,
    line_width: f32,
    /// 0 butt, 1 round, 2 square; 0 miter, 1 round, 2 bevel.
    cap: u8,
    join: u8,
    miter: f32,
    dash: Vec<f32>,
    dash_offset: f32,
    alpha: f32,
    op: Op,
    font: Font,
    /// 0 start, 1 end, 2 left, 3 right, 4 center; 0 alphabetic, 1 top, 2 hanging, 3 middle, 4 ideographic, 5 bottom;
    /// 0 inherit, 1 ltr, 2 rtl.
    align: u8,
    baseline: u8,
    direction: u8,
    /// How much of each pixel the clip lets through (none: all of it).
    clip: Option<Rc<Vec<u8>>>,
    smoothing: bool,
    smoothing_quality: u8,
    shadow_blur: f32,
    shadow_color: Rgba,
    shadow_offset: (f32, f32),
    filter: String,
}

impl State {
    fn new() -> State {
        State {
            m: Matrix::ID,
            fill: Paint::Color([0.0, 0.0, 0.0, 1.0]),
            stroke: Paint::Color([0.0, 0.0, 0.0, 1.0]),
            line_width: 1.0,
            cap: 0,
            join: 0,
            miter: 10.0,
            dash: Vec::new(),
            dash_offset: 0.0,
            alpha: 1.0,
            op: Op::SourceOver,
            font: Font::initial(),
            align: 0,
            baseline: 0,
            direction: 0,
            clip: None,
            smoothing: true,
            smoothing_quality: 0,
            shadow_blur: 0.0,
            shadow_color: [0.0, 0.0, 0.0, 0.0],
            shadow_offset: (0.0, 0.0),
            filter: String::from("none"),
        }
    }
}

/// A canvas: its pixels (straight RGB and alpha, as the page shows them), its context's state and its path.
pub struct Canvas {
    pub node: NodeId,
    pub w: u32,
    pub h: u32,
    pub rgb: Vec<u8>,
    pub alpha: Vec<u8>,
    /// Counted up each time it's drawn on (the page's painter makes its copy at the size it's shown again).
    pub version: u64,
    states: Vec<State>,
    /// The current path's steps (see `replay`), and the transform its points last went through.
    path: Vec<f32>,
    path_m: Option<Matrix>,
    /// A context made with { alpha: false }: always opaque (black where nothing's drawn).
    opaque: bool,
}

impl Canvas {
    fn new(node: NodeId, w: u32, h: u32) -> Canvas {
        let mut c = Canvas { node, w: 0, h: 0, rgb: Vec::new(), alpha: Vec::new(), version: 0, states: vec![State::new()], path: Vec::new(), path_m: None, opaque: false };
        c.resize(w, h);
        c
    }

    /// Made `w` by `h`: cleared, its state and path as they start (as setting a canvas's width or height does). Its
    /// pixels are made when something's drawn on it (a canvas made only to measure text has none).
    fn resize(&mut self, w: u32, h: u32) {
        self.w = w;
        self.h = h;
        self.rgb = Vec::new();
        self.alpha = Vec::new();
        self.states = vec![State::new()];
        self.path.clear();
        self.path_m = None;
        self.version += 1;
    }

    /// Whether it may have pixels (it isn't empty, nor too big).
    fn fits(&self) -> bool {
        let px = self.w as u64 * self.h as u64;
        px > 0 && px <= MOST_PIXELS
    }

    /// Its pixels, made if they aren't yet (transparent, or black if it's opaque): whether it has them.
    fn ensure(&mut self) -> bool {
        if self.alpha.is_empty() && self.fits() {
            let n = self.w as usize * self.h as usize;
            self.rgb = vec![0; n * 3];
            self.alpha = vec![if self.opaque { 255 } else { 0 }; n];
        }
        !self.alpha.is_empty()
    }

    fn state(&self) -> &State {
        &self.states[self.states.len() - 1]
    }

    fn state_mut(&mut self) -> &mut State {
        let k = self.states.len() - 1;
        &mut self.states[k]
    }

    /// Whether it has pixels to draw on.
    fn ok(&self) -> bool {
        !self.alpha.is_empty()
    }

    /// Its pixels as RGBA (straight), for reading and saving (none made yet: transparent, or opaque black).
    fn rgba(&self) -> Vec<u8> {
        let n = self.w as usize * self.h as usize;
        if self.alpha.is_empty() {
            let mut out = vec![0u8; n * 4];
            if self.opaque {
                out.chunks_exact_mut(4).for_each(|p| p[3] = 255);
            }
            return out;
        }
        let mut out = vec![0u8; self.alpha.len() * 4];
        for (i, a) in self.alpha.iter().enumerate() {
            out[i * 4..i * 4 + 3].copy_from_slice(&self.rgb[i * 3..i * 3 + 3]);
            out[i * 4 + 3] = *a;
        }
        out
    }
}

/// The objects a canvas's state holds, for the collector.
pub fn trace(h: &Host, roots: &mut Vec<Value>) {
    for c in &h.canvases {
        for s in &c.states {
            for p in [&s.fill, &s.stroke] {
                if let Paint::Object(g) = p {
                    roots.push(Value::Obj(*g));
                }
            }
        }
    }
}

/// A canvas element's width and height, from its attributes (300 by 150 if it hasn't them, or they aren't numbers).
fn dims(h: &Host, n: NodeId) -> (u32, u32) {
    let e = h.doc.element(n);
    let read = |name: &str, d: u32| e.and_then(|e| e.attr(name)).and_then(non_negative).unwrap_or(d);
    (read("width", 300), read("height", 150))
}

/// HTML's rules for a non-negative integer.
fn non_negative(s: &str) -> Option<u32> {
    let s = s.trim_start_matches([' ', '\t', '\n', '\x0c', '\r']);
    let s = s.strip_prefix('+').unwrap_or(s);
    let digits = &s[..s.bytes().take_while(u8::is_ascii_digit).count()];
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u64>().ok().filter(|v| *v <= i32::MAX as u64).map(|v| v as u32)
}

/// The canvas of an element (its pixels made the first time they're wanted), by its place in the host's list.
fn canvas_index(h: &mut Host, n: NodeId) -> usize {
    if let Some(i) = h.canvases.iter().position(|c| c.node == n) {
        return i;
    }
    let (w, ht) = dims(h, n);
    h.canvases.push(Canvas::new(n, w, ht));
    h.canvases.len() - 1
}

/// A canvas element's width or height attribute changed (or was set by its width or height): its pixels made that
/// size, cleared.
pub fn resized(vm: &mut Vm, n: NodeId) {
    let h = host(vm);
    if let Some(i) = h.canvases.iter().position(|c| c.node == n) {
        let (w, ht) = dims(h, n);
        h.canvases[i].resize(w, ht);
        h.canvas_drawn = true;
    }
}

// ----------------------------------------------------------------------------------------------- the interfaces

pub fn init(vm: &mut Vm) {
    let cp = interface(vm, "CanvasRenderingContext2D", None, 0, None);
    attribute(vm, cp, "canvas", ctx_canvas, None);
    let methods: &[(&str, u32, js::object::NativeFn)] = &[
        ("save", 0, save),
        ("restore", 0, restore),
        ("reset", 0, reset),
        ("scale", 2, scale),
        ("rotate", 1, rotate),
        ("translate", 2, translate),
        ("transform", 6, transform),
        ("setTransform", 0, set_transform),
        ("getTransform", 0, get_transform),
        ("resetTransform", 0, reset_transform),
        ("createLinearGradient", 4, create_linear_gradient),
        ("createRadialGradient", 6, create_radial_gradient),
        ("createConicGradient", 3, create_conic_gradient),
        ("createPattern", 2, create_pattern),
        ("clearRect", 4, clear_rect),
        ("fillRect", 4, fill_rect),
        ("strokeRect", 4, stroke_rect),
        ("beginPath", 0, begin_path),
        ("fill", 0, fill),
        ("stroke", 0, stroke),
        ("clip", 0, clip),
        ("isPointInPath", 2, is_point_in_path),
        ("isPointInStroke", 2, is_point_in_stroke),
        ("fillText", 3, fill_text),
        ("strokeText", 3, stroke_text),
        ("measureText", 1, measure_text),
        ("drawImage", 3, draw_image),
        ("createImageData", 1, create_image_data),
        ("getImageData", 4, get_image_data),
        ("putImageData", 3, put_image_data),
        ("setLineDash", 1, set_line_dash),
        ("getLineDash", 0, get_line_dash),
        ("getContextAttributes", 0, context_attributes),
        ("isContextLost", 0, false_fn),
        ("drawFocusIfNeeded", 1, nothing),
        ("scrollPathIntoView", 0, nothing),
    ];
    for &(name, len, f) in methods {
        method(vm, cp, name, len, f);
    }
    path_methods(vm, cp);
    let props: &[(&str, js::object::NativeFn, js::object::NativeFn)] = &[
        ("fillStyle", fill_style, set_fill_style),
        ("strokeStyle", stroke_style, set_stroke_style),
        ("lineWidth", line_width, set_line_width),
        ("lineCap", line_cap, set_line_cap),
        ("lineJoin", line_join, set_line_join),
        ("miterLimit", miter_limit, set_miter_limit),
        ("lineDashOffset", dash_offset, set_dash_offset),
        ("globalAlpha", global_alpha, set_global_alpha),
        ("globalCompositeOperation", composite_op, set_composite_op),
        ("font", font, set_font),
        ("textAlign", text_align, set_text_align),
        ("textBaseline", text_baseline, set_text_baseline),
        ("direction", direction, set_direction),
        ("shadowBlur", shadow_blur, set_shadow_blur),
        ("shadowColor", shadow_color, set_shadow_color),
        ("shadowOffsetX", shadow_x, set_shadow_x),
        ("shadowOffsetY", shadow_y, set_shadow_y),
        ("imageSmoothingEnabled", smoothing, set_smoothing),
        ("imageSmoothingQuality", smoothing_quality, set_smoothing_quality),
        ("filter", filter, set_filter),
    ];
    for &(name, g, s) in props {
        attribute(vm, cp, name, g, Some(s));
    }
    // (Text's finer settings: kept as they're set, not drawn differently.)
    for (name, v) in [("letterSpacing", "0px"), ("wordSpacing", "0px"), ("fontKerning", "auto"), ("fontStretch", "normal"), ("fontVariantCaps", "normal"), ("textRendering", "auto"), ("lang", "inherit")] {
        vm.put(cp, name, js(v), WRITABLE | ENUMERABLE);
    }
    let gp = interface(vm, "CanvasGradient", None, 0, None);
    method(vm, gp, "addColorStop", 2, add_color_stop);
    let pp = interface(vm, "CanvasPattern", None, 0, None);
    method(vm, pp, "setTransform", 0, pattern_set_transform);
    let p2 = interface(vm, "Path2D", None, 0, Some(path2d_new));
    path_methods(vm, p2);
    method(vm, p2, "addPath", 1, add_path);
    interface(vm, "ImageData", None, 2, Some(image_data_new));
    interface(vm, "TextMetrics", None, 0, None);
    // The canvas element's own.
    if let Some(ep) = host_ref(vm).proto("HTMLCanvasElement") {
        method(vm, ep, "getContext", 1, get_context);
        method(vm, ep, "toDataURL", 0, to_data_url);
        method(vm, ep, "toBlob", 1, to_blob);
        attribute(vm, ep, "width", canvas_width, Some(set_canvas_width));
        attribute(vm, ep, "height", canvas_height, Some(set_canvas_height));
    }
}

/// The path methods a context and a Path2D both have.
fn path_methods(vm: &mut Vm, p: Gc) {
    let methods: &[(&str, u32, js::object::NativeFn)] = &[
        ("moveTo", 2, move_to),
        ("lineTo", 2, line_to),
        ("quadraticCurveTo", 4, quadratic_curve_to),
        ("bezierCurveTo", 6, bezier_curve_to),
        ("arcTo", 5, arc_to),
        ("arc", 5, arc),
        ("ellipse", 7, ellipse),
        ("rect", 4, rect),
        ("roundRect", 4, round_rect),
        ("closePath", 0, close_path),
    ];
    for &(name, len, f) in methods {
        method(vm, p, name, len, f);
    }
}

fn nothing(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Undefined)
}

fn false_fn(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Bool(false))
}

/// `this` as a context: its canvas's node.
fn ctx_node(vm: &mut Vm, this: &Value) -> Result<NodeId, Value> {
    Ok(this_host(vm, this, CONTEXT)? as NodeId)
}

/// A context's canvas (by its place in the host's list), and the host.
fn ctx(vm: &mut Vm, this: &Value) -> Result<(NodeId, usize), Value> {
    let n = ctx_node(vm, this)?;
    let i = canvas_index(host(vm), n);
    Ok((n, i))
}

fn canvas_mut(vm: &mut Vm, i: usize) -> &mut Canvas {
    &mut host(vm).canvases[i]
}

/// The first `K` arguments as numbers: an error if they weren't all given; None if one isn't finite (the call does
/// nothing then, as the standard has it).
fn numbers<const K: usize>(vm: &mut Vm, args: &[Value], what: &str) -> Result<Option<[f32; K]>, Value> {
    if args.len() < K {
        return Err(vm.type_error(&format!("Failed to execute '{what}': {K} arguments required, but only {} present.", args.len())));
    }
    let mut out = [0f32; K];
    let mut ok = true;
    for (i, o) in out.iter_mut().enumerate() {
        let v = vm.to_number(&args[i])?;
        if !v.is_finite() {
            ok = false;
        }
        *o = v as f32;
    }
    Ok(if ok { Some(out) } else { None })
}

fn num(v: f32) -> Value {
    Value::Num(v as f64)
}

// ------------------------------------------------------------------------------------------- the context element

fn get_context(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let id = str_arg(vm, args, 0, "getContext")?;
    if id != "2d" {
        return Ok(Value::Null);
    }
    if let Some(&g) = host_ref(vm).cache.get(&(n, CONTEXT_OBJ)) {
        return Ok(Value::Obj(g));
    }
    let mut opaque = false;
    if let Value::Obj(o) = arg(args, 1) {
        let a = get(vm, o, "alpha")?;
        opaque = !a.is_undefined() && !a.truthy();
    }
    let p = host_ref(vm).proto("CanvasRenderingContext2D").unwrap_or_else(|| vm.proto(js::vm::I::ObjectProto));
    let g = vm.alloc(Some(p), Class::Host(CONTEXT, n as u64));
    let h = host(vm);
    h.cache.insert((n, CONTEXT_OBJ), g);
    let i = canvas_index(h, n);
    if opaque {
        let c = &mut h.canvases[i];
        c.opaque = true;
        let (w, ht) = (c.w, c.h);
        c.resize(w, ht);
    }
    Ok(Value::Obj(g))
}

fn ctx_canvas(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = ctx_node(vm, this)?;
    Ok(wrap(vm, n))
}

fn canvas_width(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    Ok(Value::Num(dims(host_ref(vm), n).0 as f64))
}

fn canvas_height(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    Ok(Value::Num(dims(host_ref(vm), n).1 as f64))
}

fn set_canvas_width(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    set_canvas_size(vm, this, args, "width")
}

fn set_canvas_height(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    set_canvas_size(vm, this, args, "height")
}

/// Sets a canvas's width or height: cleared even when it's the size it was (pages clear canvases that way).
fn set_canvas_size(vm: &mut Vm, this: &Value, args: &[Value], which: &str) -> R {
    let n = this_element(vm, this)?;
    let v = vm.to_uint32(&arg(args, 0))?;
    let v = if v > i32::MAX as u32 { if which == "width" { 300 } else { 150 } } else { v };
    super::dom::set_attr(vm, n, which, &v.to_string());
    resized(vm, n);
    Ok(Value::Undefined)
}

/// A canvas's pixels written as a file: PNG, or JPEG (on black, at a quality from 0 to 1) if that's asked for.
fn encoded(vm: &mut Vm, n: NodeId, ty: &str, quality: Option<f64>) -> Option<(Vec<u8>, &'static str)> {
    let h = host(vm);
    let (w, ht) = dims(h, n);
    if w == 0 || ht == 0 || w as u64 * ht as u64 > MOST_PIXELS {
        return None;
    }
    let rgba = match h.canvases.iter().find(|c| c.node == n && c.w == w && c.h == ht) {
        Some(c) => c.rgba(),
        None => vec![0u8; w as usize * ht as usize * 4],
    };
    if ty.eq_ignore_ascii_case("image/jpeg") {
        let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
        for p in rgba.chunks_exact(4) {
            let a = p[3] as u32;
            rgb.extend(p[..3].iter().map(|&c| ((c as u32 * a + 127) / 255) as u8));
        }
        let q = quality.filter(|q| (0.0..=1.0).contains(q)).unwrap_or(0.92);
        let pic = picture::Picture { width: w, height: ht, rgb };
        return Some((picture::encode_jpeg(&pic, ((q * 100.0) as u8).clamp(1, 100)), "image/jpeg"));
    }
    Some((picture::encode_png(&rgba, w, ht), "image/png"))
}

fn type_and_quality(vm: &mut Vm, t: &Value, q: &Value) -> Result<(String, Option<f64>), Value> {
    let ty = if t.is_undefined() { String::from("image/png") } else { string(vm, t)? };
    let q = match q {
        Value::Num(v) => Some(*v),
        _ => None,
    };
    Ok((ty, q))
}

fn to_data_url(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let (ty, q) = type_and_quality(vm, &arg(args, 0), &arg(args, 1))?;
    Ok(match encoded(vm, n, &ty, q) {
        Some((bytes, mime)) => js(&format!("data:{mime};base64,{}", super::window::base64_encode(&bytes))),
        None => js("data:,"),
    })
}

fn to_blob(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let cb = arg(args, 0);
    if !vm.is_callable(&cb) {
        return Err(vm.type_error("Failed to execute 'toBlob' on 'HTMLCanvasElement': The callback provided as parameter 1 is not a function."));
    }
    let (ty, q) = type_and_quality(vm, &arg(args, 1), &arg(args, 2))?;
    let blob = match encoded(vm, n, &ty, q) {
        Some((bytes, mime)) => {
            let p = host_ref(vm).proto("Blob").unwrap_or_else(|| vm.proto(js::vm::I::ObjectProto));
            Value::Obj(super::window::make_blob(vm, p, bytes, mime))
        }
        None => Value::Null,
    };
    host(vm).tasks.push((cb, Value::Undefined, vec![blob]));
    Ok(Value::Undefined)
}

// --------------------------------------------------------------------------------------------------- paths

const OP_MOVE: u32 = 0;
const OP_LINE: u32 = 1;
const OP_QUAD: u32 = 2;
const OP_CUBIC: u32 = 3;
const OP_ARC: u32 = 4;
const OP_ARC_TO: u32 = 5;
const OP_CLOSE: u32 = 6;
const OP_SVG_ARC: u32 = 7;
const OP_RECT: u32 = 8;
const OP_ROUND_RECT: u32 = 9;
/// The transform the points after it go through (the context's path: its transform when they were added).
const OP_SET: u32 = 10;
/// A transform for the points after it, on top of the one they'd have, until OP_POP (Path2D's addPath).
const OP_PUSH: u32 = 11;
const OP_POP: u32 = 12;

/// How many numbers each step has.
fn arity(code: u32) -> Option<usize> {
    Some(match code {
        OP_MOVE | OP_LINE => 2,
        OP_QUAD => 4,
        OP_CUBIC | OP_SET | OP_PUSH => 6,
        OP_ARC => 8,
        OP_ARC_TO => 5,
        OP_CLOSE | OP_POP => 0,
        OP_SVG_ARC => 7,
        OP_RECT => 4,
        OP_ROUND_RECT => 12,
        _ => return None,
    })
}

/// A path's steps drawn into a builder (whose transform is `base`): lines, curves and arcs as the canvas makes them
/// (a line with no subpath to go on starts one; arcs join on with a line).
fn replay(ops: &[f32], b: &mut Builder, base: Matrix) {
    let mut stack: Vec<Matrix> = Vec::new();
    let mut m = base;
    let mut i = 0;
    while i < ops.len() {
        let code = ops[i] as u32;
        let Some(k) = arity(code) else { break };
        let Some(v) = ops.get(i + 1..i + 1 + k) else { break };
        i += 1 + k;
        let ensure = |b: &mut Builder, p: (f32, f32)| {
            if b.is_empty() {
                b.move_to(p);
            }
        };
        match code {
            OP_MOVE => b.move_to((v[0], v[1])),
            OP_LINE => {
                if b.is_empty() {
                    b.move_to((v[0], v[1]));
                } else {
                    b.line_to((v[0], v[1]));
                }
            }
            OP_QUAD => {
                ensure(b, (v[0], v[1]));
                b.quad_to((v[0], v[1]), (v[2], v[3]));
            }
            OP_CUBIC => {
                ensure(b, (v[0], v[1]));
                b.cubic_to((v[0], v[1]), (v[2], v[3]), (v[4], v[5]));
            }
            OP_ARC => b.arc((v[0], v[1]), v[2], v[3], v[4], v[5], v[6], v[7] != 0.0),
            OP_ARC_TO => arc_to_points(b, (v[0], v[1]), (v[2], v[3]), v[4]),
            OP_CLOSE => {
                if !b.is_empty() {
                    b.close();
                }
            }
            OP_SVG_ARC => {
                ensure(b, b.pen);
                b.arc_to(v[0], v[1], v[2], v[3] != 0.0, v[4] != 0.0, (v[5], v[6]));
            }
            OP_RECT => {
                let (x, y, w, h) = (v[0], v[1], v[2], v[3]);
                b.move_to((x, y));
                b.line_to((x + w, y));
                b.line_to((x + w, y + h));
                b.line_to((x, y + h));
                b.close();
            }
            OP_ROUND_RECT => {
                let (x, y, w, h) = (v[0], v[1], v[2], v[3]);
                let [tlx, tly, trx, try_, brx, bry, blx, bly] = [v[4], v[5], v[6], v[7], v[8], v[9], v[10], v[11]];
                let pi = core::f32::consts::PI;
                b.move_to((x + tlx, y));
                b.line_to((x + w - trx, y));
                b.arc((x + w - trx, y + try_), trx, try_, 0.0, -pi / 2.0, 0.0, false);
                b.line_to((x + w, y + h - bry));
                b.arc((x + w - brx, y + h - bry), brx, bry, 0.0, 0.0, pi / 2.0, false);
                b.line_to((x + blx, y + h));
                b.arc((x + blx, y + h - bly), blx, bly, 0.0, pi / 2.0, pi, false);
                b.line_to((x, y + tly));
                b.arc((x + tlx, y + tly), tlx, tly, 0.0, pi, 1.5 * pi, false);
                b.close();
                b.move_to((x, y));
            }
            OP_SET => {
                m = Matrix([v[0], v[1], v[2], v[3], v[4], v[5]]);
                b.set_matrix(m);
            }
            OP_PUSH => {
                stack.push(m);
                m = m.then(&Matrix([v[0], v[1], v[2], v[3], v[4], v[5]]));
                b.set_matrix(m);
            }
            OP_POP => {
                if let Some(p) = stack.pop() {
                    m = p;
                    b.set_matrix(m);
                }
            }
            _ => break,
        }
    }
}

/// arcTo: a line to where a circle of radius `r` touching both lines (from the pen through `p1`, and from `p1` to
/// `p2`) meets the first, then the arc of it to where it meets the second.
fn arc_to_points(b: &mut Builder, p1: (f32, f32), p2: (f32, f32), r: f32) {
    if b.is_empty() {
        b.move_to(p1);
    }
    let p0 = b.pen;
    let (v1, v2) = ((p0.0 - p1.0, p0.1 - p1.1), (p2.0 - p1.0, p2.1 - p1.1));
    let (l1, l2) = (draw::libm_sqrt(v1.0 * v1.0 + v1.1 * v1.1), draw::libm_sqrt(v2.0 * v2.0 + v2.1 * v2.1));
    let cross = v1.0 * v2.1 - v1.1 * v2.0;
    if l1 < 1e-6 || l2 < 1e-6 || r == 0.0 || cross.abs() <= 1e-6 * l1 * l2 {
        b.line_to(p1);
        return;
    }
    let (u1, u2) = ((v1.0 / l1, v1.1 / l1), (v2.0 / l2, v2.1 / l2));
    let cos = (u1.0 * u2.0 + u1.1 * u2.1).clamp(-1.0, 1.0);
    // How far along each line the circle touches it: r / tan(half the angle between them).
    let d = r * draw::libm_sqrt((1.0 + cos) / (1.0 - cos));
    let (t1, t2) = ((p1.0 + u1.0 * d, p1.1 + u1.1 * d), (p1.0 + u2.0 * d, p1.1 + u2.1 * d));
    let Some(bis) = unit((u1.0 + u2.0, u1.1 + u2.1)) else {
        b.line_to(p1);
        return;
    };
    let dc = draw::libm_sqrt(r * r + d * d);
    let c = (p1.0 + bis.0 * dc, p1.1 + bis.1 * dc);
    let a0 = draw::atan2(t1.1 - c.1, t1.0 - c.0);
    let a1 = draw::atan2(t2.1 - c.1, t2.0 - c.0);
    let pi = core::f32::consts::PI;
    let mut delta = a1 - a0;
    while delta > pi {
        delta -= 2.0 * pi;
    }
    while delta <= -pi {
        delta += 2.0 * pi;
    }
    b.line_to(t1);
    b.arc(c, r, r, 0.0, a0, a0 + delta, delta < 0.0);
}

fn unit(v: (f32, f32)) -> Option<(f32, f32)> {
    let l = draw::libm_sqrt(v.0 * v.0 + v.1 * v.1);
    if l < 1e-9 { None } else { Some((v.0 / l, v.1 / l)) }
}

/// Where a path method adds its step: a context's path, or a Path2D.
enum Target {
    Ctx(NodeId),
    Path(Gc),
}

fn path_target(vm: &mut Vm, this: &Value) -> Result<Target, Value> {
    if let Value::Obj(g) = this {
        match vm.heap.get(*g).class {
            Class::Host(CONTEXT, n) => return Ok(Target::Ctx(n as NodeId)),
            Class::Host(PATH2D, _) => return Ok(Target::Path(*g)),
            _ => {}
        }
    }
    Err(vm.type_error("Illegal invocation"))
}

/// The bytes an object of the canvas's keeps in its private slot.
fn data_of(vm: &Vm, g: Gc) -> Option<&Vec<u8>> {
    let s = host_ref(vm).syms.data;
    match slot(vm, &Value::Obj(g), s) {
        Value::Obj(b) => match &vm.heap.get(b).class {
            Class::ArrayBuffer(Some(v), _) => Some(v),
            _ => None,
        },
        _ => None,
    }
}

fn data_mut(vm: &mut Vm, g: Gc) -> Option<&mut Vec<u8>> {
    let s = host_ref(vm).syms.data;
    match slot(vm, &Value::Obj(g), s) {
        Value::Obj(b) => match &mut vm.heap.get_mut(b).class {
            Class::ArrayBuffer(Some(v), _) => Some(v),
            _ => None,
        },
        _ => None,
    }
}

fn set_data(vm: &mut Vm, g: Gc, bytes: Vec<u8>) {
    let buf = super::window::array_buffer(vm, bytes);
    let s = host_ref(vm).syms.data;
    set_slot(vm, g, s, buf);
}

fn f32s(bytes: &[u8]) -> Vec<f32> {
    bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

fn push_f32s(bytes: &mut Vec<u8>, v: &[f32]) {
    for x in v {
        bytes.extend_from_slice(&x.to_le_bytes());
    }
}

/// A Path2D's steps.
fn path2d_ops(vm: &Vm, g: Gc) -> Vec<f32> {
    data_of(vm, g).map(|b| f32s(b)).unwrap_or_default()
}

/// Adds a step to a path.
fn add_op(vm: &mut Vm, t: &Target, code: u32, v: &[f32]) {
    match t {
        Target::Ctx(n) => {
            let h = host(vm);
            let i = canvas_index(h, *n);
            let c = &mut h.canvases[i];
            if c.path.len() + 8 + v.len() > MOST_PATH {
                return;
            }
            let m = c.state().m;
            if c.path_m.is_none_or(|pm| pm.0 != m.0) {
                c.path.push(OP_SET as f32);
                c.path.extend_from_slice(&m.0);
                c.path_m = Some(m);
            }
            c.path.push(code as f32);
            c.path.extend_from_slice(v);
        }
        Target::Path(g) => {
            if let Some(b) = data_mut(vm, *g)
                && b.len() / 4 + 1 + v.len() <= MOST_PATH
            {
                push_f32s(b, &[code as f32]);
                push_f32s(b, v);
            }
        }
    }
}

fn path_call<const K: usize>(vm: &mut Vm, this: &Value, args: &[Value], what: &str, code: u32) -> R {
    let t = path_target(vm, this)?;
    let Some(v) = numbers::<K>(vm, args, what)? else { return Ok(Value::Undefined) };
    add_op(vm, &t, code, &v);
    Ok(Value::Undefined)
}

fn move_to(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    path_call::<2>(vm, this, args, "moveTo", OP_MOVE)
}

fn line_to(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    path_call::<2>(vm, this, args, "lineTo", OP_LINE)
}

fn quadratic_curve_to(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    path_call::<4>(vm, this, args, "quadraticCurveTo", OP_QUAD)
}

fn bezier_curve_to(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    path_call::<6>(vm, this, args, "bezierCurveTo", OP_CUBIC)
}

fn rect(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    path_call::<4>(vm, this, args, "rect", OP_RECT)
}

fn close_path(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let t = path_target(vm, this)?;
    add_op(vm, &t, OP_CLOSE, &[]);
    Ok(Value::Undefined)
}

fn arc_to(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let t = path_target(vm, this)?;
    let Some(v) = numbers::<5>(vm, args, "arcTo")? else { return Ok(Value::Undefined) };
    if v[4] < 0.0 {
        return Err(dom_exception(vm, "IndexSizeError", &format!("Failed to execute 'arcTo': The radius provided ({}) is negative.", v[4])));
    }
    add_op(vm, &t, OP_ARC_TO, &v);
    Ok(Value::Undefined)
}

fn arc(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let t = path_target(vm, this)?;
    let Some(v) = numbers::<5>(vm, args, "arc")? else { return Ok(Value::Undefined) };
    if v[2] < 0.0 {
        return Err(dom_exception(vm, "IndexSizeError", &format!("Failed to execute 'arc': The radius provided ({}) is negative.", v[2])));
    }
    let ccw = arg(args, 5).truthy();
    add_op(vm, &t, OP_ARC, &[v[0], v[1], v[2], v[2], 0.0, v[3], v[4], if ccw { 1.0 } else { 0.0 }]);
    Ok(Value::Undefined)
}

fn ellipse(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let t = path_target(vm, this)?;
    let Some(v) = numbers::<7>(vm, args, "ellipse")? else { return Ok(Value::Undefined) };
    if v[2] < 0.0 || v[3] < 0.0 {
        let which = if v[2] < 0.0 { "major-axis" } else { "minor-axis" };
        return Err(dom_exception(vm, "IndexSizeError", &format!("Failed to execute 'ellipse': The {which} radius provided is negative.")));
    }
    let ccw = arg(args, 7).truthy();
    add_op(vm, &t, OP_ARC, &[v[0], v[1], v[2], v[3], v[4], v[5], v[6], if ccw { 1.0 } else { 0.0 }]);
    Ok(Value::Undefined)
}

/// roundRect's radii: a number, a point ({x, y}) or a list of one to four of them, for the four corners (top
/// left, top right, bottom right, bottom left), each (x, y).
fn radii(vm: &mut Vm, v: &Value) -> Result<Option<[(f32, f32); 4]>, Value> {
    let list: Vec<Value> = match v {
        Value::Undefined => vec![Value::Num(0.0)],
        Value::Obj(g) if matches!(vm.heap.get(*g).class, Class::Array(..)) => {
            let len = get(vm, *g, "length")?;
            let len = vm.to_number(&len)? as usize;
            let mut out = Vec::new();
            for k in 0..len.min(5) {
                out.push(vm.get(*g, Key::Index(k as u32), &Value::Obj(*g))?);
            }
            out
        }
        other => vec![other.clone()],
    };
    if list.is_empty() || list.len() > 4 {
        return Err(vm.range_error(&format!("Failed to execute 'roundRect': {} radii provided. Between one and four radii are necessary.", list.len())));
    }
    let mut rs: Vec<(f32, f32)> = Vec::new();
    let mut finite = true;
    for r in &list {
        let (x, y) = match r {
            Value::Obj(g) if !matches!(vm.heap.get(*g).class, Class::Number(_)) => {
                let x = get(vm, *g, "x")?;
                let y = get(vm, *g, "y")?;
                let x = if x.is_undefined() { 0.0 } else { vm.to_number(&x)? };
                let y = if y.is_undefined() { 0.0 } else { vm.to_number(&y)? };
                (x, y)
            }
            v => {
                let n = vm.to_number(v)?;
                (n, n)
            }
        };
        if !x.is_finite() || !y.is_finite() {
            finite = false;
        } else if x < 0.0 || y < 0.0 {
            return Err(vm.range_error("Failed to execute 'roundRect': A radius provided is negative."));
        }
        rs.push((x as f32, y as f32));
    }
    if !finite {
        return Ok(None);
    }
    Ok(Some(match rs.len() {
        1 => [rs[0]; 4],
        2 => [rs[0], rs[1], rs[0], rs[1]],
        3 => [rs[0], rs[1], rs[2], rs[1]],
        _ => [rs[0], rs[1], rs[2], rs[3]],
    }))
}

fn round_rect(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let t = path_target(vm, this)?;
    let Some([mut x, mut y, mut w, mut h]) = numbers::<4>(vm, args, "roundRect")? else { return Ok(Value::Undefined) };
    let Some(mut r) = radii(vm, &arg(args, 4))? else { return Ok(Value::Undefined) };
    // (A negative width or height: the rectangle flipped, its corners with it.)
    if w < 0.0 {
        x += w;
        w = -w;
        r = [r[1], r[0], r[3], r[2]];
    }
    if h < 0.0 {
        y += h;
        h = -h;
        r = [r[3], r[2], r[1], r[0]];
    }
    // (Corners too big for the sides: all made smaller alike.)
    let mut k: f32 = 1.0;
    for (a, b, side) in [(r[0].0, r[1].0, w), (r[3].0, r[2].0, w), (r[0].1, r[3].1, h), (r[1].1, r[2].1, h)] {
        if a + b > side && a + b > 0.0 {
            k = k.min(side / (a + b));
        }
    }
    let r = r.map(|(a, b)| (a * k, b * k));
    add_op(vm, &t, OP_ROUND_RECT, &[x, y, w, h, r[0].0, r[0].1, r[1].0, r[1].1, r[2].0, r[2].1, r[3].0, r[3].1]);
    Ok(Value::Undefined)
}

fn begin_path(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (_, i) = ctx(vm, this)?;
    let c = canvas_mut(vm, i);
    c.path.clear();
    c.path_m = None;
    Ok(Value::Undefined)
}

// ------------------------------------------------------------------------------------------------- Path2D

fn path2d_new(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let p = super::events::proto_from(vm, nt, "Path2D")?;
    let g = vm.alloc(Some(p), Class::Host(PATH2D, 0));
    let ops: Vec<f32> = match arg(args, 0) {
        Value::Undefined => Vec::new(),
        Value::Obj(o) if matches!(vm.heap.get(o).class, Class::Host(PATH2D, _)) => path2d_ops(vm, o),
        v => {
            let d = string(vm, &v)?;
            svg_ops(&d)
        }
    };
    let mut bytes = Vec::with_capacity(ops.len() * 4);
    push_f32s(&mut bytes, &ops);
    set_data(vm, g, bytes);
    Ok(Value::Obj(g))
}

/// SVG's path data as a path's steps.
fn svg_ops(d: &str) -> Vec<f32> {
    use picture::svg::PathOp;
    let mut out = Vec::new();
    for op in picture::svg::path_ops(d) {
        match op {
            PathOp::Move(p) => out.extend_from_slice(&[OP_MOVE as f32, p.0, p.1]),
            PathOp::Line(p) => out.extend_from_slice(&[OP_LINE as f32, p.0, p.1]),
            PathOp::Quad(c, p) => out.extend_from_slice(&[OP_QUAD as f32, c.0, c.1, p.0, p.1]),
            PathOp::Cubic(c1, c2, p) => out.extend_from_slice(&[OP_CUBIC as f32, c1.0, c1.1, c2.0, c2.1, p.0, p.1]),
            PathOp::Arc { rx, ry, rot, large, sweep, to } => {
                out.extend_from_slice(&[OP_SVG_ARC as f32, rx, ry, rot, if large { 1.0 } else { 0.0 }, if sweep { 1.0 } else { 0.0 }, to.0, to.1])
            }
            PathOp::Close => out.push(OP_CLOSE as f32),
        }
    }
    out
}

/// A DOMMatrix2DInit's numbers (a b c d e f, or m11 m12 m21 m22 m41 m42), the identity for what isn't given.
fn matrix_init(vm: &mut Vm, v: &Value) -> Result<Option<Matrix>, Value> {
    let Value::Obj(g) = v else { return Ok(if v.is_undefined() || matches!(v, Value::Null) { Some(Matrix::ID) } else { None }) };
    let mut out = [1.0f32, 0.0, 0.0, 1.0, 0.0, 0.0];
    for (k, (a, m)) in [("a", "m11"), ("b", "m12"), ("c", "m21"), ("d", "m22"), ("e", "m41"), ("f", "m42")].iter().enumerate() {
        let mut x = get(vm, *g, a)?;
        if x.is_undefined() {
            x = get(vm, *g, m)?;
        }
        if !x.is_undefined() {
            let n = vm.to_number(&x)?;
            if !n.is_finite() {
                return Ok(None);
            }
            out[k] = n as f32;
        }
    }
    Ok(Some(Matrix(out)))
}

fn add_path(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let Target::Path(g) = path_target(vm, this)? else { return Err(vm.type_error("Illegal invocation")) };
    let other = match arg(args, 0) {
        Value::Obj(o) if matches!(vm.heap.get(o).class, Class::Host(PATH2D, _)) => o,
        _ => return Err(vm.type_error("Failed to execute 'addPath' on 'Path2D': parameter 1 is not of type 'Path2D'.")),
    };
    let ops = path2d_ops(vm, other);
    let m = matrix_init(vm, &arg(args, 1))?;
    let Some(m) = m else { return Ok(Value::Undefined) };
    let t = Target::Path(g);
    add_op(vm, &t, OP_PUSH, &m.0);
    if let Some(b) = data_mut(vm, g)
        && b.len() / 4 + ops.len() <= MOST_PATH
    {
        push_f32s(b, &ops);
    }
    add_op(vm, &t, OP_POP, &[]);
    Ok(Value::Undefined)
}

// --------------------------------------------------------------------------------------------------- state

fn save(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (_, i) = ctx(vm, this)?;
    let c = canvas_mut(vm, i);
    if c.states.len() < MOST_STATES {
        let s = c.state().clone();
        c.states.push(s);
    }
    Ok(Value::Undefined)
}

fn restore(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (_, i) = ctx(vm, this)?;
    let c = canvas_mut(vm, i);
    if c.states.len() > 1 {
        c.states.pop();
    }
    Ok(Value::Undefined)
}

fn reset(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (_, i) = ctx(vm, this)?;
    let c = canvas_mut(vm, i);
    let (w, h) = (c.w, c.h);
    c.resize(w, h);
    host(vm).canvas_drawn = true;
    Ok(Value::Undefined)
}

/// The context's transform, changed.
fn change_matrix(vm: &mut Vm, this: &Value, f: impl FnOnce(Matrix) -> Matrix) -> R {
    let (_, i) = ctx(vm, this)?;
    let c = canvas_mut(vm, i);
    let m = f(c.state().m);
    c.state_mut().m = m;
    Ok(Value::Undefined)
}

fn scale(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let Some([x, y]) = numbers::<2>(vm, args, "scale")? else { return Ok(Value::Undefined) };
    change_matrix(vm, this, |m| m.then(&Matrix::scale(x, y)))
}

fn rotate(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let Some([a]) = numbers::<1>(vm, args, "rotate")? else { return Ok(Value::Undefined) };
    let (s, c) = draw::sin_cos(a);
    change_matrix(vm, this, |m| m.then(&Matrix([c, s, -s, c, 0.0, 0.0])))
}

fn translate(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let Some([x, y]) = numbers::<2>(vm, args, "translate")? else { return Ok(Value::Undefined) };
    change_matrix(vm, this, |m| m.then(&Matrix::translate(x, y)))
}

fn transform(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let Some(v) = numbers::<6>(vm, args, "transform")? else { return Ok(Value::Undefined) };
    change_matrix(vm, this, |m| m.then(&Matrix(v)))
}

fn set_transform(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    if args.len() >= 6 {
        let Some(v) = numbers::<6>(vm, args, "setTransform")? else { return Ok(Value::Undefined) };
        return change_matrix(vm, this, |_| Matrix(v));
    }
    let Some(m) = matrix_init(vm, &arg(args, 0))? else { return Ok(Value::Undefined) };
    change_matrix(vm, this, |_| m)
}

fn reset_transform(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    change_matrix(vm, this, |_| Matrix::ID)
}

fn get_transform(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (_, i) = ctx(vm, this)?;
    let m = canvas_mut(vm, i).state().m;
    Ok(Value::Obj(matrix_object(vm, m)))
}

/// A DOMMatrix with a transform's numbers (its 2D ones; the rest as the identity has them).
fn matrix_object(vm: &mut Vm, m: Matrix) -> Gc {
    let w = host_ref(vm).window;
    let p = match get(vm, w, "DOMMatrix") {
        Ok(Value::Obj(c)) => match get(vm, c, "prototype") {
            Ok(Value::Obj(p)) => Some(p),
            _ => None,
        },
        _ => None,
    };
    let g = match p {
        Some(p) => vm.alloc(Some(p), Class::Ordinary),
        None => vm.new_object(),
    };
    let [a, b, c, d, e, f] = m.0.map(|v| v as f64);
    for (name, v) in [("a", a), ("b", b), ("c", c), ("d", d), ("e", e), ("f", f), ("m11", a), ("m12", b), ("m21", c), ("m22", d), ("m41", e), ("m42", f)] {
        vm.put(g, name, Value::Num(v), ENUMERABLE | WRITABLE);
    }
    for (name, v) in [("m13", 0.0), ("m14", 0.0), ("m23", 0.0), ("m24", 0.0), ("m31", 0.0), ("m32", 0.0), ("m33", 1.0), ("m34", 0.0), ("m43", 0.0), ("m44", 1.0)] {
        vm.put(g, name, Value::Num(v), ENUMERABLE | WRITABLE);
    }
    vm.put(g, "is2D", Value::Bool(true), ENUMERABLE);
    vm.put(g, "isIdentity", Value::Bool(m.0 == Matrix::ID.0), ENUMERABLE);
    g
}

// ---------------------------------------------------------------------------------------- the state's values

/// A colour as the canvas gives it back: #rrggbb, or rgba(r, g, b, a) if it's see-through.
fn color_text(c: Rgba) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let (r, g, bl, a) = (b(c[0]), b(c[1]), b(c[2]), b(c[3]));
    if a == 255 {
        return format!("#{r:02x}{g:02x}{bl:02x}");
    }
    // (The alpha in as few decimals as give the same 8-bit value back.)
    let mut alpha = js::math::round(a as f64 / 255.0 * 100.0) / 100.0;
    if (alpha * 255.0 + 0.5) as u8 != a {
        alpha = js::math::round(a as f64 / 255.0 * 1000.0) / 1000.0;
    }
    format!("rgba({r}, {g}, {bl}, {})", js::value::number_to_string(alpha))
}

fn style_get(vm: &mut Vm, this: &Value, stroke: bool) -> R {
    let (_, i) = ctx(vm, this)?;
    let st = canvas_mut(vm, i).state();
    Ok(match if stroke { &st.stroke } else { &st.fill } {
        Paint::Color(c) => js(&color_text(*c)),
        Paint::Object(g) => Value::Obj(*g),
    })
}

fn style_set(vm: &mut Vm, this: &Value, args: &[Value], stroke: bool) -> R {
    let (_, i) = ctx(vm, this)?;
    let v = arg(args, 0);
    let paint = match &v {
        Value::Obj(g) if matches!(vm.heap.get(*g).class, Class::Host(GRADIENT | PATTERN, _)) => Some(Paint::Object(*g)),
        _ => {
            let s = string(vm, &v)?;
            if s.trim().eq_ignore_ascii_case("currentcolor") { Some(Paint::Color([0.0, 0.0, 0.0, 1.0])) } else { draw::color(&s).map(Paint::Color) }
        }
    };
    if let Some(p) = paint {
        let st = canvas_mut(vm, i).state_mut();
        if stroke {
            st.stroke = p;
        } else {
            st.fill = p;
        }
    }
    Ok(Value::Undefined)
}

fn fill_style(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    style_get(vm, this, false)
}

fn set_fill_style(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    style_set(vm, this, args, false)
}

fn stroke_style(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    style_get(vm, this, true)
}

fn set_stroke_style(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    style_set(vm, this, args, true)
}

/// A state's value, read.
fn read(vm: &mut Vm, this: &Value, f: impl FnOnce(&State) -> Value) -> R {
    let (_, i) = ctx(vm, this)?;
    Ok(f(canvas_mut(vm, i).state()))
}

/// A number set (ignored if it isn't finite, or `ok` says it isn't one it takes).
fn set_number(vm: &mut Vm, this: &Value, args: &[Value], ok: fn(f64) -> bool, f: fn(&mut State, f32)) -> R {
    let (_, i) = ctx(vm, this)?;
    let v = vm.to_number(&arg(args, 0))?;
    if v.is_finite() && ok(v) {
        f(canvas_mut(vm, i).state_mut(), v as f32);
    }
    Ok(Value::Undefined)
}

/// One of a list of words set (ignored if it isn't one of them): its place in the list.
fn set_word(vm: &mut Vm, this: &Value, args: &[Value], words: &[&str], f: fn(&mut State, u8)) -> R {
    let (_, i) = ctx(vm, this)?;
    let s = string(vm, &arg(args, 0))?;
    if let Some(k) = words.iter().position(|w| *w == s) {
        f(canvas_mut(vm, i).state_mut(), k as u8);
    }
    Ok(Value::Undefined)
}

const CAPS: &[&str] = &["butt", "round", "square"];
const JOINS: &[&str] = &["miter", "round", "bevel"];
const ALIGNS: &[&str] = &["start", "end", "left", "right", "center"];
const BASELINES: &[&str] = &["alphabetic", "top", "hanging", "middle", "ideographic", "bottom"];
const DIRECTIONS: &[&str] = &["inherit", "ltr", "rtl"];
const QUALITIES: &[&str] = &["low", "medium", "high"];

fn line_width(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    read(vm, this, |s| num(s.line_width))
}
fn set_line_width(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    set_number(vm, this, args, |v| v > 0.0, |s, v| s.line_width = v)
}
fn line_cap(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    read(vm, this, |s| js(CAPS[s.cap as usize]))
}
fn set_line_cap(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    set_word(vm, this, args, CAPS, |s, k| s.cap = k)
}
fn line_join(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    read(vm, this, |s| js(JOINS[s.join as usize]))
}
fn set_line_join(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    set_word(vm, this, args, JOINS, |s, k| s.join = k)
}
fn miter_limit(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    read(vm, this, |s| num(s.miter))
}
fn set_miter_limit(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    set_number(vm, this, args, |v| v > 0.0, |s, v| s.miter = v)
}
fn dash_offset(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    read(vm, this, |s| num(s.dash_offset))
}
fn set_dash_offset(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    set_number(vm, this, args, |_| true, |s, v| s.dash_offset = v)
}
fn global_alpha(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    read(vm, this, |s| num(s.alpha))
}
fn set_global_alpha(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    set_number(vm, this, args, |v| (0.0..=1.0).contains(&v), |s, v| s.alpha = v)
}
fn composite_op(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    read(vm, this, |s| js(OPS.iter().find(|(_, o)| *o == s.op).map_or("source-over", |(n, _)| n)))
}
fn set_composite_op(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (_, i) = ctx(vm, this)?;
    let s = string(vm, &arg(args, 0))?;
    if let Some((_, o)) = OPS.iter().find(|(n, _)| *n == s) {
        canvas_mut(vm, i).state_mut().op = *o;
    }
    Ok(Value::Undefined)
}
fn text_align(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    read(vm, this, |s| js(ALIGNS[s.align as usize]))
}
fn set_text_align(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    set_word(vm, this, args, ALIGNS, |s, k| s.align = k)
}
fn text_baseline(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    read(vm, this, |s| js(BASELINES[s.baseline as usize]))
}
fn set_text_baseline(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    set_word(vm, this, args, BASELINES, |s, k| s.baseline = k)
}
fn direction(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    read(vm, this, |s| js(DIRECTIONS[s.direction as usize]))
}
fn set_direction(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    set_word(vm, this, args, DIRECTIONS, |s, k| s.direction = k)
}
fn shadow_blur(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    read(vm, this, |s| num(s.shadow_blur))
}
fn set_shadow_blur(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    set_number(vm, this, args, |v| v >= 0.0, |s, v| s.shadow_blur = v)
}
fn shadow_color(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    read(vm, this, |s| js(&color_text(s.shadow_color)))
}
fn set_shadow_color(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (_, i) = ctx(vm, this)?;
    let s = string(vm, &arg(args, 0))?;
    if let Some(c) = draw::color(&s) {
        canvas_mut(vm, i).state_mut().shadow_color = c;
    }
    Ok(Value::Undefined)
}
fn shadow_x(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    read(vm, this, |s| num(s.shadow_offset.0))
}
fn set_shadow_x(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    set_number(vm, this, args, |_| true, |s, v| s.shadow_offset.0 = v)
}
fn shadow_y(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    read(vm, this, |s| num(s.shadow_offset.1))
}
fn set_shadow_y(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    set_number(vm, this, args, |_| true, |s, v| s.shadow_offset.1 = v)
}
fn smoothing(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    read(vm, this, |s| Value::Bool(s.smoothing))
}
fn set_smoothing(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (_, i) = ctx(vm, this)?;
    let v = arg(args, 0).truthy();
    canvas_mut(vm, i).state_mut().smoothing = v;
    Ok(Value::Undefined)
}
fn smoothing_quality(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    read(vm, this, |s| js(QUALITIES[s.smoothing_quality as usize]))
}
fn set_smoothing_quality(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    set_word(vm, this, args, QUALITIES, |s, k| s.smoothing_quality = k)
}
fn filter(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    read(vm, this, |s| js(&s.filter))
}
fn set_filter(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (_, i) = ctx(vm, this)?;
    let s = string(vm, &arg(args, 0))?;
    canvas_mut(vm, i).state_mut().filter = s;
    Ok(Value::Undefined)
}

fn set_line_dash(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (_, i) = ctx(vm, this)?;
    let Value::Obj(g) = arg(args, 0) else {
        return Err(vm.type_error("Failed to execute 'setLineDash' on 'CanvasRenderingContext2D': The provided value cannot be converted to a sequence."));
    };
    let len = get(vm, g, "length")?;
    let len = vm.to_number(&len)?.clamp(0.0, 1024.0) as usize;
    let mut dash = Vec::with_capacity(len);
    for k in 0..len {
        let v = vm.get(g, Key::Index(k as u32), &Value::Obj(g))?;
        let v = vm.to_number(&v)?;
        if !v.is_finite() || v < 0.0 {
            return Ok(Value::Undefined);
        }
        dash.push(v as f32);
    }
    if dash.len() % 2 == 1 {
        let again = dash.clone();
        dash.extend(again);
    }
    canvas_mut(vm, i).state_mut().dash = dash;
    Ok(Value::Undefined)
}

fn get_line_dash(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (_, i) = ctx(vm, this)?;
    let dash: Vec<Value> = canvas_mut(vm, i).state().dash.iter().map(|d| num(*d)).collect();
    Ok(Value::Obj(vm.new_array(dash)))
}

fn context_attributes(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (_, i) = ctx(vm, this)?;
    let opaque = canvas_mut(vm, i).opaque;
    let o = vm.new_object();
    vm.put(o, "alpha", Value::Bool(!opaque), js::object::ALL);
    vm.put(o, "colorSpace", js("srgb"), js::object::ALL);
    vm.put(o, "desynchronized", Value::Bool(false), js::object::ALL);
    vm.put(o, "willReadFrequently", Value::Bool(false), js::object::ALL);
    Ok(Value::Obj(o))
}

// ------------------------------------------------------------------------------------------------------ fonts

/// CSS's font shorthand, as a canvas reads it: [style] [variant] [weight] [stretch] size[/line-height] family.
fn parse_font(s: &str) -> Option<Font> {
    let text = s.trim();
    let (mut italic, mut bold) = (false, false);
    let mut rest = text;
    let size = loop {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let (tok, after) = (&rest[..end], rest[end..].trim_start());
        if tok.is_empty() {
            return None;
        }
        let lower = tok.to_ascii_lowercase();
        match lower.as_str() {
            "italic" | "oblique" => italic = true,
            "bold" | "bolder" => bold = true,
            "normal" | "small-caps" | "lighter" | "ultra-condensed" | "extra-condensed" | "condensed" | "semi-condensed" | "semi-expanded" | "expanded"
            | "extra-expanded" | "ultra-expanded" => {}
            _ => {
                if let Ok(w) = lower.parse::<f32>() {
                    if !(1.0..=1000.0).contains(&w) {
                        return None;
                    }
                    bold = w >= 600.0;
                } else {
                    // The size (and perhaps a line height after it).
                    let size_text = lower.split('/').next().unwrap_or("");
                    let px = font_size(size_text)?;
                    rest = after;
                    break px;
                }
            }
        }
        rest = after;
    };
    // (A line height written apart from the size: "12px / 2 serif".)
    let mut family = rest.trim();
    if let Some(f) = family.strip_prefix('/') {
        let f = f.trim_start();
        family = f.find(char::is_whitespace).map_or("", |e| f[e..].trim_start());
    }
    if family.is_empty() || matches!(family.to_ascii_lowercase().as_str(), "inherit" | "initial" | "unset" | "revert") {
        return None;
    }
    let (mono, sans) = crate::style::kind_of_families(family);
    let face = match (mono, sans, bold, italic) {
        (true, ..) => Face::Mono,
        (_, true, true, true) => Face::SansBoldItalic,
        (_, true, true, false) => Face::SansBold,
        (_, true, false, true) => Face::SansItalic,
        (_, true, false, false) => Face::Sans,
        (_, false, true, true) => Face::BoldItalic,
        (_, false, true, false) => Face::Bold,
        (_, false, false, true) => Face::Italic,
        (_, false, false, false) => Face::Regular,
    };
    let mut out = String::new();
    if italic {
        out.push_str("italic ");
    }
    if bold {
        out.push_str("bold ");
    }
    out.push_str(&format!("{}px {family}", js::value::number_to_string(size as f64)));
    Some(Font { text: out, size, face })
}

/// A font size in pixels (as the canvas has it: em and percent of the 16 pixels a page's text is).
fn font_size(s: &str) -> Option<f32> {
    let keyword = match s {
        "xx-small" => Some(9.0),
        "x-small" => Some(10.0),
        "small" => Some(13.0),
        "medium" => Some(16.0),
        "large" => Some(18.0),
        "x-large" => Some(24.0),
        "xx-large" => Some(32.0),
        "xxx-large" => Some(48.0),
        _ => None,
    };
    if keyword.is_some() {
        return keyword;
    }
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+' || c == 'e')).unwrap_or(s.len());
    let (n, unit) = (&s[..split], &s[split..]);
    let n: f32 = n.parse().ok()?;
    let px = match unit {
        "px" => n,
        "pt" => n * 4.0 / 3.0,
        "pc" => n * 16.0,
        "in" => n * 96.0,
        "cm" => n * 96.0 / 2.54,
        "mm" => n * 96.0 / 25.4,
        "q" => n * 96.0 / 101.6,
        "em" | "rem" => n * 16.0,
        "%" => n * 0.16,
        "" if n == 0.0 => 0.0,
        _ => return None,
    };
    if px < 0.0 || !px.is_finite() { None } else { Some(px) }
}

fn font(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    read(vm, this, |s| js(&s.font.text))
}

fn set_font(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (_, i) = ctx(vm, this)?;
    let s = string(vm, &arg(args, 0))?;
    if let Some(f) = parse_font(&s) {
        canvas_mut(vm, i).state_mut().font = f;
    }
    Ok(Value::Undefined)
}

// ------------------------------------------------------------------------------------------ gradients, patterns

fn gradient_object(vm: &mut Vm, kind: f32, p: [f32; 6]) -> Value {
    let proto = host_ref(vm).proto("CanvasGradient").unwrap_or_else(|| vm.proto(js::vm::I::ObjectProto));
    let g = vm.alloc(Some(proto), Class::Host(GRADIENT, 0));
    let mut bytes = Vec::new();
    push_f32s(&mut bytes, &[kind]);
    push_f32s(&mut bytes, &p);
    set_data(vm, g, bytes);
    Value::Obj(g)
}

fn finite_args<const K: usize>(vm: &mut Vm, args: &[Value], what: &str) -> Result<[f32; K], Value> {
    match numbers::<K>(vm, args, what)? {
        Some(v) => Ok(v),
        None => Err(vm.type_error(&format!("Failed to execute '{what}' on 'CanvasRenderingContext2D': The provided double value is non-finite."))),
    }
}

fn create_linear_gradient(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    ctx_node(vm, this)?;
    let [x0, y0, x1, y1] = finite_args::<4>(vm, args, "createLinearGradient")?;
    Ok(gradient_object(vm, 0.0, [x0, y0, x1, y1, 0.0, 0.0]))
}

fn create_radial_gradient(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    ctx_node(vm, this)?;
    let v = finite_args::<6>(vm, args, "createRadialGradient")?;
    if v[2] < 0.0 || v[5] < 0.0 {
        return Err(dom_exception(vm, "IndexSizeError", "Failed to execute 'createRadialGradient' on 'CanvasRenderingContext2D': A radius provided is negative."));
    }
    Ok(gradient_object(vm, 1.0, v))
}

fn create_conic_gradient(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    ctx_node(vm, this)?;
    let [a, x, y] = finite_args::<3>(vm, args, "createConicGradient")?;
    Ok(gradient_object(vm, 2.0, [a, x, y, 0.0, 0.0, 0.0]))
}

fn add_color_stop(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let g = match this {
        Value::Obj(g) if matches!(vm.heap.get(*g).class, Class::Host(GRADIENT, _)) => *g,
        _ => return Err(vm.type_error("Illegal invocation")),
    };
    if args.len() < 2 {
        return Err(vm.type_error("Failed to execute 'addColorStop' on 'CanvasGradient': 2 arguments required."));
    }
    let offset = vm.to_number(&args[0])?;
    if !offset.is_finite() {
        return Err(vm.type_error("Failed to execute 'addColorStop' on 'CanvasGradient': The provided double value is non-finite."));
    }
    if !(0.0..=1.0).contains(&offset) {
        return Err(dom_exception(vm, "IndexSizeError", &format!("Failed to execute 'addColorStop' on 'CanvasGradient': The provided value ({offset}) is outside the range (0.0, 1.0).")));
    }
    let text = string(vm, &args[1])?;
    let Some(c) = draw::color(&text) else {
        return Err(dom_exception(vm, "SyntaxError", &format!("Failed to execute 'addColorStop' on 'CanvasGradient': The value provided ('{text}') could not be parsed as a color.")));
    };
    if let Some(b) = data_mut(vm, g) {
        // (In order of their offsets; a stop at the same offset as others after them.)
        let mut v = f32s(b);
        let stops = (v.len().saturating_sub(7)) / 5;
        let mut at = stops;
        for k in 0..stops {
            if v[7 + k * 5] > offset as f32 {
                at = k;
                break;
            }
        }
        let stop = [offset as f32, c[0], c[1], c[2], c[3]];
        let pos = 7 + at * 5;
        for (j, x) in stop.iter().enumerate() {
            v.insert(pos + j, *x);
        }
        b.clear();
        push_f32s(b, &v);
    }
    Ok(Value::Undefined)
}

/// A picture to draw (drawImage, createPattern): a canvas, or a picture the page has.
enum Src {
    Canvas(usize),
    Picture(Arc<Pixels>),
}

/// What an argument is as a picture: None if it's one that isn't ready (a picture that hasn't come), or that
/// draws nothing here (a video).
fn source_of(vm: &mut Vm, v: &Value, what: &str) -> Result<Option<Src>, Value> {
    if let Some(n) = node_of(vm, v) {
        let name = String::from(host_ref(vm).doc.name(n));
        match name.as_str() {
            "canvas" => {
                let h = host(vm);
                let i = canvas_index(h, n);
                if h.canvases[i].w == 0 || h.canvases[i].h == 0 {
                    return Err(dom_exception(vm, "InvalidStateError", &format!("Failed to execute '{what}': The image argument is a canvas element with a width or height of 0.")));
                }
                return Ok(Some(Src::Canvas(i)));
            }
            "img" | "image" => {
                let h = host_ref(vm);
                let r = h.layout_inputs.as_ref().and_then(|li| li.picture_of.iter().find(|(k, _)| *k == n).map(|(_, r)| *r));
                return Ok(r.and_then(|r| h.pixels.get(r).cloned().flatten()).map(Src::Picture));
            }
            "video" => return Ok(None),
            _ => {}
        }
    }
    if let Value::Obj(g) = v
        && matches!(vm.heap.get(*g).class, Class::Host(CONTEXT, _))
    {
        // (An OffscreenCanvasRenderingContext's place: not one.)
        return Err(vm.type_error(&format!("Failed to execute '{what}': The provided value is not of type '(CSSImageValue or HTMLCanvasElement or HTMLImageElement or HTMLVideoElement or ImageBitmap or OffscreenCanvas or SVGImageElement or VideoFrame)'.")));
    }
    Err(vm.type_error(&format!("Failed to execute '{what}': The provided value is not of type '(CSSImageValue or HTMLCanvasElement or HTMLImageElement or HTMLVideoElement or ImageBitmap or OffscreenCanvas or SVGImageElement or VideoFrame)'.")))
}

fn create_pattern(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    ctx_node(vm, this)?;
    let src = source_of(vm, &arg(args, 0), "createPattern")?;
    let rep = match arg(args, 1) {
        Value::Null | Value::Undefined => String::new(),
        v => string(vm, &v)?,
    };
    let rep = match rep.as_str() {
        "" | "repeat" => 3u32,
        "repeat-x" => 1,
        "repeat-y" => 2,
        "no-repeat" => 0,
        _ => return Err(dom_exception(vm, "SyntaxError", &format!("Failed to execute 'createPattern' on 'CanvasRenderingContext2D': The provided type ('{rep}') is not one of 'repeat', 'no-repeat', 'repeat-x', or 'repeat-y'."))),
    };
    let Some(src) = src else { return Ok(Value::Null) };
    // A copy of its pixels as they are now (RGBA), its size as decoded and its own.
    let (w, h, nat, rgba) = match src {
        Src::Canvas(i) => {
            let c = &host_ref(vm).canvases[i];
            (c.w, c.h, (c.w as f32, c.h as f32), c.rgba())
        }
        Src::Picture(p) => {
            let mut out = vec![0u8; p.width as usize * p.height as usize * 4];
            for i in 0..(p.width as usize * p.height as usize) {
                out[i * 4..i * 4 + 3].copy_from_slice(&p.rgb[i * 3..i * 3 + 3]);
                out[i * 4 + 3] = p.alpha.as_ref().map_or(255, |a| a[i]);
            }
            (p.width, p.height, (p.natural.0 as f32, p.natural.1 as f32), out)
        }
    };
    if w as u64 * h as u64 > MOST_PIXELS {
        return Ok(Value::Null);
    }
    let proto = host_ref(vm).proto("CanvasPattern").unwrap_or_else(|| vm.proto(js::vm::I::ObjectProto));
    let g = vm.alloc(Some(proto), Class::Host(PATTERN, 0));
    let mut bytes = Vec::with_capacity(44 + rgba.len());
    bytes.extend_from_slice(&w.to_le_bytes());
    bytes.extend_from_slice(&h.to_le_bytes());
    bytes.extend_from_slice(&rep.to_le_bytes());
    push_f32s(&mut bytes, &[nat.0, nat.1]);
    push_f32s(&mut bytes, &Matrix::ID.0);
    bytes.extend_from_slice(&rgba);
    set_data(vm, g, bytes);
    Ok(Value::Obj(g))
}

fn pattern_set_transform(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let g = match this {
        Value::Obj(g) if matches!(vm.heap.get(*g).class, Class::Host(PATTERN, _)) => *g,
        _ => return Err(vm.type_error("Illegal invocation")),
    };
    let Some(m) = matrix_init(vm, &arg(args, 0))? else { return Ok(Value::Undefined) };
    if let Some(b) = data_mut(vm, g)
        && b.len() >= 44
    {
        for (k, x) in m.0.iter().enumerate() {
            b[20 + k * 4..24 + k * 4].copy_from_slice(&x.to_le_bytes());
        }
    }
    Ok(Value::Undefined)
}

// ------------------------------------------------------------------------------------------------ painting

/// Pixels to sample from: a canvas's (taken from it while it's drawn from, or copied), or a picture's.
enum Planes {
    Own(Vec<u8>, Vec<u8>),
    Shared(Arc<Pixels>),
}

struct Img {
    w: usize,
    h: usize,
    planes: Planes,
}

impl Img {
    /// A pixel, premultiplied (0 to 1).
    fn at(&self, x: usize, y: usize) -> [f32; 4] {
        let i = y * self.w + x;
        let (rgb, a) = match &self.planes {
            Planes::Own(rgb, alpha) => (rgb.as_slice(), alpha.get(i).copied().unwrap_or(0)),
            Planes::Shared(p) => (p.rgb.as_slice(), p.alpha.as_ref().map_or(255, |a| a.get(i).copied().unwrap_or(0))),
        };
        if a == 0 || rgb.len() < i * 3 + 3 {
            return [0.0; 4];
        }
        let a = a as f32 / 255.0;
        [rgb[i * 3] as f32 / 255.0 * a, rgb[i * 3 + 1] as f32 / 255.0 * a, rgb[i * 3 + 2] as f32 / 255.0 * a, a]
    }

    /// Bilinear, between the four pixels nearest (x, y) (the pixels' centres at their halves), kept within
    /// `area` (x0, y0, x1, y1 in pixels); premultiplied.
    fn smooth(&self, x: f32, y: f32, area: (usize, usize, usize, usize)) -> [f32; 4] {
        let (ax0, ay0, ax1, ay1) = area;
        let fx = (x - 0.5).clamp(ax0 as f32, (ax1 - 1) as f32);
        let fy = (y - 0.5).clamp(ay0 as f32, (ay1 - 1) as f32);
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(ax1 - 1), (y0 + 1).min(ay1 - 1));
        let (dx, dy) = (fx - x0 as f32, fy - y0 as f32);
        let mut out = [0.0f32; 4];
        for (px, py, w) in [(x0, y0, (1.0 - dx) * (1.0 - dy)), (x1, y0, dx * (1.0 - dy)), (x0, y1, (1.0 - dx) * dy), (x1, y1, dx * dy)] {
            if w > 0.0 {
                let p = self.at(px, py);
                for k in 0..4 {
                    out[k] += p[k] * w;
                }
            }
        }
        out
    }
}

/// What a shape is filled with, ready to sample at each pixel.
enum Fill {
    Solid(Rgba),
    /// 0 linear, 1 radial, 2 conic: its numbers, its stops, and the canvas's pixels to its space.
    Gradient { kind: u8, p: [f32; 6], stops: Vec<(f32, Rgba)>, inv: Matrix },
    /// The pattern's pixels, which ways it repeats, its tile's size, and the canvas's pixels to its space.
    Pattern { img: Img, rep: u32, tile: (f32, f32), inv: Matrix },
    /// drawImage's: the canvas's pixels to the picture's, the part of it drawn, smoothed or not, and how many
    /// samples a side each pixel takes (a picture made much smaller).
    Image { img: Img, inv: Matrix, area: (usize, usize, usize, usize), smooth: bool, samples: usize },
    Nothing,
}

impl Fill {
    /// The colour at a pixel (straight: RGB and alpha, 0 to 1).
    fn at(&self, x: usize, y: usize) -> Rgba {
        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
        match self {
            Fill::Solid(c) => *c,
            Fill::Gradient { kind, p, stops, inv } => {
                let (gx, gy) = inv.apply(px, py);
                match gradient_t(*kind, p, gx, gy) {
                    Some(t) => stop_color(stops, t),
                    None => [0.0; 4],
                }
            }
            Fill::Pattern { img, rep, tile, inv } => {
                let (u, v) = inv.apply(px, py);
                let wrap = |t: f32, size: f32, repeat: bool| -> Option<f32> {
                    if repeat {
                        Some(draw::rem_euclid(t, size))
                    } else if (0.0..size).contains(&t) {
                        Some(t)
                    } else {
                        None
                    }
                };
                let (Some(u), Some(v)) = (wrap(u, tile.0, rep & 1 != 0), wrap(v, tile.1, rep & 2 != 0)) else { return [0.0; 4] };
                let ix = ((u * img.w as f32 / tile.0) as usize).min(img.w - 1);
                let iy = ((v * img.h as f32 / tile.1) as usize).min(img.h - 1);
                unpremultiply(img.at(ix, iy))
            }
            Fill::Image { img, inv, area, smooth, samples } => {
                let (ax0, ay0, ax1, ay1) = *area;
                if *samples > 1 {
                    // (A picture drawn much smaller: each pixel the average of a few spread over it.)
                    let n = *samples;
                    let mut sum = [0.0f32; 4];
                    for j in 0..n {
                        for i in 0..n {
                            let (sx, sy) = inv.apply(x as f32 + (i as f32 + 0.5) / n as f32, y as f32 + (j as f32 + 0.5) / n as f32);
                            let ix = (sx.max(0.0) as usize).clamp(ax0, ax1 - 1);
                            let iy = (sy.max(0.0) as usize).clamp(ay0, ay1 - 1);
                            let p = img.at(ix, iy);
                            for k in 0..4 {
                                sum[k] += p[k];
                            }
                        }
                    }
                    let d = (n * n) as f32;
                    return unpremultiply(sum.map(|v| v / d));
                }
                let (sx, sy) = inv.apply(px, py);
                if *smooth {
                    unpremultiply(img.smooth(sx, sy, *area))
                } else {
                    let ix = (sx.max(0.0) as usize).clamp(ax0, ax1 - 1);
                    let iy = (sy.max(0.0) as usize).clamp(ay0, ay1 - 1);
                    unpremultiply(img.at(ix, iy))
                }
            }
            Fill::Nothing => [0.0; 4],
        }
    }
}

fn unpremultiply(p: [f32; 4]) -> Rgba {
    if p[3] <= 0.0 { [0.0; 4] } else { [p[0] / p[3], p[1] / p[3], p[2] / p[3], p[3]] }
}

/// Where a point is along a gradient (None: the gradient paints nothing there).
fn gradient_t(kind: u8, p: &[f32; 6], x: f32, y: f32) -> Option<f32> {
    match kind {
        0 => {
            let (dx, dy) = (p[2] - p[0], p[3] - p[1]);
            let l2 = dx * dx + dy * dy;
            if l2 <= 0.0 {
                return None;
            }
            Some(((x - p[0]) * dx + (y - p[1]) * dy) / l2)
        }
        1 => {
            // The largest ω whose circle (its centre and radius between the two circles', and on) has the point
            // on it, with a radius not below 0.
            let (x0, y0, r0, x1, y1, r1) = (p[0], p[1], p[2], p[3], p[4], p[5]);
            let (cdx, cdy, dr) = (x1 - x0, y1 - y0, r1 - r0);
            let (pdx, pdy) = (x - x0, y - y0);
            let a = cdx * cdx + cdy * cdy - dr * dr;
            let b = pdx * cdx + pdy * cdy + r0 * dr;
            let c = pdx * pdx + pdy * pdy - r0 * r0;
            let ok = |w: f32| r0 + w * dr >= 0.0;
            if a.abs() < 1e-6 {
                if b.abs() < 1e-9 {
                    return None;
                }
                let w = c / (2.0 * b);
                return if ok(w) { Some(w) } else { None };
            }
            let disc = b * b - a * c;
            if disc < 0.0 {
                return None;
            }
            let s = draw::libm_sqrt(disc);
            let (w1, w2) = ((b + s) / a, (b - s) / a);
            let (hi, lo) = if w1 > w2 { (w1, w2) } else { (w2, w1) };
            if ok(hi) {
                Some(hi)
            } else if ok(lo) {
                Some(lo)
            } else {
                None
            }
        }
        _ => {
            let a = draw::atan2(y - p[2], x - p[1]) - p[0];
            Some(draw::rem_euclid(a / core::f32::consts::TAU, 1.0))
        }
    }
}

/// A gradient's colour at `t` (padded past its ends; between stops, their colours in between, not premultiplied).
fn stop_color(stops: &[(f32, Rgba)], t: f32) -> Rgba {
    let Some(first) = stops.first() else { return [0.0; 4] };
    if t <= first.0 {
        return first.1;
    }
    for w in stops.windows(2) {
        if t < w[1].0 {
            let span = w[1].0 - w[0].0;
            let f = if span > 0.0 { (t - w[0].0) / span } else { 1.0 };
            let (a, b) = (w[0].1, w[1].1);
            return [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f, a[3] + (b[3] - a[3]) * f];
        }
    }
    stops[stops.len() - 1].1
}

/// A paint made ready to sample, for drawing through transform `m`.
fn resolve(vm: &Vm, paint: &Paint, m: Matrix) -> Fill {
    match paint {
        Paint::Color(c) => Fill::Solid(*c),
        Paint::Object(g) => {
            let Some(bytes) = data_of(vm, *g) else { return Fill::Nothing };
            match vm.heap.get(*g).class {
                Class::Host(GRADIENT, _) => {
                    let v = f32s(bytes);
                    if v.len() < 7 {
                        return Fill::Nothing;
                    }
                    let stops: Vec<(f32, Rgba)> = v[7..].chunks_exact(5).map(|s| (s[0], [s[1], s[2], s[3], s[4]])).collect();
                    let Some(inv) = m.invert() else { return Fill::Nothing };
                    if stops.is_empty() {
                        return Fill::Nothing;
                    }
                    let p = [v[1], v[2], v[3], v[4], v[5], v[6]];
                    // (A degenerate gradient paints nothing.)
                    let kind = v[0] as u8;
                    if (kind == 0 && p[0] == p[2] && p[1] == p[3]) || (kind == 1 && p[0] == p[3] && p[1] == p[4] && p[2] == p[5]) {
                        return Fill::Nothing;
                    }
                    Fill::Gradient { kind, p, stops, inv }
                }
                Class::Host(PATTERN, _) => {
                    if bytes.len() < 44 {
                        return Fill::Nothing;
                    }
                    let u = |k: usize| u32::from_le_bytes([bytes[k], bytes[k + 1], bytes[k + 2], bytes[k + 3]]);
                    let f = f32s(&bytes[12..44]);
                    let (w, h, rep) = (u(0) as usize, u(4) as usize, u(8));
                    let pm = Matrix([f[2], f[3], f[4], f[5], f[6], f[7]]);
                    let Some(inv) = m.then(&pm).invert() else { return Fill::Nothing };
                    if w == 0 || h == 0 || bytes.len() < 44 + w * h * 4 {
                        return Fill::Nothing;
                    }
                    let px = &bytes[44..44 + w * h * 4];
                    let mut rgb = Vec::with_capacity(w * h * 3);
                    let mut alpha = Vec::with_capacity(w * h);
                    for p in px.chunks_exact(4) {
                        rgb.extend_from_slice(&p[..3]);
                        alpha.push(p[3]);
                    }
                    Fill::Pattern { img: Img { w, h, planes: Planes::Own(rgb, alpha) }, rep, tile: (f[0].max(1.0), f[1].max(1.0)), inv }
                }
                _ => Fill::Nothing,
            }
        }
    }
}

/// A blend mode's colour, from what's there (`b`) and what's drawn (`s`) (straight colours).
fn blend(mode: u8, b: [f32; 3], s: [f32; 3]) -> [f32; 3] {
    let sep = |f: fn(f32, f32) -> f32| [f(b[0], s[0]), f(b[1], s[1]), f(b[2], s[2])];
    fn screen(b: f32, s: f32) -> f32 {
        b + s - b * s
    }
    fn hard_light(b: f32, s: f32) -> f32 {
        if s <= 0.5 { b * 2.0 * s } else { screen(b, 2.0 * s - 1.0) }
    }
    let lum = |c: [f32; 3]| 0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2];
    let clip_color = |c: [f32; 3]| {
        let l = lum(c);
        let n = c[0].min(c[1]).min(c[2]);
        let x = c[0].max(c[1]).max(c[2]);
        let mut c = c;
        if n < 0.0 && l - n > 1e-6 {
            c = c.map(|v| l + (v - l) * l / (l - n));
        }
        if x > 1.0 && x - l > 1e-6 {
            c = c.map(|v| l + (v - l) * (1.0 - l) / (x - l));
        }
        c
    };
    let set_lum = |c: [f32; 3], l: f32| {
        let d = l - lum(c);
        clip_color(c.map(|v| v + d))
    };
    let sat = |c: [f32; 3]| c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2]);
    let set_sat = |c: [f32; 3], s: f32| {
        let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
        if mx - mn <= 1e-6 {
            return [0.0; 3];
        }
        c.map(|v| (v - mn) * s / (mx - mn))
    };
    match mode {
        0 => sep(|b, s| b * s),
        1 => sep(screen),
        2 => sep(|b, s| hard_light(s, b)),
        3 => sep(f32::min),
        4 => sep(f32::max),
        5 => sep(|b, s| {
            if b <= 0.0 {
                0.0
            } else if s >= 1.0 {
                1.0
            } else {
                (b / (1.0 - s)).min(1.0)
            }
        }),
        6 => sep(|b, s| {
            if b >= 1.0 {
                1.0
            } else if s <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - b) / s).min(1.0)
            }
        }),
        7 => sep(hard_light),
        8 => sep(|b, s| {
            if s <= 0.5 {
                b - (1.0 - 2.0 * s) * b * (1.0 - b)
            } else {
                let d = if b <= 0.25 { ((16.0 * b - 12.0) * b + 4.0) * b } else { draw::libm_sqrt(b) };
                b + (2.0 * s - 1.0) * (d - b)
            }
        }),
        9 => sep(|b, s| (b - s).abs()),
        10 => sep(|b, s| b + s - 2.0 * b * s),
        11 => set_lum(set_sat(s, sat(b)), lum(b)),
        12 => set_lum(set_sat(b, sat(s)), lum(b)),
        13 => set_lum(s, lum(b)),
        _ => set_lum(b, lum(s)),
    }
}

/// One pixel drawn on: `s` (straight) at alpha `sa` (its own, the shape's coverage and the global alpha), by `op`,
/// `k` of it let through by the clip.
#[inline]
fn put(rgb: &mut [u8], a: &mut u8, s: Rgba, sa: f32, op: Op, k: f32, opaque: bool) {
    let da = *a as f32 / 255.0;
    if op == Op::SourceOver && k >= 1.0 {
        if sa <= 0.0 {
            return;
        }
        if sa >= 0.999 || (da <= 0.0 && !opaque) {
            for j in 0..3 {
                rgb[j] = (s[j].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
            *a = if opaque { 255 } else { (sa.min(1.0) * 255.0 + 0.5) as u8 };
            if opaque && sa < 0.999 {
                for v in rgb.iter_mut().take(3) {
                    *v = (*v as f32 * sa + 0.5) as u8;
                }
            }
            return;
        }
    }
    let d = [rgb[0] as f32 / 255.0, rgb[1] as f32 / 255.0, rgb[2] as f32 / 255.0];
    let mut oc = [0.0f32; 3];
    let mut oa;
    match op {
        Op::Blend(mode) => {
            let b = blend(mode, d, [s[0], s[1], s[2]]);
            for j in 0..3 {
                oc[j] = s[j] * sa * (1.0 - da) + d[j] * da * (1.0 - sa) + sa * da * b[j];
            }
            oa = sa + da - sa * da;
        }
        Op::Lighter => {
            for j in 0..3 {
                oc[j] = (s[j] * sa + d[j] * da).min(1.0);
            }
            oa = (sa + da).min(1.0);
        }
        _ => {
            let (fa, fb) = op.factors(sa, da);
            for j in 0..3 {
                oc[j] = s[j] * sa * fa + d[j] * da * fb;
            }
            oa = sa * fa + da * fb;
        }
    }
    if k < 1.0 {
        for j in 0..3 {
            oc[j] = d[j] * da + (oc[j] - d[j] * da) * k;
        }
        oa = da + (oa - da) * k;
    }
    if opaque {
        // (On black: the colour as it shows.)
        for j in 0..3 {
            rgb[j] = (oc[j].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
        *a = 255;
        return;
    }
    if oa <= 1.0 / 510.0 {
        rgb.copy_from_slice(&[0, 0, 0]);
        *a = 0;
        return;
    }
    for j in 0..3 {
        rgb[j] = ((oc[j] / oa).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    }
    *a = (oa.min(1.0) * 255.0 + 0.5) as u8;
}

/// A coverage's value at an accumulated sum, by the fill rule.
#[inline]
fn covered(acc: f32, even_odd: bool) -> f32 {
    if even_odd {
        let f = acc.abs() % 2.0;
        if f > 1.0 { 2.0 - f } else { f }
    } else {
        acc.abs().min(1.0)
    }
}

/// Draws a fill onto the canvas where `cov` covers it (all of the canvas, for an operation that changes what's
/// outside the shape too), by the state's operation, alpha and clip.
fn composite(c: &mut Canvas, cov: Option<&Coverage>, even_odd: bool, fill: &Fill, st: &State) {
    if !c.ensure() {
        return;
    }
    let (w, h) = (c.w as usize, c.h as usize);
    let op = st.op;
    let (x0, y0, x1, y1) = match (op.bounded(), cov) {
        (true, Some(cv)) => (cv.x0.max(0) as usize, cv.y0.max(0) as usize, ((cv.x0 + cv.w as i32).max(0) as usize).min(w), ((cv.y0 + cv.h as i32).max(0) as usize).min(h)),
        (true, None) => return,
        (false, _) => (0, 0, w, h),
    };
    let clip = st.clip.clone();
    let opaque = c.opaque;
    for y in y0..y1 {
        let row = cov.and_then(|cv| {
            let ry = y as i32 - cv.y0;
            if ry >= 0 && (ry as usize) < cv.h { Some((cv, ry as usize * (cv.w + 2))) } else { None }
        });
        let mut acc = 0.0f32;
        for x in x0..x1 {
            let mut k = 0.0f32;
            if let Some((cv, start)) = row {
                let rx = x as i32 - cv.x0;
                if rx >= 0 && (rx as usize) < cv.w {
                    acc += cv.a[start + rx as usize];
                    k = covered(acc, even_odd);
                }
            }
            if k < 1.0 / 512.0 {
                k = 0.0;
                if op.bounded() {
                    continue;
                }
            }
            let i = y * w + x;
            let through = clip.as_ref().map_or(1.0, |m| m[i] as f32 / 255.0);
            if through <= 0.0 {
                continue;
            }
            let s = if k > 0.0 { fill.at(x, y) } else { [0.0; 4] };
            let sa = s[3] * k * st.alpha;
            let (rgb, a) = (&mut c.rgb[i * 3..i * 3 + 3], &mut c.alpha[i]);
            put(rgb, a, s, sa, op, through, opaque);
        }
    }
}

/// Where an axis-aligned rectangle is on the canvas (x0, y0, x1, y1 in its pixels), if the transform keeps
/// rectangles rectangles (as a page's drawing nearly always does), and nothing makes the shape's coverage needed
/// apart from it (a shadow).
fn plain_rect(st: &State, x: f32, y: f32, w: f32, h: f32) -> Option<(f32, f32, f32, f32)> {
    let m = st.m.0;
    if m[1] != 0.0 || m[2] != 0.0 || !st.op.bounded() || (st.shadow_color[3] > 0.0 && (st.shadow_blur > 0.0 || st.shadow_offset != (0.0, 0.0))) {
        return None;
    }
    let (a, b) = (st.m.apply(x, y), st.m.apply(x + w, y + h));
    let r = (a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1));
    if r.0.is_finite() && r.1.is_finite() && r.2.is_finite() && r.3.is_finite() { Some(r) } else { None }
}

/// An axis-aligned rectangle filled (or, `clear`, cleared): each pixel's coverage the overlap of its row and its
/// column with it; inside it, a solid colour written as it is.
fn rect_fast(c: &mut Canvas, r: (f32, f32, f32, f32), fill: &Fill, st: &State, clear: bool) {
    // (Nothing drawn on it yet: nothing to clear.)
    if (clear && !c.ok()) || !c.ensure() {
        return;
    }
    let (w, h) = (c.w as usize, c.h as usize);
    let (x0, y0, x1, y1) = (r.0.max(0.0), r.1.max(0.0), r.2.min(w as f32), r.3.min(h as f32));
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let (ix0, iy0) = (draw::floor(x0) as usize, draw::floor(y0) as usize);
    let (ix1, iy1) = ((draw::ceil(x1) as usize).min(w), (draw::ceil(y1) as usize).min(h));
    let opaque = c.opaque;
    // A colour that covers what's there wholly: written as bytes.
    let solid = match fill {
        Fill::Solid(col) if !clear && st.op == Op::SourceOver && st.clip.is_none() && col[3] * st.alpha >= 0.999 => {
            Some([col[0], col[1], col[2]].map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8))
        }
        _ => None,
    };
    let clip = st.clip.clone();
    let cover = |lo: f32, hi: f32, p: usize| ((p as f32 + 1.0).min(hi) - (p as f32).max(lo)).clamp(0.0, 1.0);
    // The columns wholly inside it.
    let (cx0, cx1) = ((draw::ceil(x0) as usize).min(ix1), (draw::floor(x1) as usize).max(ix0));
    for y in iy0..iy1 {
        let vy = cover(y0, y1, y);
        if vy >= 0.999 && cx1 > cx0 && (solid.is_some() || (clear && clip.is_none())) {
            // (A row's inside written at once.)
            let row = y * w;
            let (rgb, a) = match solid {
                Some(rgb) => (rgb, 255),
                None => ([0, 0, 0], if opaque { 255 } else { 0 }),
            };
            for px in c.rgb[(row + cx0) * 3..(row + cx1) * 3].chunks_exact_mut(3) {
                px.copy_from_slice(&rgb);
            }
            c.alpha[row + cx0..row + cx1].fill(a);
        }
        for x in ix0..ix1 {
            if vy >= 0.999 && x >= cx0 && x < cx1 && (solid.is_some() || (clear && clip.is_none())) {
                continue;
            }
            let k = vy * cover(x0, x1, x);
            if k <= 0.0 {
                continue;
            }
            let i = y * w + x;
            if k >= 0.999 {
                if let Some(rgb) = solid {
                    c.rgb[i * 3..i * 3 + 3].copy_from_slice(&rgb);
                    c.alpha[i] = 255;
                    continue;
                }
                if clear && clip.is_none() {
                    c.rgb[i * 3..i * 3 + 3].copy_from_slice(&[0, 0, 0]);
                    c.alpha[i] = if opaque { 255 } else { 0 };
                    continue;
                }
            }
            let through = clip.as_ref().map_or(1.0, |m| m[i] as f32 / 255.0);
            if through <= 0.0 {
                continue;
            }
            let (rgb, a) = (&mut c.rgb[i * 3..i * 3 + 3], &mut c.alpha[i]);
            if clear {
                if opaque {
                    put(rgb, a, [0.0, 0.0, 0.0, 1.0], k, Op::SourceOver, through, true);
                } else {
                    put(rgb, a, [0.0; 4], k, Op::DestOut, through, false);
                }
            } else {
                let s = fill.at(x, y);
                put(rgb, a, s, s[3] * k * st.alpha, st.op, through, opaque);
            }
        }
    }
}

/// A path's polygons to fill (each subpath closed).
fn fill_polys(o: &Outline) -> Vec<Vec<(f32, f32)>> {
    o.paths.iter().filter(|(p, _)| p.len() >= 3).map(|(p, _)| p.clone()).collect()
}

/// A path's stroke, as polygons to fill (non-zero), made in the space the transform maps from (so it's as wide as
/// the line width there, however the transform stretches it) and mapped to the canvas.
fn stroke_polys(o: &Outline, st: &State) -> Vec<Vec<(f32, f32)>> {
    let Some(inv) = st.m.invert() else { return Vec::new() };
    let user = Outline { paths: o.paths.iter().map(|(p, closed)| (p.iter().map(|q| inv.apply(q.0, q.1)).collect(), *closed)).collect() };
    let polys = draw::stroke(&user, st.line_width, st.cap, st.join, st.miter, &st.dash, st.dash_offset);
    polys.into_iter().map(|p| p.into_iter().map(|q| st.m.apply(q.0, q.1)).collect()).collect()
}

/// How a shape is drawn.
#[derive(Clone, Copy)]
enum How {
    /// Filled (even-odd if true).
    Fill(bool),
    Stroke,
}

/// Draws a shape (its outline in the canvas's pixels) on a canvas: its shadow first, if it has one, then it.
fn draw_shape(vm: &mut Vm, i: usize, outline: &Outline, how: How) {
    let (st, w, h) = {
        let c = canvas_mut(vm, i);
        if !c.ensure() {
            return;
        }
        (c.state().clone(), c.w, c.h)
    };
    let (polys, even_odd) = match how {
        How::Fill(eo) => (fill_polys(outline), eo),
        How::Stroke => (stroke_polys(outline, &st), false),
    };
    let paint = match how {
        How::Fill(_) => st.fill.clone(),
        How::Stroke => st.stroke.clone(),
    };
    let fill = resolve(vm, &paint, st.m);
    let cov = draw::coverage_of(&polys, w as f32, h as f32);
    let c = canvas_mut(vm, i);
    if st.shadow_color[3] > 0.0 && (st.shadow_blur > 0.0 || st.shadow_offset != (0.0, 0.0))
        && let Some(cv) = cov.as_ref()
    {
        shadow(c, cv, even_odd, &fill, &st);
    }
    composite(c, cov.as_ref(), even_odd, &fill, &st);
    c.version += 1;
    host(vm).canvas_drawn = true;
}

/// A shape's shadow: its coverage (times its paint's alpha) moved by the shadow's offset, blurred (three box blurs,
/// near enough a Gaussian of half the blur), in the shadow's colour.
fn shadow(c: &mut Canvas, cov: &Coverage, even_odd: bool, fill: &Fill, st: &State) {
    let (w, h) = (c.w as usize, c.h as usize);
    let sigma = st.shadow_blur / 2.0;
    let r = draw::ceil(sigma * 1.5) as usize;
    let (ox, oy) = (draw::floor(st.shadow_offset.0 + 0.5) as i32, draw::floor(st.shadow_offset.1 + 0.5) as i32);
    // The shadow's area: the shape's, moved and grown by the blur.
    let bx0 = (cov.x0 + ox - r as i32 * 3).max(0) as usize;
    let by0 = (cov.y0 + oy - r as i32 * 3).max(0) as usize;
    let bx1 = ((cov.x0 + cov.w as i32 + ox + r as i32 * 3).max(0) as usize).min(w);
    let by1 = ((cov.y0 + cov.h as i32 + oy + r as i32 * 3).max(0) as usize).min(h);
    if bx1 <= bx0 || by1 <= by0 || (bx1 - bx0) as u64 * (by1 - by0) as u64 > MOST_PIXELS {
        return;
    }
    let (sw, sh) = (bx1 - bx0, by1 - by0);
    let mut m = vec![0.0f32; sw * sh];
    let stride = cov.w + 2;
    for y in 0..cov.h {
        let mut acc = 0.0;
        for x in 0..cov.w {
            acc += cov.a[y * stride + x];
            let k = covered(acc, even_odd);
            if k <= 0.0 {
                continue;
            }
            let (px, py) = (cov.x0 + x as i32, cov.y0 + y as i32);
            let a = fill.at(px.max(0) as usize, py.max(0) as usize)[3] * k;
            let (tx, ty) = (px + ox - bx0 as i32, py + oy - by0 as i32);
            if tx >= 0 && ty >= 0 && (tx as usize) < sw && (ty as usize) < sh {
                m[ty as usize * sw + tx as usize] = a;
            }
        }
    }
    if r > 0 {
        let mut tmp = vec![0.0f32; sw.max(sh)];
        for _ in 0..3 {
            for y in 0..sh {
                box_blur(&mut m[y * sw..(y + 1) * sw], 1, sw, r, &mut tmp);
            }
            for x in 0..sw {
                let mut col: Vec<f32> = (0..sh).map(|y| m[y * sw + x]).collect();
                box_blur(&mut col, 1, sh, r, &mut tmp);
                for (y, v) in col.into_iter().enumerate() {
                    m[y * sw + x] = v;
                }
            }
        }
    }
    let color = st.shadow_color;
    let clip = st.clip.clone();
    for y in 0..sh {
        for x in 0..sw {
            let k = m[y * sw + x];
            if k <= 1.0 / 512.0 {
                continue;
            }
            let i = (y + by0) * w + x + bx0;
            let through = clip.as_ref().map_or(1.0, |cm| cm[i] as f32 / 255.0);
            if through <= 0.0 {
                continue;
            }
            let (rgb, a) = (&mut c.rgb[i * 3..i * 3 + 3], &mut c.alpha[i]);
            let op = if st.op.bounded() { st.op } else { Op::SourceOver };
            put(rgb, a, color, color[3] * k.min(1.0) * st.alpha, op, through, c.opaque);
        }
    }
}

/// A box blur of radius `r` along a line of values `n` long.
fn box_blur(v: &mut [f32], _step: usize, n: usize, r: usize, tmp: &mut [f32]) {
    let d = (2 * r + 1) as f32;
    let mut sum = 0.0;
    for k in 0..=r.min(n.saturating_sub(1)) {
        sum += v[k];
    }
    for i in 0..n {
        tmp[i] = sum / d;
        if i + r + 1 < n {
            sum += v[i + r + 1];
        }
        if i >= r {
            sum -= v[i - r];
        }
    }
    v[..n].copy_from_slice(&tmp[..n]);
}

/// The context's current path, in the canvas's pixels.
fn current_outline(c: &Canvas) -> Outline {
    let mut b = Builder::new(Matrix::ID);
    replay(&c.path, &mut b, Matrix::ID);
    b.finish()
}

/// A Path2D's outline through the context's transform.
fn path2d_outline(vm: &Vm, g: Gc, m: Matrix) -> Outline {
    let ops = path2d_ops(vm, g);
    let mut b = Builder::new(m);
    replay(&ops, &mut b, m);
    b.finish()
}

/// fill(), stroke(), clip() and the like's arguments: a Path2D (if given first), and a fill rule ("nonzero" or
/// "evenodd").
fn path_and_rule(vm: &mut Vm, args: &[Value], what: &str) -> Result<(Option<Gc>, bool, usize), Value> {
    let (path, k) = match args.first() {
        Some(Value::Obj(g)) if matches!(vm.heap.get(*g).class, Class::Host(PATH2D, _)) => (Some(*g), 1),
        _ => (None, 0),
    };
    let rule = match args.get(k) {
        None | Some(Value::Undefined) => false,
        Some(v) => match string(vm, v)?.as_str() {
            "nonzero" => false,
            "evenodd" => true,
            other => {
                return Err(vm.type_error(&format!("Failed to execute '{what}' on 'CanvasRenderingContext2D': The provided value '{other}' is not a valid enum value of type CanvasFillRule.")));
            }
        },
    };
    Ok((path, rule, k))
}

fn outline_for(vm: &mut Vm, i: usize, path: Option<Gc>) -> Outline {
    match path {
        Some(g) => {
            let m = canvas_mut(vm, i).state().m;
            path2d_outline(vm, g, m)
        }
        None => current_outline(canvas_mut(vm, i)),
    }
}

fn fill(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (_, i) = ctx(vm, this)?;
    let (path, eo, _) = path_and_rule(vm, args, "fill")?;
    let o = outline_for(vm, i, path);
    draw_shape(vm, i, &o, How::Fill(eo));
    Ok(Value::Undefined)
}

fn stroke(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (_, i) = ctx(vm, this)?;
    let path = match args.first() {
        Some(Value::Obj(g)) if matches!(vm.heap.get(*g).class, Class::Host(PATH2D, _)) => Some(*g),
        _ => None,
    };
    let o = outline_for(vm, i, path);
    draw_shape(vm, i, &o, How::Stroke);
    Ok(Value::Undefined)
}

fn clip(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (_, i) = ctx(vm, this)?;
    let (path, eo, _) = path_and_rule(vm, args, "clip")?;
    let o = outline_for(vm, i, path);
    let c = canvas_mut(vm, i);
    if !c.fits() {
        return Ok(Value::Undefined);
    }
    let (w, h) = (c.w as usize, c.h as usize);
    let mut mask = vec![0u8; w * h];
    if let Some(cv) = draw::coverage_of(&fill_polys(&o), w as f32, h as f32) {
        let stride = cv.w + 2;
        for y in 0..cv.h {
            let mut acc = 0.0;
            for x in 0..cv.w {
                acc += cv.a[y * stride + x];
                let (px, py) = (cv.x0 + x as i32, cv.y0 + y as i32);
                if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                    mask[py as usize * w + px as usize] = (covered(acc, eo) * 255.0 + 0.5) as u8;
                }
            }
        }
    }
    let st = c.state_mut();
    if let Some(old) = &st.clip {
        for (m, o) in mask.iter_mut().zip(old.iter()) {
            *m = ((*m as u32 * *o as u32 + 127) / 255) as u8;
        }
    }
    st.clip = Some(Rc::new(mask));
    Ok(Value::Undefined)
}

/// Whether a point is inside polygons, by a fill rule.
fn inside(polys: &[Vec<(f32, f32)>], x: f32, y: f32, even_odd: bool) -> bool {
    let mut winding = 0i32;
    for p in polys {
        for k in 0..p.len() {
            let (a, b) = (p[k], p[(k + 1) % p.len()]);
            if (a.1 <= y) != (b.1 <= y) {
                let t = (y - a.1) / (b.1 - a.1);
                if a.0 + t * (b.0 - a.0) > x {
                    winding += if b.1 > a.1 { 1 } else { -1 };
                }
            }
        }
    }
    if even_odd { winding % 2 != 0 } else { winding != 0 }
}

fn is_point_in_path(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (_, i) = ctx(vm, this)?;
    let (path, k) = match args.first() {
        Some(Value::Obj(g)) if matches!(vm.heap.get(*g).class, Class::Host(PATH2D, _)) => (Some(*g), 1),
        _ => (None, 0),
    };
    let x = vm.to_number(&arg(args, k))?;
    let y = vm.to_number(&arg(args, k + 1))?;
    let (_, eo, _) = path_and_rule(vm, &args[(k + 2).min(args.len())..], "isPointInPath")?;
    if !x.is_finite() || !y.is_finite() {
        return Ok(Value::Bool(false));
    }
    let o = outline_for(vm, i, path);
    Ok(Value::Bool(inside(&fill_polys(&o), x as f32, y as f32, eo)))
}

fn is_point_in_stroke(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (_, i) = ctx(vm, this)?;
    let (path, k) = match args.first() {
        Some(Value::Obj(g)) if matches!(vm.heap.get(*g).class, Class::Host(PATH2D, _)) => (Some(*g), 1),
        _ => (None, 0),
    };
    let x = vm.to_number(&arg(args, k))?;
    let y = vm.to_number(&arg(args, k + 1))?;
    if !x.is_finite() || !y.is_finite() {
        return Ok(Value::Bool(false));
    }
    let o = outline_for(vm, i, path);
    let st = canvas_mut(vm, i).state().clone();
    Ok(Value::Bool(inside(&stroke_polys(&o, &st), x as f32, y as f32, false)))
}

/// A rectangle's outline through the context's transform.
fn rect_outline(m: Matrix, x: f32, y: f32, w: f32, h: f32) -> Outline {
    let mut b = Builder::new(m);
    b.move_to((x, y));
    b.line_to((x + w, y));
    b.line_to((x + w, y + h));
    b.line_to((x, y + h));
    b.close();
    b.finish()
}

fn fill_rect(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (_, i) = ctx(vm, this)?;
    let Some([x, y, w, h]) = numbers::<4>(vm, args, "fillRect")? else { return Ok(Value::Undefined) };
    if w == 0.0 || h == 0.0 {
        return Ok(Value::Undefined);
    }
    let st = canvas_mut(vm, i).state().clone();
    if let Some(r) = plain_rect(&st, x, y, w, h) {
        let fill = resolve(vm, &st.fill, st.m);
        let c = canvas_mut(vm, i);
        rect_fast(c, r, &fill, &st, false);
        c.version += 1;
        host(vm).canvas_drawn = true;
        return Ok(Value::Undefined);
    }
    draw_shape(vm, i, &rect_outline(st.m, x, y, w, h), How::Fill(false));
    Ok(Value::Undefined)
}

fn stroke_rect(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (_, i) = ctx(vm, this)?;
    let Some([x, y, w, h]) = numbers::<4>(vm, args, "strokeRect")? else { return Ok(Value::Undefined) };
    let m = canvas_mut(vm, i).state().m;
    let o = if w == 0.0 && h == 0.0 {
        return Ok(Value::Undefined);
    } else if w == 0.0 || h == 0.0 {
        // (A rectangle with no width or no height: a line.)
        let mut b = Builder::new(m);
        b.move_to((x, y));
        b.line_to((x + w, y + h));
        b.finish()
    } else {
        rect_outline(m, x, y, w, h)
    };
    draw_shape(vm, i, &o, How::Stroke);
    Ok(Value::Undefined)
}

fn clear_rect(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (_, i) = ctx(vm, this)?;
    let Some([x, y, w, h]) = numbers::<4>(vm, args, "clearRect")? else { return Ok(Value::Undefined) };
    let c = canvas_mut(vm, i);
    if !c.ok() || w == 0.0 || h == 0.0 {
        return Ok(Value::Undefined);
    }
    let st = c.state().clone();
    let (cw, ch) = (c.w as f32, c.h as f32);
    let identity = st.m.0[1] == 0.0 && st.m.0[2] == 0.0;
    let (p0, p1) = (st.m.apply(x, y), st.m.apply(x + w, y + h));
    if identity && st.clip.is_none() && p0.0.min(p1.0) <= 0.0 && p0.1.min(p1.1) <= 0.0 && p0.0.max(p1.0) >= cw && p0.1.max(p1.1) >= ch {
        // (The whole canvas, as pages clear it every frame.)
        let opaque = c.opaque;
        c.rgb.fill(0);
        c.alpha.fill(if opaque { 255 } else { 0 });
    } else if let Some(r) = plain_rect(&State { op: Op::SourceOver, shadow_color: [0.0; 4], ..st.clone() }, x, y, w, h) {
        rect_fast(c, r, &Fill::Nothing, &st, true);
    } else {
        let mut cleared = st.clone();
        cleared.op = Op::DestOut;
        cleared.alpha = 1.0;
        cleared.shadow_color = [0.0; 4];
        let o = rect_outline(st.m, x, y, w, h);
        let cov = draw::coverage_of(&fill_polys(&o), cw, ch);
        if c.opaque {
            // (An opaque canvas's cleared pixels are black.)
            cleared.op = Op::SourceOver;
            composite(c, cov.as_ref(), false, &Fill::Solid([0.0, 0.0, 0.0, 1.0]), &cleared);
        } else {
            composite(c, cov.as_ref(), false, &Fill::Solid([0.0, 0.0, 0.0, 1.0]), &cleared);
        }
    }
    c.version += 1;
    host(vm).canvas_drawn = true;
    Ok(Value::Undefined)
}

// --------------------------------------------------------------------------------------------------- pictures

fn draw_image(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (_, i) = ctx(vm, this)?;
    if args.len() < 3 {
        return Err(vm.type_error(&format!("Failed to execute 'drawImage' on 'CanvasRenderingContext2D': 3 arguments required, but only {} present.", args.len())));
    }
    let src = source_of(vm, &args[0], "drawImage")?;
    let rest = &args[1..];
    let mut v = [0f32; 8];
    let k = match rest.len() {
        2 => 2,
        3 | 4 => 4,
        _ => 8,
    };
    for (j, x) in v.iter_mut().enumerate().take(k) {
        let n = vm.to_number(&arg(rest, j))?;
        if !n.is_finite() {
            return Ok(Value::Undefined);
        }
        *x = n as f32;
    }
    let Some(src) = src else { return Ok(Value::Undefined) };
    // The picture's own size, and its pixels' (a picture decoded smaller than it is).
    let (iw, ih, pw, ph) = match &src {
        Src::Canvas(s) => {
            let c = &host_ref(vm).canvases[*s];
            (c.w as f32, c.h as f32, c.w as usize, c.h as usize)
        }
        Src::Picture(p) => (p.natural.0 as f32, p.natural.1 as f32, p.width as usize, p.height as usize),
    };
    let (mut sx, mut sy, mut sw, mut sh, mut dx, mut dy, mut dw, mut dh) = match k {
        2 => (0.0, 0.0, iw, ih, v[0], v[1], iw, ih),
        4 => (0.0, 0.0, iw, ih, v[0], v[1], v[2], v[3]),
        _ => (v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7]),
    };
    // (Negative sizes: the rectangles from their other corners.)
    if sw < 0.0 {
        sx += sw;
        sw = -sw;
    }
    if sh < 0.0 {
        sy += sh;
        sh = -sh;
    }
    if dw < 0.0 {
        dx += dw;
        dw = -dw;
    }
    if dh < 0.0 {
        dy += dh;
        dh = -dh;
    }
    if sw == 0.0 || sh == 0.0 || dw == 0.0 || dh == 0.0 || pw == 0 || ph == 0 {
        return Ok(Value::Undefined);
    }
    // The source rectangle kept to the picture, the destination's cut the same.
    let (kx, ky) = (dw / sw, dh / sh);
    if sx < 0.0 {
        dx -= sx * kx;
        sw += sx;
        sx = 0.0;
    }
    if sy < 0.0 {
        dy -= sy * ky;
        sh += sy;
        sy = 0.0;
    }
    if sx + sw > iw {
        sw = iw - sx;
    }
    if sy + sh > ih {
        sh = ih - sy;
    }
    if sw <= 0.0 || sh <= 0.0 {
        return Ok(Value::Undefined);
    }
    let (dw, dh) = (sw * kx, sh * ky);
    let st = canvas_mut(vm, i).state().clone();
    // The canvas's pixels to the picture's: through the inverse of where the picture's pixels go.
    let (ux, uy) = (iw / pw as f32, ih / ph as f32);
    let to_canvas = st.m.then(&Matrix::translate(dx, dy)).then(&Matrix::scale(kx, ky)).then(&Matrix::translate(-sx, -sy)).then(&Matrix::scale(ux, uy));
    let Some(inv) = to_canvas.invert() else { return Ok(Value::Undefined) };
    let area = ((sx / ux).max(0.0) as usize, (sy / uy).max(0.0) as usize, (draw::ceil((sx + sw) / ux) as usize).clamp(1, pw), (draw::ceil((sy + sh) / uy) as usize).clamp(1, ph));
    let area = (area.0.min(area.2 - 1), area.1.min(area.3 - 1), area.2, area.3);
    let shrink = inv.scale_of();
    let samples = if st.smoothing && shrink > 1.5 { (draw::ceil(shrink) as usize).min(4) } else { 1 };
    // The picture's pixels: a canvas's taken from it while it's drawn from (copied if it's this one).
    let img = match &src {
        Src::Canvas(s) if *s == i => {
            let c = &host_ref(vm).canvases[*s];
            Img { w: pw, h: ph, planes: Planes::Own(c.rgb.clone(), c.alpha.clone()) }
        }
        Src::Canvas(s) => {
            let c = &mut host(vm).canvases[*s];
            Img { w: pw, h: ph, planes: Planes::Own(core::mem::take(&mut c.rgb), core::mem::take(&mut c.alpha)) }
        }
        Src::Picture(p) => Img { w: pw, h: ph, planes: Planes::Shared(p.clone()) },
    };
    let fill = Fill::Image { img, inv, area, smooth: st.smoothing, samples };
    let o = rect_outline(st.m, dx, dy, dw, dh);
    let c = canvas_mut(vm, i);
    if c.ensure() {
        let cov = draw::coverage_of(&fill_polys(&o), c.w as f32, c.h as f32);
        if st.shadow_color[3] > 0.0 && (st.shadow_blur > 0.0 || st.shadow_offset != (0.0, 0.0))
            && let Some(cv) = cov.as_ref()
        {
            shadow(c, cv, false, &fill, &st);
        }
        composite(c, cov.as_ref(), false, &fill, &st);
        c.version += 1;
    }
    // (The other canvas's pixels back.)
    if let (Src::Canvas(s), Fill::Image { img: Img { planes: Planes::Own(rgb, alpha), .. }, .. }) = (src, fill)
        && s != i
    {
        let c = &mut host(vm).canvases[s];
        c.rgb = rgb;
        c.alpha = alpha;
    }
    host(vm).canvas_drawn = true;
    Ok(Value::Undefined)
}

// ------------------------------------------------------------------------------------------------------- text

/// A line of glyph outlines, kept in font units (to place once the line's width is known).
struct Glyphs(Vec<(u8, [i32; 4])>);

impl text::Outline for Glyphs {
    fn move_to(&mut self, x: i32, y: i32) {
        self.0.push((0, [x, y, 0, 0]));
    }
    fn line_to(&mut self, x: i32, y: i32) {
        self.0.push((1, [x, y, 0, 0]));
    }
    fn quad_to(&mut self, cx: i32, cy: i32, x: i32, y: i32) {
        self.0.push((2, [cx, cy, x, y]));
    }
    fn close(&mut self) {
        self.0.push((3, [0; 4]));
    }
}

/// Where text goes from where it's drawn at: across (by its alignment, `width` wide) and down (by its baseline),
/// in pixels; and the font's height above and below its baseline, and its em box's.
fn text_place(st: &State, width: f32) -> (f32, f32, (f32, f32), (f32, f32)) {
    let ty = text::Type::shared();
    let (asc, desc) = ty.vertical();
    let upem = ty.units_per_em() as f32;
    let size = st.font.size;
    let (a, d) = (asc as f32 * size / upem, -(desc as f32) * size / upem);
    let em_a = if a + d > 0.0 { size * a / (a + d) } else { size * 0.8 };
    let em_d = size - em_a;
    let rtl = st.direction == 2;
    let dx = match st.align {
        2 => 0.0,
        3 => -width,
        4 => -width / 2.0,
        0 if !rtl => 0.0,
        1 if rtl => 0.0,
        _ => -width,
    };
    let dy = match st.baseline {
        1 => em_a,
        2 => em_a * 0.8,
        3 => (em_a - em_d) / 2.0,
        4 => -d,
        5 => -em_d,
        _ => 0.0,
    };
    (dx, dy, (a, d), (em_a, em_d))
}

/// Text's characters as the canvas draws them (its spaces, tabs and line breaks all spaces).
fn text_arg(vm: &mut Vm, v: &Value) -> Result<String, Value> {
    Ok(string(vm, v)?.chars().map(|c| if matches!(c, '\t' | '\n' | '\x0c' | '\r') { ' ' } else { c }).collect())
}

fn draw_text(vm: &mut Vm, this: &Value, args: &[Value], how: How) -> R {
    let (_, i) = ctx(vm, this)?;
    if args.len() < 3 {
        return Err(vm.type_error(&format!("Failed to execute 'fillText' on 'CanvasRenderingContext2D': 3 arguments required, but only {} present.", args.len())));
    }
    let text = text_arg(vm, &args[0])?;
    let x = vm.to_number(&args[1])?;
    let y = vm.to_number(&args[2])?;
    let max = match args.get(3) {
        None | Some(Value::Undefined) => None,
        Some(v) => Some(vm.to_number(v)?),
    };
    if !x.is_finite() || !y.is_finite() || max.is_some_and(|m| !m.is_finite() || m <= 0.0) {
        return Ok(Value::Undefined);
    }
    let st = canvas_mut(vm, i).state().clone();
    let ty = text::Type::shared();
    let mut g = Glyphs(Vec::new());
    let units = ty.outline_line(&text, st.font.face, &mut g);
    let width = units as f32 * st.font.size / ty.units_per_em() as f32;
    // (The outlines are in 64ths of a font unit.)
    let k = st.font.size / (ty.units_per_em() as f32 * 64.0);
    let squeeze = match max {
        Some(m) if width > m as f32 && width > 0.0 => m as f32 / width,
        _ => 1.0,
    };
    let (dx, dy, _, _) = text_place(&st, width * squeeze);
    let (x0, y0) = (x as f32 + dx, y as f32 + dy);
    let mut b = Builder::new(st.m);
    for (op, v) in &g.0 {
        let p = |j: usize| (x0 + v[j] as f32 * k * squeeze, y0 - v[j + 1] as f32 * k);
        match op {
            0 => b.move_to(p(0)),
            1 => b.line_to(p(0)),
            2 => b.quad_to(p(0), p(2)),
            _ => b.close(),
        }
    }
    let o = b.finish();
    draw_shape(vm, i, &o, how);
    Ok(Value::Undefined)
}

fn fill_text(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    draw_text(vm, this, args, How::Fill(false))
}

fn stroke_text(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    draw_text(vm, this, args, How::Stroke)
}

fn measure_text(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (_, i) = ctx(vm, this)?;
    if args.is_empty() {
        return Err(vm.type_error("Failed to execute 'measureText' on 'CanvasRenderingContext2D': 1 argument required, but only 0 present."));
    }
    let text = text_arg(vm, &args[0])?;
    let st = canvas_mut(vm, i).state().clone();
    let ty = text::Type::shared();
    let mut g = Glyphs(Vec::new());
    let units = ty.outline_line(&text, st.font.face, &mut g);
    let width = units as f32 * st.font.size / ty.units_per_em() as f32;
    let k = st.font.size / (ty.units_per_em() as f32 * 64.0);
    let (dx, dy, (a, d), (em_a, em_d)) = text_place(&st, width);
    // The ink's box (font units, y up), from the outlines' points.
    let (mut x0, mut x1, mut y0, mut y1) = (i32::MAX, i32::MIN, i32::MAX, i32::MIN);
    for (op, v) in &g.0 {
        let pts: &[usize] = match op {
            0 | 1 => &[0],
            2 => &[0, 2],
            _ => &[],
        };
        for &j in pts {
            x0 = x0.min(v[j]);
            x1 = x1.max(v[j]);
            y0 = y0.min(v[j + 1]);
            y1 = y1.max(v[j + 1]);
        }
    }
    let (left, right, up, down) = if x0 > x1 { (0.0, 0.0, 0.0, 0.0) } else { (x0 as f32 * k, x1 as f32 * k, y1 as f32 * k, y0 as f32 * k) };
    let p = host_ref(vm).proto("TextMetrics").unwrap_or_else(|| vm.proto(js::vm::I::ObjectProto));
    let o = vm.alloc(Some(p), Class::Ordinary);
    let values = [
        ("width", width),
        ("actualBoundingBoxLeft", -(left + dx)),
        ("actualBoundingBoxRight", right + dx),
        ("actualBoundingBoxAscent", up - dy),
        ("actualBoundingBoxDescent", -down + dy),
        ("fontBoundingBoxAscent", a - dy),
        ("fontBoundingBoxDescent", d + dy),
        ("emHeightAscent", em_a - dy),
        ("emHeightDescent", em_d + dy),
        ("hangingBaseline", em_a * 0.8 - dy),
        ("alphabeticBaseline", -dy),
        ("ideographicBaseline", -d - dy),
    ];
    for (name, v) in values {
        vm.put(o, name, num(v), ENUMERABLE);
    }
    Ok(Value::Obj(o))
}

// ------------------------------------------------------------------------------------------- reading pixels

/// An ImageData of `w` by `h` pixels holding `bytes` (RGBA), or its data array if one's given.
fn image_data(vm: &mut Vm, w: u32, h: u32, data: Result<Vec<u8>, Gc>) -> Value {
    let arr = match data {
        Ok(bytes) => js::builtins::typed::clamped_array(vm, bytes),
        Err(g) => g,
    };
    let p = host_ref(vm).proto("ImageData").unwrap_or_else(|| vm.proto(js::vm::I::ObjectProto));
    let g = vm.alloc(Some(p), Class::Host(IMAGE_DATA, 0));
    vm.put(g, "width", Value::Num(w as f64), ENUMERABLE);
    vm.put(g, "height", Value::Num(h as f64), ENUMERABLE);
    vm.put(g, "data", Value::Obj(arr), ENUMERABLE);
    vm.put(g, "colorSpace", js("srgb"), ENUMERABLE);
    Value::Obj(g)
}

/// An ImageData's size to make: an error past what a page may have.
fn image_size(vm: &mut Vm, w: i64, h: i64, what: &str) -> Result<(u32, u32), Value> {
    if w == 0 || h == 0 {
        let which = if w == 0 { "width" } else { "height" };
        return Err(dom_exception(vm, "IndexSizeError", &format!("Failed to execute '{what}': The source {which} is 0.")));
    }
    let (w, h) = (w.unsigned_abs(), h.unsigned_abs());
    if w * h > MOST_PIXELS {
        return Err(vm.range_error(&format!("Failed to execute '{what}': Out of memory at ImageData creation.")));
    }
    Ok((w as u32, h as u32))
}

fn image_data_new(vm: &mut Vm, _nt: &Value, args: &[Value], _f: Gc) -> R {
    if let Some(Value::Obj(g)) = args.first()
        && let Class::TypedArray(t) = &vm.heap.get(*g).class
    {
        if !matches!(t.kind, ElemKind::Uint8Clamped) {
            return Err(vm.type_error("Failed to construct 'ImageData': parameter 1 is not of type 'Uint8ClampedArray'."));
        }
        let len = js::builtins::typed::bytes_of(vm, *g).map_or(0, |b| b.len());
        if len == 0 || len % 4 != 0 {
            return Err(dom_exception(vm, "InvalidStateError", "Failed to construct 'ImageData': The input data has zero elements, or its length is not a multiple of 4."));
        }
        let w = vm.to_uint32(&arg(args, 1))?;
        if w == 0 || (len / 4) % w as usize != 0 {
            return Err(dom_exception(vm, "IndexSizeError", "Failed to construct 'ImageData': The input data length is not a multiple of (4 * width)."));
        }
        let h = (len / 4 / w as usize) as u32;
        if let Some(hv) = args.get(2)
            && !hv.is_undefined()
            && vm.to_uint32(hv)? != h
        {
            return Err(dom_exception(vm, "IndexSizeError", "Failed to construct 'ImageData': The input data length is not equal to (4 * width * height)."));
        }
        return Ok(image_data(vm, w, h, Err(*g)));
    }
    if args.len() < 2 {
        return Err(vm.type_error("Failed to construct 'ImageData': 2 arguments required."));
    }
    let w = vm.to_uint32(&args[0])? as i64;
    let h = vm.to_uint32(&args[1])? as i64;
    let (w, h) = image_size(vm, w, h, "ImageData")?;
    Ok(image_data(vm, w, h, Ok(vec![0; w as usize * h as usize * 4])))
}

fn create_image_data(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    ctx_node(vm, this)?;
    let (w, h) = match args.first() {
        Some(Value::Obj(g)) if matches!(vm.heap.get(*g).class, Class::Host(IMAGE_DATA, _)) => {
            let w = get(vm, *g, "width")?;
            let h = get(vm, *g, "height")?;
            (vm.to_number(&w)? as i64, vm.to_number(&h)? as i64)
        }
        _ => {
            if args.len() < 2 {
                return Err(vm.type_error("Failed to execute 'createImageData' on 'CanvasRenderingContext2D': 2 arguments required."));
            }
            (vm.to_int32(&args[0])? as i64, vm.to_int32(&args[1])? as i64)
        }
    };
    let (w, h) = image_size(vm, w, h, "createImageData")?;
    Ok(image_data(vm, w, h, Ok(vec![0; w as usize * h as usize * 4])))
}

fn get_image_data(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (_, i) = ctx(vm, this)?;
    if args.len() < 4 {
        return Err(vm.type_error(&format!("Failed to execute 'getImageData' on 'CanvasRenderingContext2D': 4 arguments required, but only {} present.", args.len())));
    }
    let mut v = [0i64; 4];
    for (k, x) in v.iter_mut().enumerate() {
        *x = vm.to_int32(&args[k])? as i64;
    }
    let [mut sx, mut sy, sw, sh] = v;
    let (w, h) = image_size(vm, sw, sh, "getImageData")?;
    if sw < 0 {
        sx += sw;
    }
    if sh < 0 {
        sy += sh;
    }
    let c = canvas_mut(vm, i);
    let mut out = vec![0u8; w as usize * h as usize * 4];
    if c.ensure() {
        let (cw, ch) = (c.w as i64, c.h as i64);
        for y in 0..h as i64 {
            let py = sy + y;
            if py < 0 || py >= ch {
                continue;
            }
            for x in 0..w as i64 {
                let px = sx + x;
                if px < 0 || px >= cw {
                    continue;
                }
                let s = (py * cw + px) as usize;
                let d = ((y * w as i64 + x) * 4) as usize;
                out[d..d + 3].copy_from_slice(&c.rgb[s * 3..s * 3 + 3]);
                out[d + 3] = c.alpha[s];
                if out[d + 3] == 0 {
                    out[d..d + 3].copy_from_slice(&[0, 0, 0]);
                }
            }
        }
    }
    Ok(image_data(vm, w, h, Ok(out)))
}

fn put_image_data(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (_, i) = ctx(vm, this)?;
    let g = match args.first() {
        Some(Value::Obj(g)) if matches!(vm.heap.get(*g).class, Class::Host(IMAGE_DATA, _)) => *g,
        _ => return Err(vm.type_error("Failed to execute 'putImageData' on 'CanvasRenderingContext2D': parameter 1 is not of type 'ImageData'.")),
    };
    if args.len() < 3 {
        return Err(vm.type_error("Failed to execute 'putImageData' on 'CanvasRenderingContext2D': 3 arguments required."));
    }
    let dx = vm.to_int32(&args[1])? as i64;
    let dy = vm.to_int32(&args[2])? as i64;
    let iw = get(vm, g, "width")?;
    let ih = get(vm, g, "height")?;
    let (iw, ih) = (vm.to_number(&iw)? as i64, vm.to_number(&ih)? as i64);
    // The part of it put: all of it, or the dirty rectangle given.
    let (mut rx, mut ry, mut rw, mut rh) = (0i64, 0i64, iw, ih);
    if args.len() >= 7 {
        rx = vm.to_int32(&args[3])? as i64;
        ry = vm.to_int32(&args[4])? as i64;
        rw = vm.to_int32(&args[5])? as i64;
        rh = vm.to_int32(&args[6])? as i64;
        if rw < 0 {
            rx += rw;
            rw = -rw;
        }
        if rh < 0 {
            ry += rh;
            rh = -rh;
        }
    }
    if rx < 0 {
        rw += rx;
        rx = 0;
    }
    if ry < 0 {
        rh += ry;
        ry = 0;
    }
    rw = rw.min(iw - rx);
    rh = rh.min(ih - ry);
    if rw <= 0 || rh <= 0 {
        return Ok(Value::Undefined);
    }
    let data = match get(vm, g, "data")? {
        Value::Obj(d) => d,
        _ => return Ok(Value::Undefined),
    };
    // (The pixels read from the array where they are, and written into the canvas: both in the engine's keeping.)
    let bytes = match js::builtins::typed::bytes_of(vm, data) {
        Some(b) => b.to_vec(),
        None => return Ok(Value::Undefined),
    };
    let c = canvas_mut(vm, i);
    if bytes.len() < (iw * ih * 4) as usize || !c.ensure() {
        return Ok(Value::Undefined);
    }
    let (cw, ch) = (c.w as i64, c.h as i64);
    let opaque = c.opaque;
    for y in ry..ry + rh {
        let py = dy + y;
        if py < 0 || py >= ch {
            continue;
        }
        for x in rx..rx + rw {
            let px = dx + x;
            if px < 0 || px >= cw {
                continue;
            }
            let s = ((y * iw + x) * 4) as usize;
            let d = (py * cw + px) as usize;
            let a = bytes[s + 3];
            if opaque {
                for j in 0..3 {
                    c.rgb[d * 3 + j] = ((bytes[s + j] as u32 * a as u32 + 127) / 255) as u8;
                }
                c.alpha[d] = 255;
            } else {
                c.rgb[d * 3..d * 3 + 3].copy_from_slice(&bytes[s..s + 3]);
                c.alpha[d] = a;
            }
        }
    }
    c.version += 1;
    host(vm).canvas_drawn = true;
    Ok(Value::Undefined)
}
