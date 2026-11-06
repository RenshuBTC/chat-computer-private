//! Each element's style, worked out as a browser does it: the rules whose selectors match it (the browser's own sheet,
//! then the page's sheets, then its style attribute), in the cascade's order (importance, origin, specificity, then
//! order of appearance), on top of what it inherits from its parent; custom properties (--x) and var() included.
//! Lengths are worked out to device pixels here (em, rem, vw...), but for percentages, which wait for the layout.

use crate::css::{self, AttrOp, Combinator, Compound, Decl, Pseudo, PseudoElement, Selector, Sheet, Tok, Viewport};
use crate::dom::{Data, Document, NodeId};
use crate::values::{self, Color, Len, Units};
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Display {
    None,
    Contents,
    Block,
    /// A block that's a new formatting context ("flow-root").
    FlowRoot,
    Inline,
    InlineBlock,
    ListItem,
    Flex,
    InlineFlex,
    Grid,
    InlineGrid,
    Table,
    InlineTable,
    TableRow,
    TableCell,
    TableRowGroup,
    TableHeaderGroup,
    TableFooterGroup,
    TableColumn,
    TableColumnGroup,
    TableCaption,
}

impl Display {
    pub fn is_inline_level(self) -> bool {
        matches!(self, Display::Inline | Display::InlineBlock | Display::InlineFlex | Display::InlineGrid | Display::InlineTable)
    }

    /// The block-level one of an inline-level display (floats, absolutely positioned boxes, and flex and grid items
    /// are "blockified").
    pub fn blockified(self) -> Display {
        match self {
            Display::Inline | Display::InlineBlock => Display::Block,
            Display::InlineFlex => Display::Flex,
            Display::InlineGrid => Display::Grid,
            Display::InlineTable => Display::Table,
            Display::TableRow | Display::TableCell | Display::TableRowGroup | Display::TableHeaderGroup
            | Display::TableFooterGroup | Display::TableCaption => Display::Block,
            d => d,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Position {
    Static,
    Relative,
    Absolute,
    Fixed,
    Sticky,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Float {
    None,
    Left,
    Right,
}

/// A size: width, height, their minimums and maximums, margins and offsets.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Size {
    Auto,
    Len(Len),
    MinContent,
    MaxContent,
    FitContent,
    /// max-width's and max-height's "none".
    None,
}

impl Size {
    pub fn len(self) -> Option<Len> {
        match self {
            Size::Len(l) => Some(l),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TextAlign {
    Left,
    Right,
    Center,
    Justify,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WhiteSpace {
    Normal,
    NoWrap,
    Pre,
    PreWrap,
    PreLine,
}

impl WhiteSpace {
    /// Whether spaces and line breaks are kept as they are.
    pub fn keeps_spaces(self) -> bool {
        matches!(self, WhiteSpace::Pre | WhiteSpace::PreWrap)
    }
    pub fn keeps_breaks(self) -> bool {
        matches!(self, WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::PreLine)
    }
    pub fn wraps(self) -> bool {
        !matches!(self, WhiteSpace::NoWrap | WhiteSpace::Pre)
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum LineHeight {
    Normal,
    /// A multiple of the font's size (inherited as the multiple).
    Factor(f32),
    Px(f32),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum VAlign {
    Baseline,
    Top,
    Middle,
    Bottom,
    TextTop,
    TextBottom,
    Sub,
    Super,
    Len(f32),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ListStyle {
    None,
    Disc,
    Circle,
    Square,
    Decimal,
    LowerAlpha,
    UpperAlpha,
    LowerRoman,
    UpperRoman,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Transform {
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BorderStyle {
    None,
    Solid,
    Dashed,
    Dotted,
    Double,
    Inset,
    Outset,
    Groove,
    Ridge,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FlexDirection {
    Row,
    RowReverse,
    Column,
    ColumnReverse,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Normal,
    Start,
    End,
    Center,
    Stretch,
    Baseline,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

pub use crate::effects::Image;
use crate::effects::{self, ClipPath, Filter, Layers, LayerProp, ObjectFit, Pos, TFn};

/// grid-template-columns' and -rows' tracks.
#[derive(Clone, Debug, PartialEq)]
pub enum Track {
    Len(Len),
    Fr(f32),
    Auto,
    MinContent,
    MaxContent,
    /// minmax(min, max).
    MinMax(Box2<Track>),
    /// repeat(auto-fill or auto-fit, tracks).
    AutoRepeat(Vec<Track>),
}

/// Two of something (minmax()'s ends), boxed for the recursion's sake.
pub type Box2<T> = alloc::boxed::Box<(T, T)>;

/// Where a grid item goes: its start line (1-based, negative from the end, 0 for auto) and span.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct GridPlace {
    pub start: i32,
    pub end: i32,
    pub span: u32,
}

#[derive(Clone, Debug)]
pub struct Style {
    pub display: Display,
    pub position: Position,
    pub float: Float,
    pub clear: Float,
    pub clear_both: bool,
    /// top, right, bottom, left.
    pub inset: [Size; 4],
    pub width: Size,
    pub height: Size,
    pub min_width: Size,
    pub min_height: Size,
    pub max_width: Size,
    pub max_height: Size,
    pub margin: [Size; 4],
    pub padding: [Len; 4],
    pub border_width: [f32; 4],
    pub border_style: [BorderStyle; 4],
    pub border_color: [Color; 4],
    /// Its corners' rounding: top left, top right, bottom right, bottom left (circular: a percentage of its smaller
    /// side, so 50% rounds a square into a circle).
    pub radius: [Len; 4],
    /// Its box-shadow's shadows (the first on top), outside its border box or (inset) inside its padding box.
    pub shadows: Option<Arc<Vec<Shadow>>>,
    pub border_box: bool,
    pub color: Color,
    pub background: Color,
    /// Its background's pictures and gradients, and how each is sized, placed and repeated (None: the colour alone).
    pub bg: Option<Arc<Layers>>,
    /// Its mask's layers: what of it shows (None, or none with a picture: all of it).
    pub mask: Option<Arc<Layers>>,
    pub font_size: f32,
    /// How many times "medium" its font size is, when it comes from a keyword (medium, small...: the page's own,
    /// or none it inherited unchanged, or em and % of one): 0 when it doesn't. Chrome's medium is 13px rather than 16
    /// in the generic monospace family alone, and such a size follows it.
    pub keyword_size: f32,
    /// Its font family is the generic monospace one alone ("monospace": not "monospace, monospace").
    pub generic_mono: bool,
    pub font_weight: u16,
    /// font-stretch: a percentage of normal width.
    pub font_stretch: u16,
    pub italic: bool,
    pub mono: bool,
    /// The family asked for is sans-serif (drawn in the system's sans-serif font; else its serif one).
    pub sans: bool,
    pub line_height: LineHeight,
    pub text_align: TextAlign,
    pub text_indent: Len,
    pub underline: bool,
    pub line_through: bool,
    pub overline: bool,
    pub decoration_color: Option<Color>,
    pub transform_text: Transform,
    pub white_space: WhiteSpace,
    /// image-rendering: pixelated (or crisp-edges): its pictures made bigger pixel by pixel, not smoothly.
    pub pixelated: bool,
    /// text-overflow: ellipsis (a line too long for a box that clips it ends in "…").
    pub ellipsis: bool,
    /// -webkit-line-clamp: how many lines are shown (0: all), with display: -webkit-box and -webkit-box-orient:
    /// vertical, as Chrome has it; and whether its display and orient are those.
    pub line_clamp: u32,
    pub webkit_box: bool,
    pub box_vertical: bool,
    pub vertical_align: VAlign,
    pub list_style: ListStyle,
    pub list_inside: bool,
    pub list_image: bool,
    pub visible: bool,
    pub opacity: f32,
    pub clip: bool,
    pub z_index: Option<i32>,
    pub aspect_ratio: Option<f32>,
    /// aspect-ratio: auto <ratio>: a picture's own ratio first, where it has one.
    pub aspect_auto: bool,
    pub object_fit: ObjectFit,
    pub object_position: (Pos, Pos),
    pub flex_direction: FlexDirection,
    pub flex_wrap: bool,
    pub justify_content: Align,
    pub align_items: Align,
    pub align_self: Align,
    pub align_content: Align,
    pub justify_items: Align,
    pub justify_self: Align,
    pub grow: f32,
    pub shrink: f32,
    pub basis: Size,
    pub order: i32,
    pub row_gap: Len,
    pub column_gap: Len,
    pub grid_columns: Vec<Track>,
    pub grid_rows: Vec<Track>,
    pub grid_areas: Vec<Vec<String>>,
    pub grid_area: Option<String>,
    pub grid_column: GridPlace,
    pub grid_row: GridPlace,
    pub grid_auto_columns: bool,
    pub border_collapse: bool,
    pub border_spacing: f32,
    /// ::before's and ::after's content, as words.
    pub content: Option<String>,
    /// Its transform, when it only moves it: by how much (% of its own size). The layout moves it, with all that's
    /// in it.
    pub translate: (Len, Len),
    /// transform's functions (None: none), and translate's, rotate's and scale's (applied in that order, before
    /// transform's).
    pub transform: Option<Arc<Vec<TFn>>>,
    pub transform_parts: [Option<TFn>; 3],
    /// It has a transform that does more than move it: it's drawn through its matrix.
    pub turned: bool,
    /// transform-origin (its border box's percentages).
    pub origin: (Len, Len),
    /// backface-visibility: hidden (turned to show its back, it isn't drawn).
    pub backface_hidden: bool,
    pub filter: Option<Arc<Vec<Filter>>>,
    pub backdrop: Option<Arc<Vec<Filter>>>,
    /// mix-blend-mode: 255 for normal.
    pub blend: u8,
    pub isolate: bool,
    pub clip_path: Option<Arc<ClipPath>>,
    /// contain: paint (strict, content): what's in it clipped to it, and it a stacking context.
    pub contain_paint: bool,
    /// will-change names something that makes it a stacking context (transform, opacity, filter...), and whether
    /// that's a transform (it's then what fixed boxes in it are placed in).
    pub will_change: bool,
    pub will_transform: bool,
    pub custom: Arc<BTreeMap<String, Arc<Vec<Tok>>>>,
    pub hidden_text: bool,
    /// A table's caption goes under it (caption-side: bottom).
    pub caption_bottom: bool,
    /// What an <svg>'s shapes are filled and outlined with, as the page's style sheets say (inherited; none: as the
    /// picture's own text says).
    pub fill: Option<SvgPaint>,
    pub stroke: Option<SvgPaint>,
    /// It's absolutely positioned and its display was inline-level before that made it block-level: where it would
    /// have been is on its line, not under it.
    pub inline_origin: bool,
    /// Its animations (animation's lists, a value an animation).
    pub animations: Option<Arc<Animations>>,
    /// The font families it asks for, in order, in lower case (font-family's list).
    pub families: Arc<Vec<String>>,
    /// The page's font (text::loaded's number) its words are set in, if one of its families is a web font that has
    /// come; and whether it's made bold and leaned (the face found is lighter, or upright).
    pub web_face: Option<(u16, bool, bool)>,
}

/// A box's animations' lists (animation-name's, and the others' for each name, repeated where shorter): their
/// iteration counts (infinity for infinite), directions (0 normal, 1 reverse, 2 alternate, 3 alternate-reverse) and
/// fill modes (0 none, 1 forwards, 2 backwards, 3 both).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Animations {
    pub names: Vec<String>,
    pub counts: Vec<f32>,
    pub directions: Vec<u8>,
    pub fills: Vec<u8>,
}

impl Animations {
    /// Where in its keyframes (0 to 1) each named animation leaves the box drawn some time after the page has come
    /// (its animations over, but those that go on for ever): the `i`th's, or None where it leaves it as it is.
    pub fn resting(&self, i: usize) -> Option<f32> {
        let count = effects::nth(&self.counts, i, 1.0);
        let dir = effects::nth(&self.directions, i, 0);
        let fill = effects::nth(&self.fills, i, 0);
        if count.is_infinite() {
            // (One that goes on: its first frame, as it is when it starts again.)
            return Some(if dir == 1 || dir == 3 { 1.0 } else { 0.0 });
        }
        if fill != 1 && fill != 3 {
            return None;
        }
        // (Its last iteration's end: alternating ones turn round each time.)
        let iterations = crate::num::ceil(count.max(0.0)).max(1.0) as u32;
        let flipped = match dir {
            1 => true,
            2 => iterations.is_multiple_of(2),
            3 => !iterations.is_multiple_of(2),
            _ => false,
        };
        let frac = count - crate::num::floor(count);
        let end = if frac > 0.0 { frac } else { 1.0 };
        Some(if flipped { 1.0 - end } else { end })
    }
}

/// animation's value read into its lists (None if it isn't one).
fn animation_shorthand(v: &[Tok]) -> Option<Animations> {
    let mut a = Animations::default();
    for one in css::split_commas(v) {
        let mut name = String::from("none");
        let (mut count, mut dir, mut fill) = (1.0f32, 0u8, 0u8);
        for p in parts(one) {
            match values::trim(p) {
                [Tok::Number(n)] => count = n.max(0.0),
                [Tok::Dimension(..)] | [Tok::Function(_), ..] => {}
                [Tok::Str(s)] => name = s.clone(),
                [Tok::Ident(w)] => match w.to_ascii_lowercase().as_str() {
                    "infinite" => count = f32::INFINITY,
                    "normal" => dir = 0,
                    "reverse" => dir = 1,
                    "alternate" => dir = 2,
                    "alternate-reverse" => dir = 3,
                    "forwards" => fill = 1,
                    "backwards" => fill = 2,
                    "both" => fill = 3,
                    "ease" | "linear" | "ease-in" | "ease-out" | "ease-in-out" | "step-start" | "step-end" | "running" | "paused" | "initial" => {}
                    "none" => name = String::from("none"),
                    _ => name = w.clone(),
                },
                _ => return None,
            }
        }
        a.names.push(name);
        a.counts.push(count);
        a.directions.push(dir);
        a.fills.push(fill);
    }
    Some(a)
}

/// A web font a page names (@font-face): its face, and its number in text::loaded once it has come.
#[derive(Clone, Debug)]
pub struct WebFont {
    pub face: css::FontFace,
    pub loaded: Option<u16>,
}

/// How far a face's weights are from the weight asked for, as CSS's font matching orders them (smaller nearer): in
/// its range, none; asked for 400 to 500, heavier up to 500 first, then lighter, then heavier still; under 400,
/// lighter first; over 500, heavier first.
fn weight_rank(range: (u16, u16), want: u16) -> u32 {
    let (lo, hi) = (range.0 as u32, range.1 as u32);
    let w = want as u32;
    if lo <= w && w <= hi {
        return 0;
    }
    let (above, below) = (lo > w, hi < w);
    let class = match (w, above, below) {
        (400..=500, true, _) if lo <= 500 => 1,
        (400..=500, _, true) => 2,
        (400..=500, true, _) => 3,
        (0..=399, _, true) => 1,
        (0..=399, true, _) => 2,
        (_, true, _) => 1,
        _ => 2,
    };
    class * 10_000 + if above { lo - w } else { w - hi }
}

/// How far a face's widths are from the width asked for, as CSS's font matching orders them (smaller nearer): in its
/// range, none; asked for normal or narrower, narrower ones first, then wider; asked for wider, wider ones first.
fn stretch_rank(range: (u16, u16), want: u16) -> u32 {
    let (lo, hi, w) = (range.0 as u32, range.1 as u32, want as u32);
    if lo <= w && w <= hi {
        return 0;
    }
    let narrower = hi < w;
    let first = if w <= 100 { narrower } else { !narrower };
    (if first { 1 } else { 2 }) * 10_000 + if narrower { w - hi } else { lo - w }
}

/// The web font (its place in `fonts`) CSS's font matching sets a family list's words in at a weight, width and
/// style: the first of its families that an @font-face names; of that family's faces, the nearest in width, of those
/// those in its style (or, if it has none, the others), the nearest in weight, and of those the one for Latin letters
/// (a family whose faces are each for a part of Unicode, as Google Fonts sends them). With whether it's made bold (a
/// weight of 600 or more from a face lighter, or from a variable one's range) and leaned (italic from an upright
/// face).
pub fn pick_font(fonts: &[WebFont], families: &[String], weight: u16, italic: bool, stretch: u16) -> Option<(usize, bool, bool)> {
    for family in families {
        let faces: Vec<usize> = (0..fonts.len()).filter(|&i| fonts[i].face.family == *family).collect();
        if faces.is_empty() {
            continue;
        }
        let nearest = faces.iter().map(|&i| stretch_rank(fonts[i].face.stretch, stretch)).min()?;
        let faces: Vec<usize> = faces.into_iter().filter(|&i| stretch_rank(fonts[i].face.stretch, stretch) == nearest).collect();
        let styled: Vec<usize> = faces.iter().copied().filter(|&i| fonts[i].face.italic == italic).collect();
        let pool = if styled.is_empty() { faces } else { styled };
        let best = pool.iter().map(|&i| weight_rank(fonts[i].face.weight, weight)).min()?;
        let near: Vec<usize> = pool.into_iter().filter(|&i| weight_rank(fonts[i].face.weight, weight) == best).collect();
        let chosen = near.iter().copied().find(|&i| fonts[i].face.covers('a' as u32)).unwrap_or(near[0]);
        let f = &fonts[chosen].face;
        let bold = weight >= 600 && f.weight.0 < 600;
        return Some((chosen, bold, italic && !f.italic));
    }
    None
}

/// font-family's names, in order, in lower case: a name of several words as one, its quotes taken off.
fn family_list(v: &[Tok]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut name = String::new();
    for t in v {
        match t {
            Tok::Ident(i) | Tok::Str(i) => {
                if !name.is_empty() {
                    name.push(' ');
                }
                name.push_str(&i.to_ascii_lowercase());
            }
            Tok::Comma => {
                if !name.is_empty() {
                    out.push(core::mem::take(&mut name));
                }
            }
            _ => {}
        }
    }
    if !name.is_empty() {
        out.push(name);
    }
    out
}

/// An SVG paint (a fill or a stroke) a style sheet gives: none, currentColor or a colour.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SvgPaint {
    None,
    Current,
    Color(Color),
}

impl SvgPaint {
    /// As an SVG attribute's value.
    pub fn svg(self) -> String {
        match self {
            SvgPaint::None => String::from("none"),
            SvgPaint::Current => String::from("currentColor"),
            SvgPaint::Color(c) if c >> 24 == 0xFF => alloc::format!("#{:06x}", c & 0x00FF_FFFF),
            SvgPaint::Color(c) => alloc::format!("#{:06x}{:02x}", c & 0x00FF_FFFF, c >> 24),
        }
    }
}

impl Style {
    /// The initial values (for the root; what isn't inherited starts as these everywhere).
    pub fn initial(dpr: f32) -> Style {
        Style {
            display: Display::Inline,
            position: Position::Static,
            float: Float::None,
            clear: Float::None,
            clear_both: false,
            inset: [Size::Auto; 4],
            width: Size::Auto,
            height: Size::Auto,
            min_width: Size::Auto,
            min_height: Size::Auto,
            max_width: Size::None,
            max_height: Size::None,
            margin: [Size::Len(Len::ZERO); 4],
            padding: [Len::ZERO; 4],
            border_width: [0.0; 4],
            border_style: [BorderStyle::None; 4],
            border_color: [values::BLACK; 4],
            radius: [Len::ZERO; 4],
            shadows: None,
            border_box: false,
            color: values::BLACK,
            background: values::TRANSPARENT,
            bg: None,
            mask: None,
            font_size: 16.0 * dpr,
            keyword_size: 1.0,
            generic_mono: false,
            font_weight: 400,
            font_stretch: 100,
            italic: false,
            mono: false,
            sans: false,
            line_height: LineHeight::Normal,
            text_align: TextAlign::Left,
            text_indent: Len::ZERO,
            underline: false,
            line_through: false,
            overline: false,
            decoration_color: None,
            transform_text: Transform::None,
            white_space: WhiteSpace::Normal,
            ellipsis: false,
            pixelated: false,
            line_clamp: 0,
            webkit_box: false,
            box_vertical: false,
            vertical_align: VAlign::Baseline,
            list_style: ListStyle::Disc,
            list_inside: false,
            list_image: false,
            visible: true,
            opacity: 1.0,
            clip: false,
            z_index: None,
            aspect_ratio: None,
            aspect_auto: false,
            object_fit: ObjectFit::Fill,
            object_position: (Pos::CENTER, Pos::CENTER),
            flex_direction: FlexDirection::Row,
            flex_wrap: false,
            justify_content: Align::Normal,
            align_items: Align::Normal,
            align_self: Align::Normal,
            align_content: Align::Normal,
            justify_items: Align::Normal,
            justify_self: Align::Normal,
            grow: 0.0,
            shrink: 1.0,
            basis: Size::Auto,
            order: 0,
            row_gap: Len::ZERO,
            column_gap: Len::ZERO,
            grid_columns: Vec::new(),
            grid_rows: Vec::new(),
            grid_areas: Vec::new(),
            grid_area: None,
            grid_column: GridPlace::default(),
            grid_row: GridPlace::default(),
            grid_auto_columns: false,
            border_collapse: false,
            border_spacing: 0.0,
            content: None,
            translate: (Len::ZERO, Len::ZERO),
            transform: None,
            transform_parts: [None; 3],
            turned: false,
            origin: (Len::pct(50.0), Len::pct(50.0)),
            backface_hidden: false,
            filter: None,
            backdrop: None,
            blend: 255,
            isolate: false,
            clip_path: None,
            contain_paint: false,
            will_change: false,
            will_transform: false,
            custom: Arc::new(BTreeMap::new()),
            hidden_text: false,
            caption_bottom: false,
            fill: None,
            stroke: None,
            inline_origin: false,
            families: Arc::new(Vec::new()),
            web_face: None,
            animations: None,
        }
    }

    /// Its transform's functions, in the order they're applied: translate's, rotate's, scale's, then transform's.
    pub fn transform_fns(&self) -> Vec<TFn> {
        let mut out: Vec<TFn> = self.transform_parts.iter().flatten().copied().collect();
        if let Some(t) = &self.transform {
            out.extend(t.iter().copied());
        }
        out
    }

    /// Whether it has a transform (a move alone, or none in 2D, "translateZ(0)", counts): it's then what fixed and
    /// absolutely positioned boxes in it are placed in, and a stacking context.
    pub fn has_transform(&self) -> bool {
        self.transform.is_some() || self.transform_parts.iter().any(|t| t.is_some()) || self.will_transform
    }

    /// What it has that makes it a stacking context, positioned or not (a positioned box with a z-index is one too,
    /// as are flex and grid items with one: the layout knows which those are).
    pub fn effects_context(&self) -> bool {
        self.opacity < 1.0
            || self.has_transform()
            || self.filter.is_some()
            || self.backdrop.is_some()
            || self.mask.as_ref().is_some_and(|m| m.any())
            || self.clip_path.is_some()
            || self.blend != 255
            || self.isolate
            || self.contain_paint
            || self.will_change
            || matches!(self.position, Position::Fixed | Position::Sticky)
    }

    /// What a child starts from: the inherited properties of this one, the rest initial.
    pub fn inherit(&self, dpr: f32) -> Style {
        let mut s = Style::initial(dpr);
        s.color = self.color;
        s.font_size = self.font_size;
        s.keyword_size = self.keyword_size;
        s.generic_mono = self.generic_mono;
        s.font_weight = self.font_weight;
        s.font_stretch = self.font_stretch;
        s.italic = self.italic;
        s.mono = self.mono;
        s.sans = self.sans;
        s.line_height = self.line_height;
        s.text_align = self.text_align;
        s.text_indent = self.text_indent;
        s.transform_text = self.transform_text;
        s.white_space = self.white_space;
        s.pixelated = self.pixelated;
        s.list_style = self.list_style;
        s.list_inside = self.list_inside;
        s.list_image = self.list_image;
        s.visible = self.visible;
        s.border_collapse = self.border_collapse;
        s.caption_bottom = self.caption_bottom;
        s.border_spacing = self.border_spacing;
        s.justify_items = Align::Normal;
        s.custom = self.custom.clone();
        // (Text decorations are drawn through the boxes inside, as if inherited.)
        s.underline = self.underline;
        s.line_through = self.line_through;
        s.overline = self.overline;
        s.decoration_color = self.decoration_color;
        s.hidden_text = self.hidden_text;
        s.fill = self.fill;
        s.stroke = self.stroke;
        s.families = self.families.clone();
        // (Text inherits its element's font; an element's own is worked out again from what it inherits.)
        s.web_face = self.web_face;
        s
    }

    /// The line height in device pixels.
    pub fn line_px(&self) -> f32 {
        match self.line_height {
            // (From its font's own ascent, descent and line gap, as Chrome works it out.)
            LineHeight::Normal => match self.web_face.and_then(|(id, ..)| text::loaded::normal_line(id, self.font_size)) {
                Some(h) => h,
                None => self.family().normal_line(self.font_size),
            },
            LineHeight::Factor(f) => self.font_size * f,
            LineHeight::Px(p) => p,
        }
    }

    /// Its kind of font: Chrome's default serif, sans-serif or monospace.
    pub fn family(&self) -> text::Family {
        family_of(self.mono, self.sans)
    }

    pub fn is_positioned(&self) -> bool {
        self.position != Position::Static
    }

    pub fn out_of_flow(&self) -> bool {
        matches!(self.position, Position::Absolute | Position::Fixed)
    }

    pub fn is_bold(&self) -> bool {
        self.font_weight >= 600
    }

    /// Whether it starts a new block formatting context (its floats and margins kept inside).
    pub fn new_context(&self) -> bool {
        self.clip
            || self.float != Float::None
            || self.out_of_flow()
            || matches!(self.display, Display::FlowRoot | Display::InlineBlock | Display::Flex | Display::Grid
                | Display::Table | Display::TableCell | Display::InlineFlex | Display::InlineGrid)
    }
}

// ------------------------------------------------------------------------------------------------ the cascade

/// Every element's style (and its ::before's and ::after's, where they have content).
pub struct Styles {
    pub of: Vec<Option<Arc<Style>>>,
    pub before: BTreeMap<NodeId, Arc<Style>>,
    pub after: BTreeMap<NodeId, Arc<Style>>,
    /// The web fonts (by their place in Sheets' fonts) some element's words are to be set in: to be fetched, if they
    /// haven't been.
    pub wanted_fonts: alloc::collections::BTreeSet<usize>,
}

/// One selector of one rule, ready to match: which sheet and rule it's from, and its order in the cascade.
struct Entry {
    sheet: usize,
    rule: usize,
    selector: usize,
    order: u32,
    /// What its selector's subject is: 0 an element of its sheet's tree, 1 a shadow tree's host (:host), 2 an
    /// element a host has, shown in a slot (::slotted()).
    subject: u8,
}

/// The rules, by the last part of each selector (its id, else a class, else its element's name), so an element only
/// tries those it could match.
#[derive(Default)]
struct Index {
    by_id: BTreeMap<String, Vec<usize>>,
    by_class: BTreeMap<String, Vec<usize>>,
    by_tag: BTreeMap<String, Vec<usize>>,
    any: Vec<usize>,
    pseudo: Vec<usize>,
}

/// What styles are worked out from: the browser's sheet, then the page's.
pub struct Sheets<'a> {
    pub ua: &'a Sheet,
    pub author: &'a [Sheet],
    /// The page's web fonts (its @font-face rules', as they've come).
    pub fonts: &'a [WebFont],
}

pub fn compute(doc: &Document, sheets: &Sheets, view: Viewport, dpr: f32) -> Styles {
    let all: Vec<&Sheet> = core::iter::once(sheets.ua).chain(sheets.author.iter()).collect();
    let mut entries: Vec<Entry> = Vec::new();
    let mut index = Index::default();
    let mut order = 0u32;
    for (si, sheet) in all.iter().enumerate() {
        for (ri, rule) in sheet.rules.iter().enumerate() {
            order += 1;
            for (k, sel) in rule.selectors.iter().enumerate() {
                let e = entries.len();
                let subject = match sel.compounds.last().map(|c| c.pseudos.as_slice()).unwrap_or(&[]) {
                    p if p.iter().any(|p| matches!(p, Pseudo::Host(_))) => 1,
                    p if p.iter().any(|p| matches!(p, Pseudo::Slotted(_))) => 2,
                    _ => 0,
                };
                entries.push(Entry { sheet: si, rule: ri, selector: k, order, subject });
                if sel.pseudo_element.is_some() {
                    index.pseudo.push(e);
                    continue;
                }
                let Some(last) = sel.compounds.last() else { continue };
                if let Some(id) = last.ids.first() {
                    index.by_id.entry(id.clone()).or_default().push(e);
                } else if let Some(c) = last.classes.first() {
                    index.by_class.entry(c.clone()).or_default().push(e);
                } else if let Some(t) = &last.tag {
                    index.by_tag.entry(t.clone()).or_default().push(e);
                } else {
                    index.any.push(e);
                }
            }
        }
    }
    let units_root = Units { em: 16.0 * dpr, rem: 16.0 * dpr, vw: view.width * dpr, vh: view.height * dpr, dpr };
    let mut c = Cascade {
        doc,
        sheets: all,
        entries,
        index,
        view,
        dpr,
        rem: units_root.rem,
        out: Styles { of: vec![None; doc.nodes.len()], before: BTreeMap::new(), after: BTreeMap::new(), wanted_fonts: alloc::collections::BTreeSet::new() },
        fonts: sheets.fonts,
        sibling: SiblingInfo::new(doc),
    };
    // In the order they come, parents before children (no recursion: a page's elements can be deep).
    let root_style = Arc::new(Style::initial(dpr));
    let mut stack: Vec<(NodeId, Arc<Style>)> = vec![(0, root_style)];
    while let Some((id, parent)) = stack.pop() {
        let mine = if c.doc.element(id).is_some() {
            let s = Arc::new(c.style_of(id, &parent));
            // (Every other element's rem is the root's font size: 10px on a page that sets it so, as many do.)
            if c.doc.parent(id) == Some(0) {
                c.rem = s.font_size;
            }
            c.out.of[id] = Some(s.clone());
            s
        } else {
            parent
        };
        // (A shadow host's own children inherit from the slots that show them, not from it; its shadow tree's
        // elements inherit from it.)
        let shadow = c.doc.shadow_of(id);
        if shadow.is_none() {
            for &child in c.doc.children(id).iter().rev() {
                if c.doc.element(child).is_some() {
                    stack.push((child, mine.clone()));
                }
            }
        }
        if let Some(root) = shadow {
            for &child in c.doc.children(root).iter().rev() {
                if c.doc.element(child).is_some() {
                    stack.push((child, mine.clone()));
                }
            }
        }
        if c.doc.name(id) == "slot" {
            for child in c.doc.assigned(id).into_iter().rev() {
                if c.doc.element(child).is_some() {
                    stack.push((child, mine.clone()));
                }
            }
        }
    }
    c.out
}

/// The shadow root whose tree an element is in (None: the page's own tree, or none).
fn tree_scope(doc: &Document, id: NodeId) -> Option<NodeId> {
    let r = doc.root_of(id);
    if r != 0 && doc.shadow_host(r).is_some() { Some(r) } else { None }
}

/// Each element's place among its parent's elements, and how many there are.
pub(crate) struct SiblingInfo {
    index: Vec<u32>,
    count: Vec<u32>,
}

impl SiblingInfo {
    pub(crate) fn new(doc: &Document) -> SiblingInfo {
        let mut index = vec![0u32; doc.nodes.len()];
        let mut count = vec![0u32; doc.nodes.len()];
        for n in &doc.nodes {
            let kids: Vec<NodeId> = n.children.iter().copied().filter(|&k| doc.element(k).is_some()).collect();
            for (i, &k) in kids.iter().enumerate() {
                index[k] = i as u32 + 1;
                count[k] = kids.len() as u32;
            }
        }
        SiblingInfo { index, count }
    }
}

struct Cascade<'a> {
    fonts: &'a [WebFont],
    doc: &'a Document,
    sheets: Vec<&'a Sheet>,
    entries: Vec<Entry>,
    index: Index,
    view: Viewport,
    dpr: f32,
    rem: f32,
    out: Styles,
    sibling: SiblingInfo,
}

/// A declaration's place in the cascade: (tier, specificity, order), lowest first.
type Rank = (u8, u32, u32);

impl Cascade<'_> {
    fn decl(&self, e: &Entry) -> &[Decl] {
        &self.sheets[e.sheet].rules[e.rule].decls
    }

    fn selector(&self, e: &Entry) -> &Selector {
        &self.sheets[e.sheet].rules[e.rule].selectors[e.selector]
    }

    fn style_of(&mut self, id: NodeId, parent: &Style) -> Style {
        let Some(el) = self.doc.element(id) else { return parent.inherit(self.dpr) };
        let mut found: Vec<(Rank, &Decl)> = Vec::new();
        let mut candidates: Vec<usize> = Vec::new();
        if let Some(i) = el.id()
            && let Some(v) = self.index.by_id.get(i)
        {
            candidates.extend(v);
        }
        for class in el.classes() {
            if let Some(v) = self.index.by_class.get(class) {
                candidates.extend(v);
            }
        }
        if let Some(v) = self.index.by_tag.get(&el.name) {
            candidates.extend(v);
        }
        candidates.extend(&self.index.any);
        candidates.sort_unstable();
        candidates.dedup();
        let tree = tree_scope(self.doc, id);
        for &e in &candidates {
            let entry = &self.entries[e];
            if !self.applies(entry, id, tree) {
                continue;
            }
            let sel = self.selector(entry);
            if matches(self.doc, &self.sibling, id, sel) {
                let ua = entry.sheet == 0;
                for d in self.decl(entry) {
                    let tier = match (ua, d.important) {
                        (true, false) => 0,
                        (false, false) => 1,
                        (false, true) => 3,
                        (true, true) => 5,
                    };
                    // (A shadow tree's rules for its host give way to the page's.)
                    let rank = if entry.subject == 1 && !d.important { (tier, 0, 0) } else { (tier, sel.specificity, entry.order) };
                    found.push((rank, d));
                }
            }
        }
        // The style attribute.
        let inline: Vec<Decl> = el.attr("style").map(css::inline_style).unwrap_or_default();
        for d in &inline {
            found.push(((if d.important { 4 } else { 2 }, 0, u32::MAX), d));
        }
        // HTML's attributes that style (presentational hints), below the page's own rules.
        let hints = hints(self.doc, id);
        for d in &hints {
            found.push(((1, 0, 0), d));
        }
        found.sort_by(|a, b| a.0.cmp(&b.0));
        let mut s = self.apply(id, parent, &found);
        self.with_font(&mut s);
        // ::before and ::after.
        let pseudo: Vec<usize> = self.index.pseudo.clone();
        for which in [PseudoElement::Before, PseudoElement::After] {
            let mut pf: Vec<(Rank, &Decl)> = Vec::new();
            for &e in &pseudo {
                let entry = &self.entries[e];
                if !self.applies(entry, id, tree) {
                    continue;
                }
                let sel = self.selector(entry);
                if sel.pseudo_element == Some(which) && matches(self.doc, &self.sibling, id, sel) {
                    let ua = entry.sheet == 0;
                    for d in self.decl(entry) {
                        let tier = if d.important { if ua { 5 } else { 3 } } else if ua { 0 } else { 1 };
                        pf.push(((tier, sel.specificity, entry.order), d));
                    }
                }
            }
            if pf.is_empty() {
                continue;
            }
            pf.sort_by(|a, b| a.0.cmp(&b.0));
            let ps = self.apply_pseudo(id, &s, &pf);
            if let Some(mut ps) = ps {
                self.with_font(&mut ps);
                let map = if which == PseudoElement::Before { &mut self.out.before } else { &mut self.out.after };
                map.insert(id, Arc::new(ps));
            }
        }
        if s.display == Display::None {
            s.content = None;
        }
        s
    }

    /// The web font a style's words are set in, if one of its families is one (noted as wanted, to be fetched).
    fn with_font(&mut self, s: &mut Style) {
        s.web_face = None;
        if self.fonts.is_empty() || s.families.is_empty() {
            return;
        }
        if let Some((i, bold, lean)) = pick_font(self.fonts, &s.families, s.font_weight, s.italic, s.font_stretch) {
            self.out.wanted_fonts.insert(i);
            s.web_face = self.fonts[i].loaded.map(|id| (id, bold, lean));
        }
    }

    /// Whether a rule is for an element at all, by the trees they're in: the browser's own rules are for every
    /// element; the page's for those in its own tree; a shadow tree's for its own elements, its host (:host), and
    /// what the host shows in its slots (::slotted()).
    fn applies(&self, entry: &Entry, id: NodeId, tree: Option<NodeId>) -> bool {
        if entry.sheet == 0 {
            return entry.subject == 0;
        }
        match self.sheets[entry.sheet].scope {
            None => tree.is_none() && entry.subject == 0,
            Some(s) => match entry.subject {
                0 => tree == Some(s),
                1 => self.doc.shadow_of(id) == Some(s),
                _ => self.doc.parent(id).and_then(|p| self.doc.shadow_of(p)) == Some(s),
            },
        }
    }

    fn apply_pseudo(&self, id: NodeId, parent: &Style, found: &[(Rank, &Decl)]) -> Option<Style> {
        let s = self.apply_with(id, parent, found, true);
        let text = s.content.as_ref()?;
        if s.display == Display::None {
            return None;
        }
        // (Icon fonts' characters, which no font here has, and empty content that only draws a box with no size.)
        let shown = text.chars().any(|c| !matches!(c as u32, 0xE000..=0xF8FF | 0xF0000..=0x10FFFF));
        let has_box = s.width.len().is_some() || s.height.len().is_some() || s.background != values::TRANSPARENT
            || s.border_width.iter().any(|&w| w > 0.0);
        if !shown && !has_box {
            return None;
        }
        Some(s)
    }

    fn apply(&self, id: NodeId, parent: &Style, found: &[(Rank, &Decl)]) -> Style {
        self.apply_with(id, parent, found, false)
    }

    fn apply_with(&self, id: NodeId, parent: &Style, found: &[(Rank, &Decl)], pseudo: bool) -> Style {
        let mut s = parent.inherit(self.dpr);
        // Custom properties first (var() needs them), then font-size (em needs it), then color (currentColor), then
        // the rest, each in cascade order.
        let customs: Vec<&Decl> = found.iter().filter(|(_, d)| d.name.starts_with("--")).map(|(_, d)| *d).collect();
        if !customs.is_empty() {
            // (Each name's winning declaration: the last, in cascade order.)
            let mut declared: BTreeMap<&str, &[Tok]> = BTreeMap::new();
            for d in &customs {
                declared.insert(d.name.as_str(), d.value.as_slice());
            }
            let inherited = s.custom.clone();
            let (mut done, mut active, mut cyclic) = (BTreeMap::new(), Vec::new(), Vec::new());
            let mut map = (*inherited).clone();
            for &name in declared.keys() {
                match custom_value(name, &declared, &inherited, &mut done, &mut active, &mut cyclic) {
                    Some(v) => {
                        map.insert(String::from(name), v);
                    }
                    None => {
                        map.remove(name);
                    }
                }
            }
            s.custom = Arc::new(map);
        }
        let is_root = self.doc.parent(id) == Some(0);
        let mut ctx = Ctx { parent, units: Units { em: parent.font_size, rem: self.rem, vw: self.view.width * self.dpr, vh: self.view.height * self.dpr, dpr: self.dpr } };
        for pass in 0..3 {
            for (_, d) in found {
                if d.name.starts_with("--") {
                    continue;
                }
                let early = matches!(d.name.as_str(), "font-size" | "font" | "font-family");
                let colour = d.name == "color";
                let wanted = match pass {
                    0 => early,
                    1 => colour,
                    _ => !early && !colour,
                };
                if !wanted {
                    continue;
                }
                let value: Vec<Tok>;
                let v: &[Tok] = if has_var(&d.value) {
                    match substitute(&d.value, &s.custom, 0) {
                        Some(x) => {
                            value = x;
                            &value
                        }
                        None => continue,
                    }
                } else {
                    &d.value
                };
                apply_one(&mut s, &d.name, v, &ctx);
            }
            if pass == 2
                && let Some(anims) = s.animations.clone()
            {
                // Its animations, as they leave it: their keyframe's declarations over its own (as animations
                // override the page's declarations), the last named on top.
                for (i, name) in anims.names.iter().enumerate() {
                    let Some(at) = anims.resting(i) else { continue };
                    let Some(k) = self.sheets.iter().rev().flat_map(|sh| sh.keyframes.iter().rev()).find(|k| k.name == *name) else { continue };
                    for (places, decls) in &k.frames {
                        if !places.iter().any(|p| (p - at).abs() < 1e-4) {
                            continue;
                        }
                        for dcl in decls {
                            if dcl.name.starts_with("--") || dcl.name.starts_with("animation") {
                                continue;
                            }
                            let value: Vec<Tok>;
                            let v: &[Tok] = if has_var(&dcl.value) {
                                match substitute(&dcl.value, &s.custom, 0) {
                                    Some(x) => {
                                        value = x;
                                        &value
                                    }
                                    None => continue,
                                }
                            } else {
                                &dcl.value
                            };
                            apply_one(&mut s, &dcl.name, v, &ctx);
                        }
                    }
                }
            }
            if pass == 0 {
                // (A keyword's size, medium's 13px in the generic monospace family alone, as Chrome makes it.)
                if s.keyword_size > 0.0 {
                    s.font_size = s.keyword_size * if s.generic_mono { 13.0 } else { 16.0 } * self.dpr;
                }
                ctx.units.em = s.font_size;
                if is_root {
                    // (rem is the root's font size.)
                    ctx.units.rem = s.font_size;
                }
            }
        }
        let _ = pseudo;
        // A clamped -webkit-box (display: -webkit-box, -webkit-box-orient: vertical, -webkit-line-clamp): laid out as
        // a block, its lines after the clamp's left out, as Chrome lays one out.
        if s.line_clamp > 0 && s.webkit_box && s.box_vertical {
            s.display = if s.display == Display::InlineFlex { Display::InlineBlock } else { Display::FlowRoot };
        } else {
            s.line_clamp = 0;
        }
        // Its transform: a move alone is the layout's; anything more is drawn through its matrix.
        s.translate = (Len::ZERO, Len::ZERO);
        s.turned = false;
        if s.transform.is_some() || s.transform_parts.iter().any(|t| t.is_some()) {
            match effects::only_moves(&s.transform_fns()) {
                Some(t) => s.translate = t,
                None => s.turned = true,
            }
        }
        // Blockified: floats, absolutely positioned boxes, the root, and flex and grid items.
        // (A ::before's or ::after's box is in its element's: a flex item of a flex container's.)
        let parent_display = if pseudo { Some(parent.display) } else { self.doc.parent_element(id).and_then(|p| self.out.of.get(p).and_then(|x| x.as_ref())).map(|p| p.display) };
        let in_flex = matches!(parent_display, Some(Display::Flex | Display::InlineFlex | Display::Grid | Display::InlineGrid));
        if s.out_of_flow() {
            s.inline_origin = s.display.is_inline_level();
        }
        if s.float != Float::None || s.out_of_flow() || (is_root && !pseudo) || in_flex {
            s.display = s.display.blockified();
        }
        if s.out_of_flow() {
            s.float = Float::None;
        }
        if s.display == Display::None {
            return s;
        }
        // Text hidden off the screen ("text-indent: -9999px", or clipped to nothing) isn't drawn.
        if s.text_indent.px < -1000.0 {
            s.hidden_text = true;
        }
        s
    }
}

/// HTML attributes that style: width and height (of pictures, tables, cells...), align, bgcolor, border, color...
fn hints(doc: &Document, id: NodeId) -> Vec<Decl> {
    let Some(el) = doc.element(id) else { return Vec::new() };
    let mut out = Vec::new();
    let mut add = |name: &str, css: String| {
        for mut d in css::inline_style(&alloc::format!("{name}: {css}")) {
            d.important = false;
            out.push(d);
        }
    };
    let dim = |v: &str| -> Option<String> {
        let v = v.trim();
        if let Some(p) = v.strip_suffix('%') {
            p.trim().parse::<f32>().ok().map(|n| alloc::format!("{n}%"))
        } else {
            let n: String = v.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
            n.parse::<f32>().ok().map(|n| alloc::format!("{n}px"))
        }
    };
    let name = el.name.as_str();
    if matches!(name, "img" | "table" | "td" | "th" | "col" | "iframe" | "video" | "canvas" | "embed" | "object" | "svg" | "input" | "hr" | "image") {
        if let Some(w) = el.attr("width").and_then(dim) {
            add("width", w);
        }
        if let Some(h) = el.attr("height").and_then(dim)
            && name != "hr"
        {
            add("height", h);
        }
    }
    if let Some(a) = el.attr("align") {
        let a = a.to_ascii_lowercase();
        match (name, a.as_str()) {
            ("img" | "iframe" | "object" | "embed" | "table", "left") => add("float", String::from("left")),
            ("img" | "iframe" | "object" | "embed" | "table", "right") => add("float", String::from("right")),
            ("table", "center") => {
                add("margin-left", String::from("auto"));
                add("margin-right", String::from("auto"));
            }
            ("img", "middle" | "absmiddle" | "center") => add("vertical-align", String::from("middle")),
            ("img", "top") => add("vertical-align", String::from("top")),
            (_, "center" | "middle") => add("text-align", String::from("center")),
            (_, "left" | "right" | "justify") => add("text-align", a.clone()),
            _ => {}
        }
    }
    if let Some(v) = el.attr("valign") {
        add("vertical-align", v.to_ascii_lowercase());
    }
    if let Some(c) = el.attr("bgcolor") {
        add("background-color", legacy_color(c));
    }
    if name == "font" {
        if let Some(c) = el.attr("color") {
            add("color", legacy_color(c));
        }
        if let Some(sz) = el.attr("size") {
            let sz = sz.trim();
            let n: i32 = sz.trim_start_matches(['+', '-']).parse().unwrap_or(3);
            let n = if sz.starts_with('+') { 3 + n } else if sz.starts_with('-') { 3 - n } else { n };
            let size = match n.clamp(1, 7) {
                1 => "x-small",
                2 => "small",
                3 => "medium",
                4 => "large",
                5 => "x-large",
                6 => "xx-large",
                _ => "xxx-large",
            };
            add("font-size", String::from(size));
        }
        if let Some(f) = el.attr("face") {
            add("font-family", String::from(f));
        }
    }
    if name == "body" {
        if let Some(c) = el.attr("text") {
            add("color", legacy_color(c));
        }
        if let Some(m) = el.attr("marginwidth").or(el.attr("leftmargin")).and_then(dim) {
            add("margin-left", m.clone());
            add("margin-right", m);
        }
        if let Some(m) = el.attr("marginheight").or(el.attr("topmargin")).and_then(dim) {
            add("margin-top", m.clone());
            add("margin-bottom", m);
        }
    }
    if name == "table" && el.attr("align").is_none() {
        let mut at = doc.parent(id);
        while let Some(p) = at {
            let Some(e) = doc.element(p) else { break };
            if e.name == "center" || e.attr("align").is_some_and(|a| a.eq_ignore_ascii_case("center")) {
                add("margin-left", String::from("auto"));
                add("margin-right", String::from("auto"));
                break;
            }
            if e.attr("align").is_some() || matches!(e.name.as_str(), "td" | "th" | "table" | "body") {
                break;
            }
            at = doc.parent(p);
        }
    }
    if name == "table" {
        if let Some(b) = el.attr("border") {
            let w = b.trim().parse::<f32>().unwrap_or(1.0);
            if w > 0.0 {
                add("border", alloc::format!("{w}px outset grey"));
            }
        }
        if let Some(cs) = el.attr("cellspacing").and_then(dim) {
            add("border-spacing", cs);
        }
    }
    if matches!(name, "td" | "th") {
        // A table's cellpadding and border reach its cells.
        if let Some(table) = doc.ancestor_named(id, "table").and_then(|t| doc.element(t)) {
            if let Some(p) = table.attr("cellpadding").and_then(dim) {
                add("padding", p);
            }
            if table.attr("border").and_then(|b| b.trim().parse::<f32>().ok()).unwrap_or(0.0) > 0.0 {
                add("border", String::from("1px inset grey"));
            }
        }
        if el.has_attr("nowrap") {
            add("white-space", String::from("nowrap"));
        }
    }
    if matches!(name, "ul" | "ol") && let Some(t) = el.attr("type") {
        let ls = match t {
            "1" => "decimal",
            "a" => "lower-alpha",
            "A" => "upper-alpha",
            "i" => "lower-roman",
            "I" => "upper-roman",
            "circle" => "circle",
            "square" => "square",
            _ => "disc",
        };
        add("list-style-type", String::from(ls));
    }
    if name == "hr" && let Some(w) = el.attr("width").and_then(dim) {
        add("width", w);
    }
    // (An input's size and a textarea's cols and rows make their natural size: layout.rs.)
    out
}

/// A colour as old HTML attributes write it ("red", "#fff", "ff0000").
fn legacy_color(c: &str) -> String {
    let c = c.trim();
    if c.len() == 6 && c.chars().all(|x| x.is_ascii_hexdigit()) {
        return alloc::format!("#{c}");
    }
    String::from(c)
}

fn has_var(t: &[Tok]) -> bool {
    t.iter().any(|x| matches!(x, Tok::Function(f) if f == "var" || f == "env"))
}

/// A custom property's value on an element: its declaration's, with the var()s in it replaced (the element's other
/// custom properties worked out first; those it doesn't declare, as it inherits them). None if it's invalid: a cycle
/// among them (--x: var(--x, 1rem) included) makes all in it so, as CSS says, and a var() in one then takes its
/// fallback.
fn custom_value<'a>(
    name: &'a str,
    declared: &BTreeMap<&'a str, &'a [Tok]>,
    inherited: &BTreeMap<String, Arc<Vec<Tok>>>,
    done: &mut BTreeMap<&'a str, Option<Arc<Vec<Tok>>>>,
    active: &mut Vec<&'a str>,
    cyclic: &mut Vec<&'a str>,
) -> Option<Arc<Vec<Tok>>> {
    if let Some(v) = done.get(name) {
        return v.clone();
    }
    if let Some(at) = active.iter().position(|n| *n == name) {
        for &n in &active[at..] {
            if !cyclic.contains(&n) {
                cyclic.push(n);
            }
        }
        return None;
    }
    let raw = *declared.get(name)?;
    let value = match ident(raw).as_deref() {
        Some("initial") => None,
        Some("inherit" | "unset" | "revert" | "revert-layer") => inherited.get(name).cloned(),
        _ if has_var(raw) => {
            active.push(name);
            let v = substitute_with(
                raw,
                &mut |other: &str| match declared.get_key_value(other) {
                    Some((&k, _)) => custom_value(k, declared, inherited, done, active, cyclic),
                    None => inherited.get(other).cloned(),
                },
                0,
            );
            active.pop();
            v.map(Arc::new)
        }
        _ => Some(Arc::new(raw.to_vec())),
    };
    let value = if cyclic.contains(&name) { None } else { value };
    done.insert(name, value.clone());
    value
}

/// var(--x, fallback) (and env(), as its fallback) replaced by what it stands for: None if one can't be.
fn substitute(t: &[Tok], custom: &BTreeMap<String, Arc<Vec<Tok>>>, depth: u32) -> Option<Vec<Tok>> {
    substitute_with(t, &mut |name: &str| custom.get(name).cloned(), depth)
}

/// var() replaced as substitute does, a custom property's value from `lookup`.
fn substitute_with(t: &[Tok], lookup: &mut dyn FnMut(&str) -> Option<Arc<Vec<Tok>>>, depth: u32) -> Option<Vec<Tok>> {
    if depth > 8 {
        return None;
    }
    let mut out = Vec::with_capacity(t.len());
    let mut i = 0;
    while i < t.len() {
        if let Tok::Function(f) = &t[i]
            && (f == "var" || f == "env")
        {
            // Its arguments, to its ")".
            let mut depth_b = 0;
            let mut end = t.len();
            for (k, tok) in t.iter().enumerate().skip(i) {
                match tok {
                    Tok::Function(_) | Tok::OpenParen => depth_b += 1,
                    Tok::CloseParen => {
                        depth_b -= 1;
                        if depth_b == 0 {
                            end = k;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let inner = &t[(i + 1).min(t.len())..end.min(t.len())];
            let comma = inner.iter().position(|x| *x == Tok::Comma);
            let name_part = values::trim(&inner[..comma.unwrap_or(inner.len())]);
            let fallback = comma.map(|c| &inner[c + 1..]);
            let value = match (f.as_str(), name_part) {
                ("var", [Tok::Ident(name)]) => lookup(name.as_str()).filter(|v| !v.is_empty() && !(v.len() == 1 && matches!(&v[0], Tok::Ident(i) if i == "initial"))),
                _ => None,
            };
            match (value, fallback) {
                (Some(v), _) => out.extend(substitute_with(&v, lookup, depth + 1)?),
                (None, Some(fb)) => out.extend(substitute_with(values::trim(fb), lookup, depth + 1)?),
                (None, None) => return None,
            }
            i = end + 1;
            continue;
        }
        out.push(t[i].clone());
        i += 1;
    }
    Some(out)
}

// ------------------------------------------------------------------------------------------------ matching

pub(crate) fn matches(doc: &Document, sib: &SiblingInfoRef, id: NodeId, sel: &Selector) -> bool {
    let n = sel.compounds.len();
    if n == 0 {
        return false;
    }
    match_from(doc, sib, id, sel, n - 1, 0)
}

type SiblingInfoRef = SiblingInfo;

/// Whether the compounds up to `k` match with the `k`th on `id`.
fn match_from(doc: &Document, sib: &SiblingInfo, id: NodeId, sel: &Selector, k: usize, depth: u32) -> bool {
    if depth > 64 || !compound(doc, sib, id, &sel.compounds[k]) {
        return false;
    }
    if k == 0 {
        return true;
    }
    match sel.combinators.get(k - 1) {
        Some(Combinator::Child) => up(doc, id).is_some_and(|p| match_from(doc, sib, p, sel, k - 1, depth + 1)),
        Some(Combinator::Descendant) | None => {
            let mut at = up(doc, id);
            while let Some(p) = at {
                if match_from(doc, sib, p, sel, k - 1, depth + 1) {
                    return true;
                }
                at = up(doc, p);
            }
            false
        }
        Some(Combinator::Next) => previous_element(doc, id).is_some_and(|p| match_from(doc, sib, p, sel, k - 1, depth + 1)),
        Some(Combinator::Later) => {
            let mut at = previous_element(doc, id);
            while let Some(p) = at {
                if match_from(doc, sib, p, sel, k - 1, depth + 1) {
                    return true;
                }
                at = previous_element(doc, p);
            }
            false
        }
    }
}

/// An element's parent element, for selectors: at the top of a shadow tree, its host (which only :host matches,
/// from the tree's own sheets).
fn up(doc: &Document, id: NodeId) -> Option<NodeId> {
    doc.parent_element(id).or_else(|| doc.parent(id).and_then(|p| doc.shadow_host(p)))
}

fn previous_element(doc: &Document, id: NodeId) -> Option<NodeId> {
    let p = doc.parent(id)?;
    let kids = doc.children(p);
    let at = kids.iter().position(|&k| k == id)?;
    kids[..at].iter().rev().copied().find(|&k| doc.element(k).is_some())
}

fn compound(doc: &Document, sib: &SiblingInfo, id: NodeId, c: &Compound) -> bool {
    let Some(el) = doc.element(id) else { return false };
    if let Some(t) = &c.tag
        && *t != el.name
    {
        return false;
    }
    for i in &c.ids {
        if el.id() != Some(i.as_str()) {
            return false;
        }
    }
    for cl in &c.classes {
        if !el.has_class(cl) {
            return false;
        }
    }
    for a in &c.attrs {
        let Some(v) = el.attr(&a.name) else { return false };
        let (v, want) = if a.ci { (v.to_ascii_lowercase(), a.value.to_ascii_lowercase()) } else { (String::from(v), a.value.clone()) };
        let ok = match a.op {
            AttrOp::Exists => true,
            AttrOp::Equals => v == want,
            AttrOp::Includes => v.split_ascii_whitespace().any(|w| w == want),
            AttrOp::Dash => v == want || v.starts_with(&alloc::format!("{want}-")),
            AttrOp::Prefix => !want.is_empty() && v.starts_with(&want),
            AttrOp::Suffix => !want.is_empty() && v.ends_with(&want),
            AttrOp::Substring => !want.is_empty() && v.contains(&want),
        };
        if !ok {
            return false;
        }
    }
    for p in &c.pseudos {
        if !pseudo(doc, sib, id, p) {
            return false;
        }
    }
    true
}

fn nth(a: i32, b: i32, n: i32) -> bool {
    if a == 0 {
        return n == b;
    }
    let d = n - b;
    d % a == 0 && d / a >= 0
}

fn pseudo(doc: &Document, sib: &SiblingInfo, id: NodeId, p: &Pseudo) -> bool {
    let Some(el) = doc.element(id) else { return false };
    let index = sib.index.get(id).copied().unwrap_or(1) as i32;
    let count = sib.count.get(id).copied().unwrap_or(1) as i32;
    match p {
        Pseudo::FirstChild => index == 1,
        Pseudo::LastChild => index == count,
        Pseudo::OnlyChild => count == 1,
        Pseudo::Nth(a, b, last) => nth(*a, *b, if *last { count - index + 1 } else { index }),
        Pseudo::NthOfType(a, b, last) => {
            let (all, at) = doc.element_siblings(id);
            let same: Vec<NodeId> = all.iter().copied().filter(|&s| doc.name(s) == el.name).collect();
            let pos = same.iter().position(|&s| s == id).unwrap_or(0) as i32;
            let _ = at;
            let n = if *last { same.len() as i32 - pos } else { pos + 1 };
            nth(*a, *b, n)
        }
        Pseudo::OnlyOfType => {
            let (all, _) = doc.element_siblings(id);
            all.iter().filter(|&&s| doc.name(s) == el.name).count() == 1
        }
        Pseudo::Not(list) => !list.iter().any(|s| matches(doc, sib, id, s)),
        Pseudo::Host(list) => doc.shadow_of(id).is_some() && (list.is_empty() || list.iter().any(|s| matches(doc, sib, id, s))),
        Pseudo::Slotted(list) => doc.parent(id).is_some_and(|p| doc.shadow_of(p).is_some()) && list.iter().any(|s| matches(doc, sib, id, s)),
        Pseudo::Is(list) => list.iter().any(|s| matches(doc, sib, id, s)),
        Pseudo::Has(list) => {
            // Anything inside it matching (the relative part's combinator aside), at most a few thousand looked at.
            doc.descendants(id).skip(1).take(4000).any(|d| doc.element(d).is_some() && list.iter().any(|s| {
                let last = Selector { compounds: s.compounds.last().cloned().into_iter().collect(), combinators: Vec::new(), specificity: 0, pseudo_element: None };
                matches(doc, sib, d, if s.compounds.len() > 1 && s.compounds[0].tag.is_none() && s.compounds[0].classes.is_empty() && s.compounds[0].ids.is_empty() { &last } else { s })
            }))
        }
        Pseudo::Root => doc.parent(id) == Some(0),
        Pseudo::Empty => doc.children(id).iter().all(|&k| match &doc.nodes[k].data {
            Data::Element(_) => false,
            Data::Text(t) => t.is_empty(),
            Data::Document | Data::Comment(_) | Data::Fragment => true,
        }),
        Pseudo::Link => matches!(el.name.as_str(), "a" | "area") && el.has_attr("href"),
        Pseudo::Checked => (el.name == "input" && el.has_attr("checked")) || (el.name == "option" && el.has_attr("selected")),
        Pseudo::Disabled => el.has_attr("disabled") && matches!(el.name.as_str(), "input" | "button" | "select" | "textarea" | "option" | "fieldset"),
        Pseudo::Enabled => !el.has_attr("disabled") && matches!(el.name.as_str(), "input" | "button" | "select" | "textarea" | "option"),
        Pseudo::Required => el.has_attr("required"),
        Pseudo::Optional => !el.has_attr("required") && matches!(el.name.as_str(), "input" | "select" | "textarea"),
        Pseudo::PlaceholderShown => el.has_attr("placeholder") && el.attr("value").unwrap_or("").is_empty(),
        Pseudo::PopoverOpen => el.has_attr("popover") && doc.popovers.contains(&id),
        Pseudo::Never => false,
    }
}

// ------------------------------------------------------------------------------------------------ properties

struct Ctx<'a> {
    parent: &'a Style,
    units: Units,
}

fn ident(v: &[Tok]) -> Option<String> {
    match values::trim(v) {
        [Tok::Ident(i)] => Some(i.to_ascii_lowercase()),
        _ => None,
    }
}

/// The value's parts, split at spaces (a function and what's in it are one part).
fn parts(v: &[Tok]) -> Vec<&[Tok]> {
    let mut out = Vec::new();
    let mut i = 0;
    let v = values::trim(v);
    while i < v.len() {
        if v[i] == Tok::Ws {
            i += 1;
            continue;
        }
        let start = i;
        if matches!(v[i], Tok::Function(_) | Tok::OpenParen | Tok::OpenSquare) {
            let mut depth = 0;
            while i < v.len() {
                match v[i] {
                    Tok::Function(_) | Tok::OpenParen | Tok::OpenSquare => depth += 1,
                    Tok::CloseParen | Tok::CloseSquare => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            i += 1;
        } else {
            i += 1;
        }
        out.push(&v[start..i.min(v.len())]);
    }
    out
}

fn size_value(v: &[Tok], c: &Ctx, base: f32) -> Option<Size> {
    if let Some(i) = ident(v) {
        return Some(match i.as_str() {
            "auto" => Size::Auto,
            "none" => Size::None,
            "min-content" => Size::MinContent,
            "max-content" => Size::MaxContent,
            "fit-content" | "-webkit-fit-content" | "-moz-fit-content" | "stretch" | "-webkit-fill-available" | "-moz-available" => {
                if i.contains("fill") || i == "stretch" || i.contains("available") { Size::Auto } else { Size::FitContent }
            }
            _ => return None,
        });
    }
    if let [Tok::Function(f), ..] = values::trim(v)
        && f == "fit-content"
    {
        return Some(Size::FitContent);
    }
    values::length(v, &c.units, base).map(Size::Len)
}

/// One, two, three or four values for the four sides (top, right, bottom, left).
fn four<T: Copy>(v: &[Tok], f: impl Fn(&[Tok]) -> Option<T>) -> Option<[T; 4]> {
    let p = parts(v);
    let vals: Vec<T> = p.iter().map(|x| f(x)).collect::<Option<_>>()?;
    Some(match vals.len() {
        1 => [vals[0]; 4],
        2 => [vals[0], vals[1], vals[0], vals[1]],
        3 => [vals[0], vals[1], vals[2], vals[1]],
        4 => [vals[0], vals[1], vals[2], vals[3]],
        _ => return None,
    })
}

fn border_style(i: &str) -> Option<BorderStyle> {
    Some(match i {
        "none" | "hidden" => BorderStyle::None,
        "solid" => BorderStyle::Solid,
        "dashed" => BorderStyle::Dashed,
        "dotted" => BorderStyle::Dotted,
        "double" => BorderStyle::Double,
        "inset" => BorderStyle::Inset,
        "outset" => BorderStyle::Outset,
        "groove" => BorderStyle::Groove,
        "ridge" => BorderStyle::Ridge,
        _ => return None,
    })
}

fn border_width(v: &[Tok], c: &Ctx) -> Option<f32> {
    match ident(v).as_deref() {
        Some("thin") => Some(1.0 * c.units.dpr),
        Some("medium") => Some(3.0 * c.units.dpr),
        Some("thick") => Some(5.0 * c.units.dpr),
        _ => values::length(v, &c.units, 0.0).map(|l| l.px.max(0.0)),
    }
}

/// "1px solid red" in any order: (width, style, colour).
fn border_parts(v: &[Tok], c: &Ctx, current: Color) -> Option<(Option<f32>, Option<BorderStyle>, Option<Color>)> {
    let (mut w, mut st, mut col) = (None, None, None);
    for p in parts(v) {
        if let Some(i) = ident(p)
            && let Some(s) = border_style(&i)
        {
            st = Some(s);
        } else if let Some(x) = border_width(p, c) {
            w = Some(x);
        } else if let Some(x) = values::color(p, current) {
            col = Some(x);
        } else {
            return None;
        }
    }
    Some((w, st, col))
}

fn align(i: &str) -> Option<Align> {
    Some(match i {
        "normal" | "auto" => Align::Normal,
        "start" | "flex-start" | "self-start" | "left" | "top" => Align::Start,
        "end" | "flex-end" | "self-end" | "right" | "bottom" => Align::End,
        "center" => Align::Center,
        "stretch" => Align::Stretch,
        "baseline" | "first" | "last" => Align::Baseline,
        "space-between" => Align::SpaceBetween,
        "space-around" => Align::SpaceAround,
        "space-evenly" => Align::SpaceEvenly,
        "safe" | "unsafe" => return None,
        _ => return None,
    })
}

fn align_value(v: &[Tok]) -> Option<Align> {
    // ("safe center", "first baseline": the last word that means something.)
    parts(v).iter().filter_map(|p| ident(p)).filter_map(|i| align(&i)).last()
}

fn display_value(i: &str) -> Option<Display> {
    Some(match i {
        "none" => Display::None,
        "contents" => Display::Contents,
        "block" | "block flow" => Display::Block,
        "flow-root" | "block flow-root" => Display::FlowRoot,
        "inline" | "inline flow" | "ruby" | "ruby-text" | "ruby-base" => Display::Inline,
        "inline-block" | "inline flow-root" => Display::InlineBlock,
        "list-item" | "block list-item" | "list-item block" => Display::ListItem,
        "flex" | "block flex" | "-webkit-box" | "-webkit-flex" | "-ms-flexbox" => Display::Flex,
        "inline-flex" | "inline flex" | "-webkit-inline-box" | "-webkit-inline-flex" => Display::InlineFlex,
        "grid" | "block grid" => Display::Grid,
        "inline-grid" | "inline grid" => Display::InlineGrid,
        "table" | "block table" => Display::Table,
        "inline-table" | "inline table" => Display::InlineTable,
        "table-row" => Display::TableRow,
        "table-cell" => Display::TableCell,
        "table-row-group" => Display::TableRowGroup,
        "table-header-group" => Display::TableHeaderGroup,
        "table-footer-group" => Display::TableFooterGroup,
        "table-column" => Display::TableColumn,
        "table-column-group" => Display::TableColumnGroup,
        "table-caption" => Display::TableCaption,
        _ => return None,
    })
}

fn list_style(i: &str) -> Option<ListStyle> {
    Some(match i {
        "none" => ListStyle::None,
        "disc" => ListStyle::Disc,
        "circle" => ListStyle::Circle,
        "square" => ListStyle::Square,
        "decimal" | "decimal-leading-zero" | "arabic-indic" => ListStyle::Decimal,
        "lower-alpha" | "lower-latin" => ListStyle::LowerAlpha,
        "upper-alpha" | "upper-latin" => ListStyle::UpperAlpha,
        "lower-roman" => ListStyle::LowerRoman,
        "upper-roman" => ListStyle::UpperRoman,
        _ => return None,
    })
}

fn font_size(v: &[Tok], c: &Ctx) -> Option<f32> {
    let parent = c.parent.font_size;
    let dpr = c.units.dpr;
    if let Some(i) = ident(v) {
        return Some(match i.as_str() {
            "xx-small" => 9.0 * dpr,
            "x-small" => 10.0 * dpr,
            "small" => 13.0 * dpr,
            "medium" => 16.0 * dpr,
            "large" => 18.0 * dpr,
            "x-large" => 24.0 * dpr,
            "xx-large" => 32.0 * dpr,
            "xxx-large" => 48.0 * dpr,
            "smaller" => parent / 1.2,
            "larger" => parent * 1.2,
            _ => return None,
        });
    }
    // em and % are of the parent's size here.
    let u = Units { em: parent, ..c.units };
    let l = values::length(v, &u, 0.0)?;
    Some((l.px + l.pct * parent / 100.0).max(0.0))
}

/// How many times "medium" a font size is, when it comes from a keyword or is em or % of one (0 when it doesn't).
fn keyword_size(v: &[Tok], c: &Ctx) -> f32 {
    let parent = c.parent.keyword_size;
    if let Some(i) = ident(v) {
        return match i.as_str() {
            "xx-small" => 9.0 / 16.0,
            "x-small" => 10.0 / 16.0,
            "small" => 13.0 / 16.0,
            "medium" => 1.0,
            "large" => 18.0 / 16.0,
            "x-large" => 1.5,
            "xx-large" => 2.0,
            "xxx-large" => 3.0,
            "smaller" => parent / 1.2,
            "larger" => parent * 1.2,
            _ => 0.0,
        };
    }
    match values::trim(v) {
        [Tok::Dimension(n, u)] if u.eq_ignore_ascii_case("em") => parent * n,
        [Tok::Percent(n)] => parent * n / 100.0,
        _ => 0.0,
    }
}

/// A box shadow: how far it's moved (x, y), its blur radius and spread (device pixels), its colour, and whether it's
/// cast inside the box (inset).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub x: f32,
    pub y: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: Color,
    pub inset: bool,
}

/// box-shadow's shadows: Some(None) for none, None if it isn't a valid value.
fn box_shadow(v: &[Tok], units: &Units, current: Color) -> Option<Option<Arc<Vec<Shadow>>>> {
    if ident(v).as_deref() == Some("none") {
        return Some(None);
    }
    let mut out = Vec::new();
    for one in css::split_commas(v) {
        let mut lengths: Vec<f32> = Vec::new();
        let mut color = None;
        let mut inset = false;
        for t in parts(one) {
            if ident(t).as_deref() == Some("inset") {
                inset = true;
            } else if let Some(l) = values::length(t, units, 0.0) {
                lengths.push(l.px);
            } else if let Some(c) = values::color(t, current) {
                color = Some(c);
            } else {
                return None;
            }
        }
        if lengths.len() < 2 || lengths.len() > 4 {
            return None;
        }
        let at = |i: usize| lengths.get(i).copied().unwrap_or(0.0);
        out.push(Shadow { x: at(0), y: at(1), blur: at(2).max(0.0), spread: at(3), color: color.unwrap_or(current), inset });
    }
    Some(Some(Arc::new(out)))
}

/// Whether a font-family is the generic monospace family alone.
fn generic_mono(v: &[Tok]) -> bool {
    matches!(values::trim(v), [Tok::Ident(i)] if i.eq_ignore_ascii_case("monospace"))
}

fn font_weight(v: &[Tok], parent: u16) -> Option<u16> {
    if let Some(i) = ident(v) {
        return Some(match i.as_str() {
            "normal" => 400,
            "bold" => 700,
            "bolder" => {
                if parent < 350 {
                    400
                } else if parent < 550 {
                    700
                } else {
                    900
                }
            }
            "lighter" => {
                if parent < 550 {
                    100
                } else if parent < 750 {
                    400
                } else {
                    700
                }
            }
            _ => return None,
        });
    }
    match values::trim(v) {
        [Tok::Number(n)] => Some(n.clamp(1.0, 1000.0) as u16),
        _ => None,
    }
}

/// Families by kind, by a word in their names: monospace, sans-serif and serif ones.
const MONO_WORDS: &[&str] = &["mono", "courier", "consolas", "menlo", "monaco", "code", "fixed", "terminal"];
const SANS_WORDS: &[&str] = &[
    "sans", "system-ui", "-apple-system", "blinkmacsystemfont", "segoe", "roboto", "helvetica", "arial", "inter",
    "lato", "verdana", "tahoma", "trebuchet", "ubuntu", "cantarell", "montserrat", "poppins", "nunito", "raleway",
    "avenir", "gill", "lucida", "calibri", "oxygen", "geist", "futura", "franklin", "proxima", "manrope", "figtree",
    "karla", "rubik", "barlow", "mulish", "quicksand", "graphik", "circular", "hiragino", "meiryo", "gothic", "yahei",
    "pingfang", "malgun", "atkinson",
];
const SERIF_WORDS: &[&str] = &[
    "serif", "times", "georgia", "garamond", "palatino", "baskerville", "cambria", "merriweather", "lora", "playfair",
    "book", "charter", "iowan", "crimson", "caslon", "didot", "bodoni", "minion", "constantia", "spectral",
    "literata", "new york", "tiempos", "mincho", "songti",
];

/// The kind of font for what `family_kind` found: monospace, sans-serif, or (else) serif.
pub fn family_of(mono: bool, sans: bool) -> text::Family {
    if mono {
        text::Family::Mono
    } else if sans {
        text::Family::Sans
    } else {
        text::Family::Serif
    }
}

/// What a font-family list written out asks for (a canvas's font): (monospace, sans-serif), as `family_kind`.
pub fn kind_of_families(list: &str) -> (bool, bool) {
    for f in list.split(',') {
        let f = f.trim().trim_matches(['"', '\'']).to_ascii_lowercase();
        if MONO_WORDS.iter().any(|w| f.contains(w)) {
            return (true, false);
        }
        if SANS_WORDS.iter().any(|w| f.contains(w)) {
            return (false, true);
        }
        if SERIF_WORDS.iter().any(|w| f.contains(w)) {
            return (false, false);
        }
    }
    (false, false)
}

/// What a font-family list asks for: (monospace, sans-serif); neither is serif. Its first family of a kind this
/// browser can tell (the others are fonts it hasn't, which the list's later ones stand in for).
fn family_kind(v: &[Tok]) -> (bool, bool) {
    let mut families: Vec<String> = Vec::new();
    let mut name = String::new();
    for t in v {
        match t {
            Tok::Ident(i) | Tok::Str(i) => {
                if !name.is_empty() {
                    name.push(' ');
                }
                name.push_str(&i.to_ascii_lowercase());
            }
            Tok::Comma => families.push(core::mem::take(&mut name)),
            _ => {}
        }
    }
    families.push(name);
    for f in &families {
        if MONO_WORDS.iter().any(|w| f.contains(w)) {
            return (true, false);
        }
        if SANS_WORDS.iter().any(|w| f.contains(w)) {
            return (false, true);
        }
        if SERIF_WORDS.iter().any(|w| f.contains(w)) {
            return (false, false);
        }
    }
    (false, false)
}

fn line_height(v: &[Tok], c: &Ctx, size: f32) -> Option<LineHeight> {
    if ident(v).as_deref() == Some("normal") {
        return Some(LineHeight::Normal);
    }
    if let Some(n) = values::number(v, &c.units) {
        return Some(LineHeight::Factor(n.max(0.0)));
    }
    let u = Units { em: size, ..c.units };
    let l = values::length(v, &u, 0.0)?;
    Some(LineHeight::Px((l.px + l.pct * size / 100.0).max(0.0)))
}

/// A background shorthand not read whole: the last layer's colour and the first picture or gradient, as best they can
/// be found.
fn background_layers(v: &[Tok], s: &mut Style, current: Color, units: &Units) {
    let layers = css::split_commas(v);
    let mut image = None;
    for (n, layer) in layers.iter().enumerate() {
        for p in parts(layer) {
            if image.is_none()
                && let Some(Some(i)) = effects::image(p, current, units)
            {
                image = Some(i);
                continue;
            }
            if n == layers.len() - 1
                && let Some(c) = values::color(p, current)
            {
                s.background = c;
            }
        }
    }
    s.bg = image.map(|i| Arc::new(Layers { images: alloc::vec![Some(i)], ..Default::default() }));
}

fn grid_place(v: &[Tok]) -> GridPlace {
    // "span 2", "1 / 3", "1 / -1", "2", "auto", "1 / span 2".
    let halves = {
        let mut out: Vec<Vec<Tok>> = vec![Vec::new()];
        for t in v {
            if *t == Tok::Delim('/') {
                out.push(Vec::new());
            } else if let Some(last) = out.last_mut() {
                last.push(t.clone());
            }
        }
        out
    };
    let read = |t: &[Tok]| -> (i32, u32) {
        let mut line = 0;
        let mut span = 0;
        let mut spanning = false;
        for p in parts(t) {
            match values::trim(p) {
                [Tok::Ident(i)] if i.eq_ignore_ascii_case("span") => spanning = true,
                [Tok::Number(n)] => {
                    if spanning {
                        span = (*n as i32).max(1) as u32;
                    } else {
                        line = *n as i32;
                    }
                }
                _ => {}
            }
        }
        if spanning && span == 0 {
            span = 1;
        }
        (line, span)
    };
    let (start, s1) = read(&halves[0]);
    let (end, s2) = halves.get(1).map(|h| read(h)).unwrap_or((0, 0));
    let span = if s1 > 0 { s1 } else if s2 > 0 { s2 } else if start != 0 && end != 0 && end > start { (end - start) as u32 } else { 0 };
    GridPlace { start, end, span }
}

fn tracks(v: &[Tok], c: &Ctx) -> Vec<Track> {
    let mut out = Vec::new();
    for p in parts(v) {
        match values::trim(p) {
            [Tok::OpenSquare, ..] => {} // a line's name
            [Tok::Function(f), rest @ ..] if f == "repeat" => {
                let inner = &rest[..rest.len().saturating_sub(1)];
                let args = css::split_commas(inner);
                if args.len() < 2 {
                    continue;
                }
                let what = tracks(args[1], c);
                match values::trim(args[0]) {
                    [Tok::Number(n)] => {
                        for _ in 0..(*n as usize).min(100) {
                            out.extend(what.iter().cloned());
                        }
                    }
                    _ => out.push(Track::AutoRepeat(what)),
                }
            }
            [Tok::Function(f), rest @ ..] if f == "minmax" => {
                let inner = &rest[..rest.len().saturating_sub(1)];
                let args = css::split_commas(inner);
                if args.len() == 2 {
                    let a = tracks(args[0], c).into_iter().next().unwrap_or(Track::Auto);
                    let b = tracks(args[1], c).into_iter().next().unwrap_or(Track::Auto);
                    out.push(Track::MinMax(alloc::boxed::Box::new((a, b))));
                }
            }
            [Tok::Function(f), ..] if f == "fit-content" => out.push(Track::Auto),
            [Tok::Dimension(n, u)] if u == "fr" => out.push(Track::Fr(*n)),
            [Tok::Ident(i)] => out.push(match i.to_ascii_lowercase().as_str() {
                "min-content" => Track::MinContent,
                "max-content" => Track::MaxContent,
                _ => Track::Auto,
            }),
            x => {
                if let Some(l) = values::length(x, &c.units, 0.0) {
                    out.push(Track::Len(l));
                }
            }
        }
    }
    out
}

fn content(v: &[Tok], el_attr: impl Fn(&str) -> Option<String>) -> Option<String> {
    let mut out = String::new();
    let mut any = false;
    for p in parts(v) {
        match values::trim(p) {
            [Tok::Str(s)] => {
                out.push_str(s);
                any = true;
            }
            [Tok::Ident(i)] => match i.to_ascii_lowercase().as_str() {
                "none" | "normal" => return None,
                "open-quote" => {
                    out.push('“');
                    any = true;
                }
                "close-quote" => {
                    out.push('”');
                    any = true;
                }
                _ => {}
            },
            [Tok::Function(f), Tok::Ident(a), Tok::CloseParen] if f == "attr" => {
                out.push_str(&el_attr(a).unwrap_or_default());
                any = true;
            }
            [Tok::Function(_), ..] => any = true,
            [Tok::Delim('/'), ..] => break,
            _ => {}
        }
    }
    any.then_some(out)
}

/// One declaration on the style being worked out.
fn apply_one(s: &mut Style, name: &str, v: &[Tok], c: &Ctx) {
    let p = c.parent;
    // The keywords every property takes.
    if let Some(kw) = ident(v) {
        match kw.as_str() {
            "inherit" => {
                inherit_one(s, p, name);
                return;
            }
            "initial" | "unset" | "revert" | "revert-layer" => {
                let init = Style::initial(c.units.dpr);
                let inherited = matches!(name, "color" | "font-size" | "font-weight" | "font-style" | "line-height" | "text-align" | "white-space" | "visibility" | "list-style-type" | "text-transform" | "font-family" | "font" | "fill" | "stroke" | "image-rendering");
                if kw == "initial" || !inherited {
                    inherit_one(s, &init, name);
                } else {
                    inherit_one(s, p, name);
                }
                return;
            }
            _ => {}
        }
    }
    let cur = s.color;
    let units = &c.units;
    match name {
        "display" => {
            let words: Vec<String> = parts(v).iter().filter_map(|x| ident(x)).collect();
            if let Some(d) = display_value(&words.join(" ")) {
                s.display = d;
                s.webkit_box = matches!(words.join(" ").as_str(), "-webkit-box" | "-webkit-inline-box");
            }
        }
        "text-overflow" => s.ellipsis = parts(v).iter().filter_map(|x| ident(x)).last().as_deref() == Some("ellipsis"),
        "image-rendering" => {
            if let Some(w) = ident(v) {
                s.pixelated = matches!(w.as_str(), "pixelated" | "crisp-edges" | "-moz-crisp-edges" | "-webkit-optimize-contrast" | "optimizespeed" | "optimize-contrast");
            }
        }
        "animation" | "-webkit-animation" => {
            if let Some(a) = animation_shorthand(v) {
                s.animations = if a.names.iter().all(|n| n == "none") { None } else { Some(Arc::new(a)) };
            }
        }
        "animation-name" | "-webkit-animation-name" | "animation-iteration-count" | "-webkit-animation-iteration-count" | "animation-direction" | "-webkit-animation-direction" | "animation-fill-mode" | "-webkit-animation-fill-mode" => {
            let mut a = s.animations.as_deref().cloned().unwrap_or_default();
            let items = css::split_commas(v);
            let word = |t: &[Tok]| ident(t).unwrap_or_default();
            match name.trim_start_matches("-webkit-") {
                "animation-name" => {
                    a.names = items
                        .iter()
                        .map(|t| match values::trim(t) {
                            [Tok::Str(x)] => x.clone(),
                            [Tok::Ident(x)] => x.clone(),
                            _ => String::from("none"),
                        })
                        .collect()
                }
                "animation-iteration-count" => {
                    a.counts = items
                        .iter()
                        .map(|t| match values::trim(t) {
                            [Tok::Number(n)] => n.max(0.0),
                            _ if word(t) == "infinite" => f32::INFINITY,
                            _ => 1.0,
                        })
                        .collect()
                }
                "animation-direction" => {
                    a.directions = items
                        .iter()
                        .map(|t| match word(t).as_str() {
                            "reverse" => 1,
                            "alternate" => 2,
                            "alternate-reverse" => 3,
                            _ => 0,
                        })
                        .collect()
                }
                _ => {
                    a.fills = items
                        .iter()
                        .map(|t| match word(t).as_str() {
                            "forwards" => 1,
                            "backwards" => 2,
                            "both" => 3,
                            _ => 0,
                        })
                        .collect()
                }
            }
            s.animations = if a.names.is_empty() || a.names.iter().all(|n| n == "none") { None } else { Some(Arc::new(a)) };
        }
        "-webkit-line-clamp" | "line-clamp" => {
            s.line_clamp = match values::trim(v) {
                [Tok::Number(n)] if *n >= 1.0 => *n as u32,
                _ => 0,
            };
        }
        "position" => {
            if let Some(i) = ident(v) {
                s.position = match i.as_str() {
                    "relative" => Position::Relative,
                    "absolute" => Position::Absolute,
                    "fixed" => Position::Fixed,
                    "sticky" | "-webkit-sticky" => Position::Sticky,
                    _ => Position::Static,
                };
            }
        }
        "float" => {
            if let Some(i) = ident(v) {
                s.float = match i.as_str() {
                    "left" | "inline-start" => Float::Left,
                    "right" | "inline-end" => Float::Right,
                    _ => Float::None,
                };
            }
        }
        "clear" => {
            if let Some(i) = ident(v) {
                s.clear_both = i == "both";
                s.clear = match i.as_str() {
                    "left" | "inline-start" => Float::Left,
                    "right" | "inline-end" => Float::Right,
                    _ => Float::None,
                };
            }
        }
        "top" | "right" | "bottom" | "left" | "inset-block-start" | "inset-inline-end" | "inset-block-end" | "inset-inline-start" => {
            if let Some(x) = size_value(v, c, 0.0) {
                let k = match name {
                    "top" | "inset-block-start" => 0,
                    "right" | "inset-inline-end" => 1,
                    "bottom" | "inset-block-end" => 2,
                    _ => 3,
                };
                s.inset[k] = x;
            }
        }
        "inset" => {
            if let Some(x) = four(v, |t| size_value(t, c, 0.0)) {
                s.inset = x;
            }
        }
        "inset-inline" | "inset-block" => {
            let ps = parts(v);
            if let Some(a) = ps.first().and_then(|t| size_value(t, c, 0.0)) {
                let b = ps.get(1).and_then(|t| size_value(t, c, 0.0)).unwrap_or(a);
                if name == "inset-inline" {
                    s.inset[3] = a;
                    s.inset[1] = b;
                } else {
                    s.inset[0] = a;
                    s.inset[2] = b;
                }
            }
        }
        "width" | "inline-size" => {
            if let Some(x) = size_value(v, c, 0.0) {
                s.width = x;
            }
        }
        "height" | "block-size" => {
            if let Some(x) = size_value(v, c, 0.0) {
                s.height = x;
            }
        }
        "min-width" | "min-inline-size" => {
            if let Some(x) = size_value(v, c, 0.0) {
                s.min_width = x;
            }
        }
        "min-height" | "min-block-size" => {
            if let Some(x) = size_value(v, c, 0.0) {
                s.min_height = x;
            }
        }
        "max-width" | "max-inline-size" => {
            if let Some(x) = size_value(v, c, 0.0) {
                s.max_width = x;
            }
        }
        "max-height" | "max-block-size" => {
            if let Some(x) = size_value(v, c, 0.0) {
                s.max_height = x;
            }
        }
        "margin" => {
            if let Some(x) = four(v, |t| size_value(t, c, 0.0)) {
                s.margin = x;
            }
        }
        "margin-top" | "margin-right" | "margin-bottom" | "margin-left" | "margin-block-start" | "margin-inline-end"
        | "margin-block-end" | "margin-inline-start" => {
            if let Some(x) = size_value(v, c, 0.0) {
                s.margin[side(name)] = x;
            }
        }
        "margin-inline" | "margin-block" => {
            let ps = parts(v);
            if let Some(a) = ps.first().and_then(|t| size_value(t, c, 0.0)) {
                let b = ps.get(1).and_then(|t| size_value(t, c, 0.0)).unwrap_or(a);
                if name == "margin-inline" {
                    s.margin[3] = a;
                    s.margin[1] = b;
                } else {
                    s.margin[0] = a;
                    s.margin[2] = b;
                }
            }
        }
        "padding" => {
            if let Some(x) = four(v, |t| values::length(t, units, 0.0)) {
                s.padding = x.map(|l| Len { px: l.px.max(0.0), pct: l.pct.max(0.0) });
            }
        }
        "padding-top" | "padding-right" | "padding-bottom" | "padding-left" | "padding-block-start"
        | "padding-inline-end" | "padding-block-end" | "padding-inline-start" => {
            if let Some(x) = values::length(v, units, 0.0) {
                s.padding[side(name)] = Len { px: x.px.max(0.0), pct: x.pct.max(0.0) };
            }
        }
        "padding-inline" | "padding-block" => {
            let ps = parts(v);
            if let Some(a) = ps.first().and_then(|t| values::length(t, units, 0.0)) {
                let b = ps.get(1).and_then(|t| values::length(t, units, 0.0)).unwrap_or(a);
                if name == "padding-inline" {
                    s.padding[3] = a;
                    s.padding[1] = b;
                } else {
                    s.padding[0] = a;
                    s.padding[2] = b;
                }
            }
        }
        "border" | "border-top" | "border-right" | "border-bottom" | "border-left" | "border-block-start"
        | "border-inline-end" | "border-block-end" | "border-inline-start" | "border-inline" | "border-block" => {
            if let Some((w, st, col)) = border_parts(v, c, cur) {
                let sides: &[usize] = match name {
                    "border" => &[0, 1, 2, 3],
                    "border-inline" => &[1, 3],
                    "border-block" => &[0, 2],
                    n => match side(n) {
                        0 => &[0],
                        1 => &[1],
                        2 => &[2],
                        _ => &[3],
                    },
                };
                for &k in sides {
                    s.border_width[k] = w.unwrap_or(3.0 * c.units.dpr);
                    s.border_style[k] = st.unwrap_or(BorderStyle::None);
                    s.border_color[k] = col.unwrap_or(cur);
                }
            }
        }
        "border-width" => {
            if let Some(x) = four(v, |t| border_width(t, c)) {
                s.border_width = x;
            }
        }
        "border-style" => {
            if let Some(x) = four(v, |t| ident(t).and_then(|i| border_style(&i))) {
                s.border_style = x;
            }
        }
        "border-color" => {
            if let Some(x) = four(v, |t| values::color(t, cur)) {
                s.border_color = x;
            }
        }
        "border-top-width" | "border-right-width" | "border-bottom-width" | "border-left-width" | "border-block-start-width"
        | "border-inline-end-width" | "border-block-end-width" | "border-inline-start-width" => {
            if let Some(x) = border_width(v, c) {
                s.border_width[side(name)] = x;
            }
        }
        "border-top-style" | "border-right-style" | "border-bottom-style" | "border-left-style" => {
            if let Some(x) = ident(v).and_then(|i| border_style(&i)) {
                s.border_style[side(name)] = x;
            }
        }
        "border-top-color" | "border-right-color" | "border-bottom-color" | "border-left-color" => {
            if let Some(x) = values::color(v, cur) {
                s.border_color[side(name)] = x;
            }
        }
        "border-radius" | "-webkit-border-radius" => {
            // Up to four: the corners' (horizontal) radii, from the top left round; what's after a "/" (their vertical
            // ones, for ellipses) is left out.
            let mut found: Vec<Len> = Vec::new();
            for t in parts(v) {
                if values::trim(t).first() == Some(&Tok::Delim('/')) {
                    break;
                }
                if let Some(l) = values::length(t, units, 0.0) {
                    found.push(l);
                }
            }
            if !found.is_empty() {
                let g = |i: usize| found[i.min(found.len() - 1)];
                s.radius = match found.len() {
                    1 => [g(0); 4],
                    2 => [g(0), g(1), g(0), g(1)],
                    3 => [g(0), g(1), g(2), g(1)],
                    _ => [g(0), g(1), g(2), g(3)],
                };
            }
        }
        "border-top-left-radius" | "border-top-right-radius" | "border-bottom-right-radius" | "border-bottom-left-radius"
        | "border-start-start-radius" | "border-start-end-radius" | "border-end-end-radius" | "border-end-start-radius" => {
            let corner = match name {
                "border-top-left-radius" | "border-start-start-radius" => 0,
                "border-top-right-radius" | "border-start-end-radius" => 1,
                "border-bottom-right-radius" | "border-end-end-radius" => 2,
                _ => 3,
            };
            if let Some(l) = parts(v).first().and_then(|first| values::length(first, units, 0.0)) {
                s.radius[corner] = l;
            }
        }
        "box-shadow" | "-webkit-box-shadow" => {
            if let Some(sh) = box_shadow(v, units, s.color) {
                s.shadows = sh;
            }
        }
        "box-sizing" => s.border_box = ident(v).as_deref() == Some("border-box"),
        "fill" | "stroke" => {
            let paint = match ident(v).as_deref() {
                Some("none") => Some(SvgPaint::None),
                Some("currentcolor") => Some(SvgPaint::Current),
                _ => values::color(v, cur).map(SvgPaint::Color),
            };
            if paint.is_some() {
                if name == "fill" {
                    s.fill = paint;
                } else {
                    s.stroke = paint;
                }
            }
        }
        "color" => {
            if let Some(x) = values::color(v, p.color) {
                s.color = x;
            }
        }
        "background-color" => {
            if let Some(x) = values::color(v, cur) {
                s.background = x;
            }
        }
        "background" => match effects::layers_shorthand(v, cur, units, false) {
            Some((l, colour)) => {
                s.background = colour.unwrap_or(values::TRANSPARENT);
                s.bg = if l.any() { Some(Arc::new(l)) } else { None };
            }
            None => {
                // (One not read whole: its colour and first picture, as best they can be found.)
                s.background = values::TRANSPARENT;
                background_layers(v, s, cur, units);
            }
        },
        "background-image" | "background-position" | "background-position-x" | "background-position-y" | "background-size" | "-webkit-background-size" | "background-repeat" | "background-origin" | "-webkit-background-origin" | "background-clip" | "-webkit-background-clip" | "background-attachment" => {
            let prop = layer_prop(name.trim_start_matches("-webkit-").trim_start_matches("background-"));
            let mut l = s.bg.as_deref().cloned().unwrap_or_default();
            if effects::set_layer_prop(&mut l, prop, v, cur, units) {
                s.bg = Some(Arc::new(l));
            }
        }
        "mask" | "-webkit-mask" => {
            if let Some((l, _)) = effects::layers_shorthand(v, cur, units, true) {
                s.mask = if l.any() { Some(Arc::new(l)) } else { None };
            }
        }
        "mask-image" | "-webkit-mask-image" | "mask-position" | "-webkit-mask-position" | "mask-position-x" | "-webkit-mask-position-x" | "mask-position-y" | "-webkit-mask-position-y" | "mask-size" | "-webkit-mask-size" | "mask-repeat" | "-webkit-mask-repeat" | "mask-origin" | "-webkit-mask-origin" | "mask-clip" | "-webkit-mask-clip" | "mask-mode" | "mask-composite" | "-webkit-mask-composite" => {
            let prop = layer_prop(name.trim_start_matches("-webkit-").trim_start_matches("mask-"));
            let mut l = s.mask.as_deref().cloned().unwrap_or_default();
            if effects::set_layer_prop(&mut l, prop, v, cur, units) {
                s.mask = Some(Arc::new(l));
            }
        }
        "filter" | "-webkit-filter" => {
            if let Some(f) = effects::filters(v, cur, units) {
                s.filter = if f.is_empty() { None } else { Some(Arc::new(f)) };
            }
        }
        "backdrop-filter" | "-webkit-backdrop-filter" => {
            if let Some(f) = effects::filters(v, cur, units) {
                s.backdrop = if f.is_empty() { None } else { Some(Arc::new(f)) };
            }
        }
        "mix-blend-mode" => {
            if let Some(b) = ident(v).and_then(|w| effects::blend_mode(&w)) {
                s.blend = b;
            }
        }
        "isolation" => s.isolate = ident(v).as_deref() == Some("isolate"),
        "contain" => {
            let words: Vec<String> = parts(v).iter().filter_map(|x| ident(x)).collect();
            s.contain_paint = words.iter().any(|w| matches!(w.as_str(), "paint" | "strict" | "content"));
        }
        "will-change" => {
            let words: Vec<String> = css::split_commas(v).iter().filter_map(|x| ident(x)).collect();
            s.will_transform = words.iter().any(|w| matches!(w.as_str(), "transform" | "translate" | "rotate" | "scale" | "perspective"));
            s.will_change = s.will_transform || words.iter().any(|w| matches!(w.as_str(), "opacity" | "filter" | "backdrop-filter" | "mask" | "clip-path" | "mix-blend-mode" | "isolation"));
        }
        "object-position" => {
            if let Some(p) = effects::position(&parts(v), units) {
                s.object_position = p;
            }
        }
        "transform" | "-webkit-transform" => {
            if let Some(f) = effects::transform_list(v, units) {
                s.transform = if f.is_empty() { None } else { Some(Arc::new(f)) };
            }
        }
        "translate" | "rotate" | "scale" => {
            let t = match name {
                "translate" => effects::translate_prop(v, units),
                "rotate" => effects::rotate_prop(v, units),
                _ => effects::scale_prop(v, units),
            };
            if let Some(t) = t {
                s.transform_parts[match name {
                    "translate" => 0,
                    "rotate" => 1,
                    _ => 2,
                }] = t;
            }
        }
        "transform-origin" | "-webkit-transform-origin" => {
            if let Some(o) = effects::origin(v, units) {
                s.origin = o;
            }
        }
        "backface-visibility" | "-webkit-backface-visibility" => s.backface_hidden = ident(v).as_deref() == Some("hidden"),
        "font-size" => {
            if let Some(x) = font_size(v, c) {
                s.font_size = x;
                s.keyword_size = keyword_size(v, c);
            }
        }
        "font-weight" => {
            if let Some(x) = font_weight(v, p.font_weight) {
                s.font_weight = x;
            }
        }
        "font-style" => {
            if let Some(i) = parts(v).first().and_then(|t| ident(t)) {
                s.italic = i == "italic" || i == "oblique";
            }
        }
        "font-stretch" | "font-width" => {
            let w = match values::trim(v) {
                [Tok::Percent(p)] if *p >= 0.0 => Some(crate::num::round(*p).min(1000.0) as u16),
                _ => ident(v).and_then(|w| css::stretch_value(&w)),
            };
            if let Some(w) = w {
                s.font_stretch = w;
            }
        }
        "font-family" => {
            (s.mono, s.sans) = family_kind(v);
            s.generic_mono = generic_mono(v);
            s.families = Arc::new(family_list(v));
        }
        "font" => {
            // [style] [variant] [weight] [stretch] size[/line-height] family
            let ps = parts(v);
            let mut k = 0;
            s.italic = false;
            s.font_weight = 400;
            s.font_stretch = 100;
            s.line_height = LineHeight::Normal;
            if let Some(i) = ident(v) {
                // A system font ("caption", "menu"...).
                if matches!(i.as_str(), "caption" | "icon" | "menu" | "message-box" | "small-caption" | "status-bar") {
                    s.font_size = 13.333 * units.dpr;
                }
                return;
            }
            while k < ps.len() {
                let t = ps[k];
                if let Some(i) = ident(t) {
                    match i.as_str() {
                        "italic" | "oblique" => s.italic = true,
                        "bold" | "bolder" | "lighter" => s.font_weight = font_weight(t, p.font_weight).unwrap_or(700),
                        "condensed" | "expanded" | "semi-condensed" | "semi-expanded" | "ultra-condensed" | "extra-condensed" | "extra-expanded" | "ultra-expanded" => {
                            s.font_stretch = css::stretch_value(&i).unwrap_or(100);
                        }
                        "normal" | "small-caps" => {}
                        _ => {
                            if let Some(sz) = font_size(t, c) {
                                s.font_size = sz;
                                s.keyword_size = keyword_size(t, c);
                                k += 1;
                                break;
                            }
                            break;
                        }
                    }
                } else if let [Tok::Number(n)] = values::trim(t) {
                    s.font_weight = n.clamp(1.0, 1000.0) as u16;
                } else if let Some(sz) = font_size(t, c) {
                    s.font_size = sz;
                    s.keyword_size = keyword_size(t, c);
                    k += 1;
                    break;
                } else {
                    break;
                }
                k += 1;
            }
            // "/line-height".
            if let Some(t) = ps.get(k)
                && values::trim(t).first() == Some(&Tok::Delim('/'))
            {
                if let Some(lh) = ps.get(k + 1).and_then(|x| line_height(x, c, s.font_size)) {
                    s.line_height = lh;
                }
                k += 2;
            }
            let rest: Vec<Tok> = ps.iter().skip(k).flat_map(|x| x.iter().cloned().chain(core::iter::once(Tok::Ws))).collect();
            (s.mono, s.sans) = family_kind(&rest);
            s.generic_mono = generic_mono(&rest);
            s.families = Arc::new(family_list(&rest));
        }
        "line-height" => {
            if let Some(x) = line_height(v, c, s.font_size) {
                s.line_height = x;
            }
        }
        "text-align" => {
            if let Some(i) = ident(v) {
                s.text_align = match i.as_str() {
                    "right" | "end" | "-webkit-right" => TextAlign::Right,
                    "center" | "-webkit-center" | "-moz-center" | "match-parent" if i != "match-parent" => TextAlign::Center,
                    "justify" => TextAlign::Justify,
                    "match-parent" => p.text_align,
                    _ => TextAlign::Left,
                };
            }
        }
        "text-indent" => {
            if let Some(x) = values::length(v, units, 0.0) {
                s.text_indent = x;
            }
        }
        "text-decoration" | "text-decoration-line" | "-webkit-text-decoration" => {
            let words: Vec<String> = parts(v).iter().filter_map(|x| ident(x)).collect();
            if words.iter().any(|w| w == "none") {
                s.underline = false;
                s.line_through = false;
                s.overline = false;
            }
            if words.iter().any(|w| w == "underline") {
                s.underline = true;
            }
            if words.iter().any(|w| w == "line-through") {
                s.line_through = true;
            }
            if words.iter().any(|w| w == "overline") {
                s.overline = true;
            }
            if name == "text-decoration" {
                if let Some(col) = parts(v).iter().find_map(|x| values::color(x, cur)) {
                    s.decoration_color = Some(col);
                }
            }
        }
        "text-decoration-color" => s.decoration_color = values::color(v, cur),
        "text-transform" => {
            if let Some(i) = ident(v) {
                s.transform_text = match i.as_str() {
                    "uppercase" => Transform::Uppercase,
                    "lowercase" => Transform::Lowercase,
                    "capitalize" => Transform::Capitalize,
                    _ => Transform::None,
                };
            }
        }
        "white-space" | "white-space-collapse" | "text-wrap-mode" | "text-wrap" => {
            let words: Vec<String> = parts(v).iter().filter_map(|x| ident(x)).collect();
            let has = |w: &str| words.iter().any(|x| x == w);
            s.white_space = match name {
                "text-wrap-mode" | "text-wrap" => {
                    if has("nowrap") {
                        match s.white_space {
                            WhiteSpace::PreWrap | WhiteSpace::Pre => WhiteSpace::Pre,
                            _ => WhiteSpace::NoWrap,
                        }
                    } else if has("wrap") || has("balance") || has("pretty") || has("stable") {
                        match s.white_space {
                            WhiteSpace::Pre => WhiteSpace::PreWrap,
                            WhiteSpace::NoWrap => WhiteSpace::Normal,
                            w => w,
                        }
                    } else {
                        s.white_space
                    }
                }
                "white-space-collapse" => {
                    if has("preserve") || has("break-spaces") {
                        if s.white_space.wraps() { WhiteSpace::PreWrap } else { WhiteSpace::Pre }
                    } else if has("preserve-breaks") {
                        WhiteSpace::PreLine
                    } else if s.white_space.wraps() {
                        WhiteSpace::Normal
                    } else {
                        WhiteSpace::NoWrap
                    }
                }
                _ => {
                    if has("nowrap") && has("preserve") {
                        WhiteSpace::Pre
                    } else if has("nowrap") {
                        WhiteSpace::NoWrap
                    } else if has("pre") {
                        WhiteSpace::Pre
                    } else if has("pre-wrap") || has("break-spaces") {
                        WhiteSpace::PreWrap
                    } else if has("pre-line") {
                        WhiteSpace::PreLine
                    } else {
                        WhiteSpace::Normal
                    }
                }
            };
        }
        "vertical-align" => {
            if let Some(i) = ident(v) {
                s.vertical_align = match i.as_str() {
                    "top" => VAlign::Top,
                    "middle" => VAlign::Middle,
                    "bottom" => VAlign::Bottom,
                    "text-top" => VAlign::TextTop,
                    "text-bottom" => VAlign::TextBottom,
                    "sub" => VAlign::Sub,
                    "super" => VAlign::Super,
                    _ => VAlign::Baseline,
                };
            } else if let Some(l) = values::length(v, units, 0.0) {
                s.vertical_align = VAlign::Len(l.px + l.pct * s.line_px() / 100.0);
            }
        }
        "list-style-type" => {
            if let Some(x) = ident(v).and_then(|i| list_style(&i)) {
                s.list_style = x;
            } else if let [Tok::Str(_)] = values::trim(v) {
                s.list_style = ListStyle::None;
            }
        }
        "list-style-position" => s.list_inside = ident(v).as_deref() == Some("inside"),
        "list-style" => {
            for t in parts(v) {
                if let Some(i) = ident(t) {
                    if i == "inside" {
                        s.list_inside = true;
                    } else if i == "outside" {
                        s.list_inside = false;
                    } else if let Some(x) = list_style(&i) {
                        s.list_style = x;
                    }
                } else if matches!(values::trim(t).first(), Some(Tok::Url(_))) {
                    s.list_image = true;
                }
            }
        }
        "list-style-image" => s.list_image = !matches!(ident(v).as_deref(), Some("none")),
        "visibility" => {
            if let Some(i) = ident(v) {
                s.visible = i == "visible";
            }
        }
        "opacity" => {
            if let Some(n) = values::number(v, units) {
                s.opacity = n.clamp(0.0, 1.0);
            } else if let [Tok::Percent(pc)] = values::trim(v) {
                s.opacity = (pc / 100.0).clamp(0.0, 1.0);
            }
        }
        "overflow" | "overflow-x" | "overflow-y" => {
            let words: Vec<String> = parts(v).iter().filter_map(|x| ident(x)).collect();
            if name == "overflow" || name == "overflow-x" || words.iter().any(|w| w != "visible") {
                s.clip = words.iter().any(|w| matches!(w.as_str(), "hidden" | "clip" | "auto" | "scroll" | "overlay"));
            }
        }
        "clip" => {
            // clip: rect(0 0 0 0), hiding what's in it ("visually hidden").
            if let [Tok::Function(f), rest @ ..] = values::trim(v)
                && f == "rect"
            {
                let n: Vec<f32> = rest.iter().filter_map(|t| match t {
                    Tok::Dimension(x, _) | Tok::Number(x) => Some(*x),
                    _ => None,
                }).collect();
                if n.len() == 4 && n[2] - n[0] <= 1.0 && n[1] - n[3] <= 1.0 {
                    s.hidden_text = true;
                }
            }
        }
        "clip-path" | "-webkit-clip-path" => s.clip_path = effects::clip_path(v, units).map(Arc::new),
        "z-index" => {
            s.z_index = match values::trim(v) {
                [Tok::Number(n)] => Some(*n as i32),
                _ => None,
            };
        }
        "aspect-ratio" => {
            let nums: Vec<f32> = v.iter().filter_map(|t| if let Tok::Number(n) = t { Some(*n) } else { None }).collect();
            s.aspect_ratio = match nums.as_slice() {
                [a] if *a > 0.0 => Some(*a),
                [a, b, ..] if *a > 0.0 && *b > 0.0 => Some(a / b),
                _ => None,
            };
            s.aspect_auto = v.iter().any(|t| matches!(t, Tok::Ident(w) if w.eq_ignore_ascii_case("auto")));
        }
        "object-fit" => {
            if let Some(f) = ident(v).and_then(|w| ObjectFit::from_word(&w)) {
                s.object_fit = f;
            }
        }
        "flex-direction" | "-webkit-flex-direction" => {
            if let Some(i) = ident(v) {
                s.flex_direction = match i.as_str() {
                    "row-reverse" => FlexDirection::RowReverse,
                    "column" => FlexDirection::Column,
                    "column-reverse" => FlexDirection::ColumnReverse,
                    _ => FlexDirection::Row,
                };
            }
        }
        "-webkit-box-orient" => {
            s.box_vertical = ident(v).as_deref() == Some("vertical");
            if s.box_vertical {
                s.flex_direction = FlexDirection::Column;
            }
        }
        "flex-wrap" => s.flex_wrap = matches!(ident(v).as_deref(), Some("wrap" | "wrap-reverse")),
        "flex-flow" => {
            for t in parts(v) {
                match ident(t).as_deref() {
                    Some("row") => s.flex_direction = FlexDirection::Row,
                    Some("row-reverse") => s.flex_direction = FlexDirection::RowReverse,
                    Some("column") => s.flex_direction = FlexDirection::Column,
                    Some("column-reverse") => s.flex_direction = FlexDirection::ColumnReverse,
                    Some("wrap" | "wrap-reverse") => s.flex_wrap = true,
                    Some("nowrap") => s.flex_wrap = false,
                    _ => {}
                }
            }
        }
        "justify-content" => {
            if let Some(a) = align_value(v) {
                s.justify_content = a;
            }
        }
        "align-items" => {
            if let Some(a) = align_value(v) {
                s.align_items = a;
            }
        }
        "align-self" => {
            if let Some(a) = align_value(v) {
                s.align_self = a;
            }
        }
        "align-content" => {
            if let Some(a) = align_value(v) {
                s.align_content = a;
            }
        }
        "justify-items" => {
            if let Some(a) = align_value(v) {
                s.justify_items = a;
            }
        }
        "justify-self" => {
            if let Some(a) = align_value(v) {
                s.justify_self = a;
            }
        }
        "place-items" | "place-content" | "place-self" => {
            let ps = parts(v);
            let a = ps.first().and_then(|t| align_value(t));
            let b = ps.get(1).and_then(|t| align_value(t)).or(a);
            match name {
                "place-items" => {
                    if let Some(a) = a {
                        s.align_items = a;
                    }
                    if let Some(b) = b {
                        s.justify_items = b;
                    }
                }
                "place-content" => {
                    if let Some(a) = a {
                        s.align_content = a;
                    }
                    if let Some(b) = b {
                        s.justify_content = b;
                    }
                }
                _ => {
                    if let Some(a) = a {
                        s.align_self = a;
                    }
                    if let Some(b) = b {
                        s.justify_self = b;
                    }
                }
            }
        }
        "flex" => {
            // none | auto | grow [shrink] [basis] | basis
            match ident(v).as_deref() {
                Some("none") => {
                    s.grow = 0.0;
                    s.shrink = 0.0;
                    s.basis = Size::Auto;
                }
                Some("auto") => {
                    s.grow = 1.0;
                    s.shrink = 1.0;
                    s.basis = Size::Auto;
                }
                Some("initial") => {
                    s.grow = 0.0;
                    s.shrink = 1.0;
                    s.basis = Size::Auto;
                }
                _ => {
                    let mut nums = Vec::new();
                    let mut basis = None;
                    for t in parts(v) {
                        match values::trim(t) {
                            [Tok::Number(n)] => nums.push(*n),
                            x => {
                                if let Some(b) = size_value(x, c, 0.0) {
                                    basis = Some(b);
                                }
                            }
                        }
                    }
                    s.grow = nums.first().copied().unwrap_or(1.0).max(0.0);
                    s.shrink = nums.get(1).copied().unwrap_or(1.0).max(0.0);
                    s.basis = basis.unwrap_or(Size::Len(Len::ZERO));
                }
            }
        }
        "flex-grow" | "-webkit-box-flex" => {
            if let Some(n) = values::number(v, units) {
                s.grow = n.max(0.0);
            }
        }
        "flex-shrink" => {
            if let Some(n) = values::number(v, units) {
                s.shrink = n.max(0.0);
            }
        }
        "flex-basis" => {
            if let Some(x) = size_value(v, c, 0.0) {
                s.basis = if ident(v).as_deref() == Some("content") { Size::Auto } else { x };
            }
        }
        "order" => {
            if let [Tok::Number(n)] = values::trim(v) {
                s.order = *n as i32;
            }
        }
        "gap" | "grid-gap" => {
            let ps = parts(v);
            if let Some(a) = ps.first().and_then(|t| values::length(t, units, 0.0)) {
                s.row_gap = a;
                s.column_gap = ps.get(1).and_then(|t| values::length(t, units, 0.0)).unwrap_or(a);
            }
        }
        "row-gap" | "grid-row-gap" => {
            if let Some(a) = values::length(v, units, 0.0) {
                s.row_gap = a;
            }
        }
        "column-gap" | "grid-column-gap" => {
            if let Some(a) = values::length(v, units, 0.0) {
                s.column_gap = a;
            }
        }
        "grid-template-columns" => s.grid_columns = tracks(v, c),
        "grid-template-rows" => s.grid_rows = tracks(v, c),
        "grid-template-areas" => {
            s.grid_areas = v.iter().filter_map(|t| if let Tok::Str(row) = t { Some(row.split_ascii_whitespace().map(String::from).collect()) } else { None }).collect();
        }
        "grid-template" => {
            // "rows / columns", or areas with each row's size after its string: "a a" 40px "b c" 1fr / auto 1fr.
            let slash = v.iter().position(|t| *t == Tok::Delim('/'));
            let before = &v[..slash.unwrap_or(v.len())];
            let areas: Vec<Vec<String>> = before.iter().filter_map(|t| if let Tok::Str(row) = t { Some(row.split_ascii_whitespace().map(String::from).collect()) } else { None }).collect();
            if areas.is_empty() {
                s.grid_rows = tracks(before, c);
            } else {
                s.grid_areas = areas;
                let mut rows = Vec::new();
                let mut i = 0;
                while i < before.len() {
                    if matches!(before[i], Tok::Str(_)) {
                        let mut j = i + 1;
                        while j < before.len() && !matches!(before[j], Tok::Str(_)) {
                            j += 1;
                        }
                        rows.push(tracks(&before[i + 1..j], c).into_iter().next().unwrap_or(Track::Auto));
                        i = j;
                    } else {
                        i += 1;
                    }
                }
                s.grid_rows = rows;
            }
            if let Some(slash) = slash {
                s.grid_columns = tracks(&v[slash + 1..], c);
            }
        }
        "grid-area" => {
            if let Some(i) = ident(v)
                && i != "auto"
            {
                s.grid_area = Some(match values::trim(v) {
                    [Tok::Ident(raw)] => raw.clone(),
                    _ => i,
                });
            } else {
                // "row-start / column-start / row-end / column-end".
                let nums: Vec<Vec<Tok>> = {
                    let mut out: Vec<Vec<Tok>> = vec![Vec::new()];
                    for t in v {
                        if *t == Tok::Delim('/') {
                            out.push(Vec::new());
                        } else if let Some(l) = out.last_mut() {
                            l.push(t.clone());
                        }
                    }
                    out
                };
                if nums.len() >= 2 {
                    let mut row = nums[0].clone();
                    let mut col = nums[1].clone();
                    if let Some(r2) = nums.get(2) {
                        row.push(Tok::Delim('/'));
                        row.extend(r2.iter().cloned());
                    }
                    if let Some(c2) = nums.get(3) {
                        col.push(Tok::Delim('/'));
                        col.extend(c2.iter().cloned());
                    }
                    s.grid_row = grid_place(&row);
                    s.grid_column = grid_place(&col);
                }
            }
        }
        "grid-column" => s.grid_column = grid_place(v),
        "grid-row" => s.grid_row = grid_place(v),
        "grid-column-start" => s.grid_column.start = grid_place(v).start.max(s.grid_column.start),
        "grid-column-end" => {
            let g = grid_place(v);
            s.grid_column.end = g.start;
            if g.span > 0 {
                s.grid_column.span = g.span;
            } else if s.grid_column.start > 0 && g.start > s.grid_column.start {
                s.grid_column.span = (g.start - s.grid_column.start) as u32;
            }
        }
        "grid-row-start" => s.grid_row.start = grid_place(v).start,
        "grid-row-end" => {
            let g = grid_place(v);
            s.grid_row.end = g.start;
            if g.span > 0 {
                s.grid_row.span = g.span;
            }
        }
        "grid-auto-flow" => s.grid_auto_columns = parts(v).iter().any(|t| ident(t).as_deref() == Some("column")),
        "border-collapse" => s.border_collapse = ident(v).as_deref() == Some("collapse"),
        "caption-side" => s.caption_bottom = ident(v).as_deref() == Some("bottom"),
        "border-spacing" => {
            if let Some(l) = parts(v).first().and_then(|t| values::length(t, units, 0.0)) {
                s.border_spacing = l.px.max(0.0);
            }
        }
        "content" => {
            s.content = content(v, |_| None);
        }
        _ => {}
    }
}

/// Which side a property's name is for: 0 top, 1 right, 2 bottom, 3 left (logical sides as left-to-right text has).
fn side(name: &str) -> usize {
    if name.contains("top") || name.contains("block-start") {
        0
    } else if name.contains("right") || name.contains("inline-end") {
        1
    } else if name.contains("bottom") || name.contains("block-end") {
        2
    } else {
        3
    }
}

/// A property taken from `from` (for "inherit", and the like).
fn inherit_one(s: &mut Style, from: &Style, name: &str) {
    match name {
        "color" => s.color = from.color,
        "font-size" => {
            s.font_size = from.font_size;
            s.keyword_size = from.keyword_size;
        }
        "font-weight" => s.font_weight = from.font_weight,
        "font-style" => s.italic = from.italic,
        "font-stretch" | "font-width" => s.font_stretch = from.font_stretch,
        "font-family" => {
            s.mono = from.mono;
            s.sans = from.sans;
            s.generic_mono = from.generic_mono;
            s.families = from.families.clone();
        }
        "font" => {
            s.font_size = from.font_size;
            s.keyword_size = from.keyword_size;
            s.font_weight = from.font_weight;
            s.italic = from.italic;
            s.mono = from.mono;
            s.sans = from.sans;
            s.generic_mono = from.generic_mono;
            s.line_height = from.line_height;
        }
        "line-height" => s.line_height = from.line_height,
        "text-align" => s.text_align = from.text_align,
        "white-space" => s.white_space = from.white_space,
        "image-rendering" => s.pixelated = from.pixelated,
        "visibility" => s.visible = from.visible,
        "display" => s.display = from.display,
        "position" => s.position = from.position,
        "width" => s.width = from.width,
        "height" => s.height = from.height,
        "max-width" => s.max_width = from.max_width,
        "min-width" => s.min_width = from.min_width,
        "max-height" => s.max_height = from.max_height,
        "min-height" => s.min_height = from.min_height,
        "margin" => s.margin = from.margin,
        "padding" => s.padding = from.padding,
        "background" | "background-color" => {
            s.background = from.background;
            if name == "background" {
                s.bg = from.bg.clone();
            }
        }
        "background-image" | "background-position" | "background-position-x" | "background-position-y" | "background-size" | "-webkit-background-size" | "background-repeat" | "background-origin" | "-webkit-background-origin" | "background-clip" | "-webkit-background-clip" | "background-attachment" => {
            let prop = layer_prop(name.trim_start_matches("-webkit-").trim_start_matches("background-"));
            s.bg = with_layer_prop(&s.bg, &from.bg, prop);
        }
        "mask" | "-webkit-mask" => s.mask = from.mask.clone(),
        "mask-image" | "-webkit-mask-image" | "mask-position" | "-webkit-mask-position" | "mask-size" | "-webkit-mask-size" | "mask-repeat" | "-webkit-mask-repeat" | "mask-origin" | "-webkit-mask-origin" | "mask-clip" | "-webkit-mask-clip" | "mask-mode" | "mask-composite" | "-webkit-mask-composite" => {
            let prop = layer_prop(name.trim_start_matches("-webkit-").trim_start_matches("mask-"));
            s.mask = with_layer_prop(&s.mask, &from.mask, prop);
        }
        "filter" | "-webkit-filter" => s.filter = from.filter.clone(),
        "backdrop-filter" | "-webkit-backdrop-filter" => s.backdrop = from.backdrop.clone(),
        "mix-blend-mode" => s.blend = from.blend,
        "isolation" => s.isolate = from.isolate,
        "clip-path" | "-webkit-clip-path" => s.clip_path = from.clip_path.clone(),
        "transform-origin" | "-webkit-transform-origin" => s.origin = from.origin,
        "object-fit" => s.object_fit = from.object_fit,
        "object-position" => s.object_position = from.object_position,
        "border" => {
            s.border_width = from.border_width;
            s.border_style = from.border_style;
            s.border_color = from.border_color;
        }
        "float" => s.float = from.float,
        "opacity" => s.opacity = from.opacity,
        "text-decoration" | "text-decoration-line" => {
            s.underline = from.underline;
            s.line_through = from.line_through;
            s.overline = from.overline;
        }
        "list-style-type" | "list-style" => s.list_style = from.list_style,
        "text-transform" => s.transform_text = from.transform_text,
        "overflow" => s.clip = from.clip,
        "flex-direction" => s.flex_direction = from.flex_direction,
        "justify-content" => s.justify_content = from.justify_content,
        "align-items" => s.align_items = from.align_items,
        "transform" | "-webkit-transform" => s.transform = from.transform.clone(),
        "translate" => s.transform_parts[0] = from.transform_parts[0],
        "rotate" => s.transform_parts[1] = from.transform_parts[1],
        "scale" => s.transform_parts[2] = from.transform_parts[2],
        "border-radius" => s.radius = from.radius,
        "z-index" => s.z_index = from.z_index,
        "content" => s.content = from.content.clone(),
        "vertical-align" => s.vertical_align = from.vertical_align,
        "fill" => s.fill = from.fill,
        "stroke" => s.stroke = from.stroke,
        _ => {}
    }
}

/// Which layer list property a background-* or mask-* name (without its prefix) is.
fn layer_prop(rest: &str) -> LayerProp {
    match rest {
        "image" => LayerProp::Image,
        "position" => LayerProp::Position,
        "position-x" => LayerProp::PositionX,
        "position-y" => LayerProp::PositionY,
        "size" => LayerProp::Size,
        "repeat" => LayerProp::Repeat,
        "origin" => LayerProp::Origin,
        "clip" => LayerProp::Clip,
        "attachment" => LayerProp::Attachment,
        "mode" => LayerProp::Mode,
        _ => LayerProp::Composite,
    }
}

/// `to`'s layers with one property's list taken from `from`'s (inherit, initial).
fn with_layer_prop(to: &Option<Arc<Layers>>, from: &Option<Arc<Layers>>, prop: LayerProp) -> Option<Arc<Layers>> {
    let mut l = to.as_deref().cloned().unwrap_or_default();
    let f = from.as_deref().cloned().unwrap_or_default();
    match prop {
        LayerProp::Image => l.images = f.images,
        LayerProp::Position => {
            l.pos_x = f.pos_x;
            l.pos_y = f.pos_y;
        }
        LayerProp::PositionX => l.pos_x = f.pos_x,
        LayerProp::PositionY => l.pos_y = f.pos_y,
        LayerProp::Size => l.sizes = f.sizes,
        LayerProp::Repeat => l.repeats = f.repeats,
        LayerProp::Origin => l.origins = f.origins,
        LayerProp::Clip => l.clips = f.clips,
        LayerProp::Attachment => l.fixed = f.fixed,
        LayerProp::Mode => l.luminance = f.luminance,
        LayerProp::Composite => l.composite = f.composite,
    }
    Some(Arc::new(l))
}
