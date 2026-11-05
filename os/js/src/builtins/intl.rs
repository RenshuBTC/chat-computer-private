//! Intl: numbers, dates and times, lists, plurals, relative times and the rest of ECMA-402 that pages' scripts use
//! (NumberFormat, DateTimeFormat, Collator, PluralRules, RelativeTimeFormat, ListFormat, Segmenter, DisplayNames,
//! Locale), and the methods that go through it (toLocaleString, toLocaleDateString, toLocaleTimeString,
//! localeCompare). Everything is written in American English, whatever locale is asked for (a page asking for another
//! gets English, and resolvedOptions says so), with the options ECMA-402 gives: styles, currencies, units, digits and
//! the ways of rounding them (on the number as it's written in decimal, not on its binary value), grouping, signs,
//! compact, scientific and engineering notation; dates' fields in their lengths, the hour cycles, and time zones (UTC,
//! the PC's own, offsets, and the common zones by name with their daylight saving rules); collation in Unicode's
//! default order (letters before their accents before their case); and text's graphemes, words and sentences by
//! Unicode's rules, simplified.

use super::{arg, getter, js_str, sym_method, tag};
use crate::convert::Hint;
use crate::names as n;
use crate::object::*;
use crate::string::JsStr;
use crate::value::{Gc, Value, number_to_string};
use crate::vm::{I, Sym, Vm};
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

type R = Result<Value, Value>;
type Parts = Vec<(&'static str, String)>;

/// An Intl object: the locale it resolved to, its settings, and its format (or compare) function once a script has
/// asked for it.
pub struct Intl {
    pub locale: String,
    pub data: IntlData,
    pub bound: Option<Gc>,
}

pub enum IntlData {
    Number(Box<NumberOpts>),
    Date(Box<DateOpts>),
    Collator(CollatorOpts),
    Plural(Box<PluralOpts>),
    Relative(Box<RelativeOpts>),
    List(ListOpts),
    /// A Segmenter's granularity (0 grapheme, 1 word, 2 sentence).
    Segmenter(u8),
    /// What Segmenter.segment gave: the string and the granularity.
    Segments(JsStr, u8),
    /// An iterator over segments: the string, the granularity, and where the next starts (in UTF-16 units).
    SegmentIter(JsStr, u8, usize),
    /// DisplayNames: its type, style, whether it gives undefined for what it has no name for, and whether languages
    /// are named in the standard way ("English (United Kingdom)", not "British English").
    DisplayNames(u8, u8, bool, bool),
    Locale(String),
}

/// The locale this writes in when none it has is asked for.
const DEFAULT_LOCALE: &str = "en-US";

fn data_of<'a>(vm: &'a Vm, v: &Value) -> Option<&'a IntlData> {
    match v {
        Value::Obj(g) => match &vm.heap.get(*g).class {
            Class::Intl(i) => Some(&i.data),
            _ => None,
        },
        _ => None,
    }
}

fn alloc_intl(vm: &mut Vm, proto: Gc, locale: String, data: IntlData) -> Value {
    Value::Obj(vm.alloc(Some(proto), Class::Intl(Box::new(Intl { locale, data, bound: None }))))
}

/// The locale an Intl object resolved to.
fn locale_of(vm: &Vm, v: &Value) -> String {
    match v {
        Value::Obj(g) => match &vm.heap.get(*g).class {
            Class::Intl(i) => i.locale.clone(),
            _ => String::from(DEFAULT_LOCALE),
        },
        _ => String::from(DEFAULT_LOCALE),
    }
}

fn incompatible(vm: &mut Vm, what: &str) -> Value {
    vm.type_error(&format!("Method Intl.{what} called on incompatible receiver"))
}

// ------------------------------------------------------------------------------------------- the interfaces

type Methods = &'static [(&'static str, u32, NativeFn)];
type Getters = &'static [(&'static str, NativeFn)];

pub fn init(vm: &mut Vm) {
    let op = vm.proto(I::ObjectProto);
    let intl = vm.alloc(Some(op), Class::Ordinary);
    tag(vm, intl, "Intl");
    vm.method(intl, "getCanonicalLocales", 1, get_canonical_locales);
    vm.method(intl, "supportedValuesOf", 1, supported_values_of);
    let g = vm.global;
    vm.put(g, "Intl", Value::Obj(intl), HIDDEN);
    let kinds: [(&str, u32, I, NativeFn, Methods, Getters); 9] = [
        (
            "NumberFormat",
            0,
            I::IntlNumberFormatProto,
            number_format_new,
            &[("formatToParts", 1, nf_format_to_parts), ("formatRange", 2, nf_format_range), ("formatRangeToParts", 2, nf_format_range_to_parts), ("resolvedOptions", 0, nf_resolved)],
            &[("format", nf_format_getter)],
        ),
        (
            "DateTimeFormat",
            0,
            I::IntlDateTimeFormatProto,
            date_format_new,
            &[("formatToParts", 1, df_format_to_parts), ("formatRange", 2, df_format_range), ("formatRangeToParts", 2, df_format_range_to_parts), ("resolvedOptions", 0, df_resolved)],
            &[("format", df_format_getter)],
        ),
        ("Collator", 0, I::IntlCollatorProto, collator_new, &[("resolvedOptions", 0, collator_resolved)], &[("compare", collator_compare_getter)]),
        ("PluralRules", 0, I::IntlPluralRulesProto, plural_new, &[("select", 1, plural_select), ("selectRange", 2, plural_select_range), ("resolvedOptions", 0, plural_resolved)], &[]),
        (
            "RelativeTimeFormat",
            0,
            I::IntlRelativeTimeFormatProto,
            relative_new,
            &[("format", 2, relative_format), ("formatToParts", 2, relative_format_to_parts), ("resolvedOptions", 0, relative_resolved)],
            &[],
        ),
        ("ListFormat", 0, I::IntlListFormatProto, list_new, &[("format", 1, list_format), ("formatToParts", 1, list_format_to_parts), ("resolvedOptions", 0, list_resolved)], &[]),
        ("Segmenter", 0, I::IntlSegmenterProto, segmenter_new, &[("segment", 1, segmenter_segment), ("resolvedOptions", 0, segmenter_resolved)], &[]),
        ("DisplayNames", 2, I::IntlDisplayNamesProto, display_names_new, &[("of", 1, display_names_of), ("resolvedOptions", 0, display_names_resolved)], &[]),
        (
            "Locale",
            1,
            I::IntlLocaleProto,
            locale_new,
            &[("toString", 0, locale_to_string), ("toJSON", 0, locale_to_string), ("maximize", 0, locale_maximize), ("minimize", 0, locale_minimize)],
            &[
                ("baseName", locale_base_name),
                ("language", locale_language),
                ("region", locale_region),
                ("script", locale_script),
                ("calendar", locale_calendar),
                ("caseFirst", locale_case_first),
                ("collation", locale_collation),
                ("hourCycle", locale_hour_cycle),
                ("numeric", locale_numeric),
                ("numberingSystem", locale_numbering),
            ],
        ),
    ];
    for (name, length, i, construct, methods, getters) in kinds {
        let p = vm.alloc(Some(op), Class::Ordinary);
        vm.intrinsics[i as usize] = p;
        tag(vm, p, &format!("Intl.{name}"));
        let c = vm.native_with(name, length, ctor_call, Some(construct), js_str(name));
        vm.put_key(c, Key::Str(n::PROTOTYPE), Value::Obj(p), 0);
        vm.put_key(p, Key::Str(n::CONSTRUCTOR), Value::Obj(c), HIDDEN);
        if name != "Locale" {
            vm.method(c, "supportedLocalesOf", 1, supported_locales_of);
        }
        for &(m, l, f) in methods {
            vm.method(p, m, l, f);
        }
        for &(m, f) in getters {
            getter(vm, p, m, f);
        }
        vm.put(intl, name, Value::Obj(c), HIDDEN);
    }
    // Segments (what segment gives) and their iterator.
    let sp = vm.alloc(Some(op), Class::Ordinary);
    vm.intrinsics[I::IntlSegmentsProto as usize] = sp;
    vm.method(sp, "containing", 1, segments_containing);
    sym_method(vm, sp, Sym::Iterator, "[Symbol.iterator]", 0, segments_iterator);
    let ip = vm.proto(I::IteratorProto);
    let sip = vm.alloc(Some(ip), Class::Ordinary);
    vm.intrinsics[I::IntlSegmentIterProto as usize] = sip;
    vm.method(sip, "next", 0, segment_iter_next);
    tag(vm, sip, "Segmenter String Iterator");
}

/// Intl's constructors called without new: NumberFormat, DateTimeFormat and Collator make one all the same (as
/// they always have); the others need new.
fn ctor_call(vm: &mut Vm, _this: &Value, args: &[Value], f: Gc) -> R {
    let name = match vm.data(f) {
        Value::Str(s) => s.to_rust(),
        _ => String::new(),
    };
    match name.as_str() {
        "NumberFormat" => number_format_new(vm, &Value::Undefined, args, f),
        "DateTimeFormat" => date_format_new(vm, &Value::Undefined, args, f),
        "Collator" => collator_new(vm, &Value::Undefined, args, f),
        _ => Err(vm.type_error(&format!("Constructor Intl.{name} requires 'new'"))),
    }
}

/// A getter's function bound to its object (format, compare): made once, the same each time after.
fn bound(vm: &mut Vm, this: &Value, length: u32, f: NativeFn) -> R {
    let Value::Obj(g) = this else { return Err(vm.type_error("not an Intl object")) };
    let g = *g;
    if let Class::Intl(i) = &vm.heap.get(g).class
        && let Some(b) = i.bound
    {
        return Ok(Value::Obj(b));
    }
    let b = vm.native_with("", length, f, None, this.clone());
    if let Class::Intl(i) = &mut vm.heap.get_mut(g).class {
        i.bound = Some(b);
    }
    Ok(Value::Obj(b))
}

// ------------------------------------------------------------------------------------------------ locales

/// Old language codes and their replacements.
const LANGUAGE_ALIASES: &[(&str, &str)] = &[("iw", "he"), ("in", "id"), ("ji", "yi"), ("jw", "jv"), ("mo", "ro"), ("cmn", "zh"), ("art-lojban", "jbo")];

fn title(p: &str) -> String {
    let mut out = p[..1].to_ascii_uppercase();
    out.push_str(&p[1..]);
    out
}

fn is_alpha(p: &str) -> bool {
    p.bytes().all(|b| b.is_ascii_alphabetic())
}

fn is_variant(p: &str) -> bool {
    p.len() >= 5 || (p.len() == 4 && p.as_bytes()[0].is_ascii_digit())
}

/// A -u- extension's subtags in canonical order (attributes, then keywords sorted by key, "true" left out), if
/// they're well formed.
fn unicode_extension(sub: &[String]) -> Option<String> {
    let mut i = 0;
    let mut out: Vec<String> = Vec::new();
    while i < sub.len() && sub[i].len() >= 3 {
        if !out.contains(&sub[i]) {
            out.push(sub[i].clone());
        }
        i += 1;
    }
    let mut keys: Vec<(String, Vec<String>)> = Vec::new();
    while i < sub.len() {
        let k = &sub[i];
        if k.len() != 2 || !k.as_bytes()[1].is_ascii_alphabetic() {
            return None;
        }
        i += 1;
        let mut values = Vec::new();
        while i < sub.len() && sub[i].len() >= 3 {
            values.push(sub[i].clone());
            i += 1;
        }
        if !keys.iter().any(|(x, _)| x == k) {
            keys.push((k.clone(), values));
        }
    }
    keys.sort();
    for (k, v) in keys {
        out.push(k);
        if !(v.len() == 1 && v[0] == "true") {
            out.extend(v);
        }
    }
    Some(out.join("-"))
}

/// A -t- extension's subtags in canonical order (its language, then its fields sorted), if they're well formed.
fn transformed_extension(sub: &[String]) -> Option<String> {
    let mut i = 0;
    let mut out: Vec<String> = Vec::new();
    if i < sub.len() && is_alpha(&sub[i]) && (2..=8).contains(&sub[i].len()) && sub[i].len() != 4 {
        out.push(sub[i].clone());
        i += 1;
        if i < sub.len() && sub[i].len() == 4 && is_alpha(&sub[i]) {
            out.push(sub[i].clone());
            i += 1;
        }
        if i < sub.len() && ((sub[i].len() == 2 && is_alpha(&sub[i])) || (sub[i].len() == 3 && sub[i].bytes().all(|b| b.is_ascii_digit()))) {
            out.push(sub[i].clone());
            i += 1;
        }
        let mut variants: Vec<String> = Vec::new();
        while i < sub.len() && is_variant(&sub[i]) {
            if variants.contains(&sub[i]) {
                return None;
            }
            variants.push(sub[i].clone());
            i += 1;
        }
        variants.sort();
        out.extend(variants);
    }
    let mut fields: Vec<(String, Vec<String>)> = Vec::new();
    while i < sub.len() {
        let k = &sub[i];
        if k.len() != 2 || !k.as_bytes()[0].is_ascii_alphabetic() || !k.as_bytes()[1].is_ascii_digit() {
            return None;
        }
        i += 1;
        let mut values = Vec::new();
        while i < sub.len() && sub[i].len() >= 3 {
            values.push(sub[i].clone());
            i += 1;
        }
        if values.is_empty() || fields.iter().any(|(x, _)| x == k) {
            return None;
        }
        fields.push((k.clone(), values));
    }
    if out.is_empty() && fields.is_empty() {
        return None;
    }
    fields.sort();
    for (k, v) in fields {
        out.push(k);
        out.extend(v);
    }
    Some(out.join("-"))
}

/// A language tag as Unicode's locale identifiers write it (language lower case, script title case, region upper
/// case; variants and extensions in order, -u- keywords sorted), if it's a well-formed one.
fn canonical(tag: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let lower = LANGUAGE_ALIASES.iter().find(|(a, _)| lower == *a || lower.strip_prefix(a).is_some_and(|r| r.starts_with('-'))).map_or(lower.clone(), |(a, b)| format!("{b}{}", &lower[a.len()..]));
    let parts: Vec<String> = lower.split('-').map(String::from).collect();
    if parts.iter().any(|p| p.is_empty() || p.len() > 8 || !p.bytes().all(|b| b.is_ascii_alphanumeric())) {
        return None;
    }
    let lang = &parts[0];
    if !is_alpha(lang) || !(2..=8).contains(&lang.len()) || lang.len() == 4 {
        return None;
    }
    let mut out = lang.clone();
    let mut i = 1;
    if i < parts.len() && parts[i].len() == 4 && is_alpha(&parts[i]) {
        out.push('-');
        out.push_str(&title(&parts[i]));
        i += 1;
    }
    if i < parts.len() && ((parts[i].len() == 2 && is_alpha(&parts[i])) || (parts[i].len() == 3 && parts[i].bytes().all(|b| b.is_ascii_digit()))) {
        out.push('-');
        out.push_str(&parts[i].to_ascii_uppercase());
        i += 1;
    }
    let mut variants: Vec<&String> = Vec::new();
    while i < parts.len() && is_variant(&parts[i]) {
        if variants.contains(&&parts[i]) {
            return None;
        }
        variants.push(&parts[i]);
        i += 1;
    }
    variants.sort();
    for v in variants {
        out.push('-');
        out.push_str(v);
    }
    let mut extensions: Vec<(u8, String)> = Vec::new();
    let mut private = None;
    while i < parts.len() {
        if parts[i].len() != 1 {
            return None;
        }
        let singleton = parts[i].as_bytes()[0];
        i += 1;
        let start = i;
        if singleton == b'x' {
            if i == parts.len() {
                return None;
            }
            private = Some(parts[i..].join("-"));
            break;
        }
        while i < parts.len() && parts[i].len() > 1 {
            i += 1;
        }
        let sub = &parts[start..i];
        if sub.is_empty() || extensions.iter().any(|(c, _)| *c == singleton) {
            return None;
        }
        let body = match singleton {
            b'u' => unicode_extension(sub)?,
            b't' => transformed_extension(sub)?,
            _ => sub.join("-"),
        };
        extensions.push((singleton, body));
    }
    extensions.sort();
    for (c, body) in extensions {
        if !body.is_empty() {
            out.push('-');
            out.push(c as char);
            out.push('-');
            out.push_str(&body);
        }
    }
    if let Some(p) = private {
        out.push_str("-x-");
        out.push_str(&p);
    }
    Some(out)
}

/// The locales asked for (a tag, a list of them, or a Locale), each as BCP 47 writes it; a RangeError for one that
/// isn't a tag.
fn locales(vm: &mut Vm, v: &Value) -> Result<Vec<String>, Value> {
    let list: Vec<Value> = match v {
        Value::Undefined => return Ok(Vec::new()),
        Value::Str(_) => vec![v.clone()],
        Value::Obj(_) if matches!(data_of(vm, v), Some(IntlData::Locale(_))) => vec![v.clone()],
        Value::Null => return Err(vm.type_error("Cannot convert null to object")),
        _ => {
            let o = vm.to_object(v)?;
            let len = vm.get(o, Key::Str(n::LENGTH), &Value::Obj(o))?;
            let len = vm.to_number(&len)?;
            let len = if len.is_nan() { 0 } else { len.clamp(0.0, 1000.0) as u32 };
            let mut out = Vec::new();
            for k in 0..len {
                if vm.has_property(o, Key::Index(k))? {
                    out.push(vm.get(o, Key::Index(k), &Value::Obj(o))?);
                }
            }
            out
        }
    };
    let mut out: Vec<String> = Vec::new();
    for item in list {
        let s = match (&item, data_of(vm, &item)) {
            (_, Some(IntlData::Locale(l))) => l.clone(),
            (Value::Str(s), _) => s.to_rust(),
            (Value::Obj(_), _) => vm.to_string(&item)?.to_rust(),
            _ => return Err(vm.type_error("Language ID should be string or object.")),
        };
        let Some(c) = canonical(&s) else { return Err(vm.range_error(&format!("Incorrect locale information provided: {s}"))) };
        if !out.contains(&c) {
            out.push(c);
        }
    }
    Ok(out)
}

/// What a constructor's locales come to (ECMA-402's ResolveLocale, with the locales "en" and "en-US"): the locale
/// it writes in, the request's -u- keywords, and those it takes, to say in the locale resolvedOptions gives.
struct Resolution {
    base: &'static str,
    keywords: Vec<(String, String)>,
    kept: Vec<String>,
}

impl Resolution {
    fn new(requested: &[String]) -> Resolution {
        for r in requested {
            let (lang, script, region) = subtags(r);
            if lang != "en" {
                continue;
            }
            let base = if script.is_none() && region.as_deref() == Some("US") { "en-US" } else { "en" };
            let mut keywords = Vec::new();
            if let Some(i) = r.find("-u-") {
                let rest = &r[i + 3..];
                let rest = rest.split("-x-").next().unwrap_or("");
                let mut key: Option<String> = None;
                let mut value: Vec<&str> = Vec::new();
                for p in rest.split('-') {
                    if p.len() == 1 {
                        break;
                    }
                    if p.len() == 2 {
                        if let Some(k) = key.take() {
                            keywords.push((k, value.join("-")));
                        }
                        key = Some(String::from(p));
                        value.clear();
                    } else if key.is_some() {
                        value.push(p);
                    }
                }
                if let Some(k) = key {
                    keywords.push((k, value.join("-")));
                }
            }
            return Resolution { base, keywords, kept: Vec::new() };
        }
        Resolution { base: DEFAULT_LOCALE, keywords: Vec::new(), kept: Vec::new() }
    }

    /// A relevant key's value: an option's, if it's one the locale has; else the request's keyword, if it's one the
    /// locale has (and then the keyword stays in the locale); else None (the locale's default).
    fn key(&mut self, key: &str, option: Option<&str>, has: &[&str]) -> Option<String> {
        let mut value = None;
        let mut kept = None;
        if let Some((_, v)) = self.keywords.iter().find(|(k, _)| k == key) {
            let v = if v.is_empty() { "true" } else { v.as_str() };
            if has.contains(&v) {
                value = Some(String::from(v));
                kept = Some(if v == "true" { String::from(key) } else { format!("{key}-{v}") });
            }
        }
        if let Some(o) = option {
            let o = if o.is_empty() { "true" } else { o };
            if has.contains(&o) && value.as_deref() != Some(o) {
                value = Some(String::from(o));
                kept = None;
            }
        }
        if let Some(k) = kept {
            self.kept.push(k);
        }
        value
    }

    fn locale(&self) -> String {
        if self.kept.is_empty() { String::from(self.base) } else { format!("{}-u-{}", self.base, self.kept.join("-")) }
    }
}

fn resolution(vm: &mut Vm, v: &Value) -> Result<Resolution, Value> {
    let list = locales(vm, v)?;
    Ok(Resolution::new(&list))
}

/// The language whose case mapping toLocaleUpperCase and toLocaleLowerCase use for these locales: "tr" (Turkish and
/// Azeri's dotted and dotless i), else None (Unicode's default).
pub fn case_language(vm: &mut Vm, v: &Value) -> Result<Option<&'static str>, Value> {
    let list = locales(vm, v)?;
    Ok(match list.first().map(|l| subtags(l).0) {
        Some(l) if l == "tr" || l == "az" => Some("tr"),
        _ => None,
    })
}

fn get_canonical_locales(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let list = locales(vm, &arg(args, 0))?;
    let values = list.iter().map(|s| js_str(s)).collect();
    Ok(Value::Obj(vm.new_array(values)))
}

/// supportedLocalesOf: those asked for that this writes in (English).
fn supported_locales_of(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let list = locales(vm, &arg(args, 0))?;
    let o = options(vm, &arg(args, 1))?;
    opt_str(vm, o, "localeMatcher", MATCHERS, Some(1))?;
    let values = list.iter().filter(|l| *l == "en" || l.starts_with("en-")).map(|s| js_str(s)).collect();
    Ok(Value::Obj(vm.new_array(values)))
}

fn supported_values_of(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let key = vm.to_string(&arg(args, 0))?.to_rust();
    let list: Vec<&str> = match key.as_str() {
        "calendar" => vec!["gregory"],
        "collation" => Vec::new(),
        "currency" => CURRENCIES.iter().map(|c| c.0).collect(),
        "numberingSystem" => numbering_names(),
        "timeZone" => ZONES.iter().map(|z| z.0).collect(),
        "unit" => UNITS.iter().map(|u| u.0).collect(),
        _ => return Err(vm.range_error(&format!("Invalid key : {key}"))),
    };
    let mut list: Vec<String> = list.into_iter().map(String::from).collect();
    if key == "timeZone" {
        list.extend((1..=12).map(|h| format!("Etc/GMT+{h}")));
        list.extend((1..=14).map(|h| format!("Etc/GMT-{h}")));
    }
    list.sort_unstable();
    let values = list.iter().map(|s| js_str(s)).collect();
    Ok(Value::Obj(vm.new_array(values)))
}

// ------------------------------------------------------------------------------------------------ options

const MATCHERS: &[&str] = &["lookup", "best fit"];

/// GetOptionsObject: the options argument of the newer constructors (ListFormat, Segmenter, DisplayNames), which
/// has to be an object if it's given.
fn options_object(vm: &mut Vm, v: &Value) -> Result<Option<Gc>, Value> {
    match v {
        Value::Undefined => Ok(None),
        Value::Obj(g) => Ok(Some(*g)),
        _ => Err(vm.type_error("Options must be an object")),
    }
}

/// The options argument: None if it wasn't given; an object (a primitive made one).
fn options(vm: &mut Vm, v: &Value) -> Result<Option<Gc>, Value> {
    match v {
        Value::Undefined => Ok(None),
        Value::Null => Err(vm.type_error("Cannot convert null to object")),
        _ => Ok(Some(vm.to_object(v)?)),
    }
}

fn get(vm: &mut Vm, o: Option<Gc>, name: &str) -> R {
    match o {
        Some(g) => {
            let a = vm.atom(name);
            vm.get(g, Key::Str(a), &Value::Obj(g))
        }
        None => Ok(Value::Undefined),
    }
}

/// A string option: which of `allowed` it is, or the default; a RangeError for another.
fn opt_str(vm: &mut Vm, o: Option<Gc>, name: &str, allowed: &[&str], default: Option<usize>) -> Result<Option<usize>, Value> {
    let v = get(vm, o, name)?;
    if v.is_undefined() {
        return Ok(default);
    }
    let s = vm.to_string(&v)?.to_rust();
    match allowed.iter().position(|a| *a == s) {
        Some(i) => Ok(Some(i)),
        None => Err(vm.range_error(&format!("Value {s} out of range for Intl options property {name}"))),
    }
}

/// A free-form string option.
fn opt_text(vm: &mut Vm, o: Option<Gc>, name: &str) -> Result<Option<String>, Value> {
    let v = get(vm, o, name)?;
    if v.is_undefined() {
        return Ok(None);
    }
    Ok(Some(vm.to_string(&v)?.to_rust()))
}

/// DefaultNumberOption: a number from `min` to `max` (floored), if it's given.
fn number_option(vm: &mut Vm, v: &Value, min: u32, max: u32, name: &str) -> Result<Option<u32>, Value> {
    if v.is_undefined() {
        return Ok(None);
    }
    let x = vm.to_number(v)?;
    if x.is_nan() || x < min as f64 || x > max as f64 {
        return Err(vm.range_error(&format!("{name} value is out of range.")));
    }
    Ok(Some(crate::math::floor(x) as u32))
}

fn opt_num(vm: &mut Vm, o: Option<Gc>, name: &str, min: u32, max: u32) -> Result<Option<u32>, Value> {
    let v = get(vm, o, name)?;
    number_option(vm, &v, min, max, name)
}

fn opt_bool(vm: &mut Vm, o: Option<Gc>, name: &str) -> Result<Option<bool>, Value> {
    let v = get(vm, o, name)?;
    if v.is_undefined() {
        return Ok(None);
    }
    Ok(Some(v.truthy()))
}

/// A numberingSystem, calendar or collation option: if it's given, a well-formed name.
fn opt_keyword(vm: &mut Vm, o: Option<Gc>, name: &str) -> Result<Option<String>, Value> {
    let v = opt_text(vm, o, name)?;
    if let Some(s) = &v
        && !s.split('-').all(|p| (3..=8).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric()))
    {
        return Err(vm.range_error(&format!("Invalid {name} : {s}")));
    }
    Ok(v)
}

/// A plain object of resolved options, in their order (undefined ones left out).
fn resolved(vm: &mut Vm, items: Vec<(&str, Value)>) -> R {
    let o = vm.new_object();
    for (k, v) in items {
        if !v.is_undefined() {
            vm.put(o, k, v, ALL);
        }
    }
    Ok(Value::Obj(o))
}

/// formatToParts's list: { type, value } objects (and a unit, or a source, where `extra` gives one).
fn parts_array(vm: &mut Vm, parts: &[(&str, String)], extra: Option<(&str, &dyn Fn(usize, &str) -> Option<String>)>) -> R {
    let mut out = Vec::new();
    for (k, (t, v)) in parts.iter().enumerate() {
        let o = vm.new_object();
        vm.put(o, "type", js_str(t), ALL);
        vm.put(o, "value", js_str(v), ALL);
        if let Some((name, f)) = extra
            && let Some(x) = f(k, t)
        {
            vm.put(o, name, js_str(&x), ALL);
        }
        out.push(Value::Obj(o));
    }
    Ok(Value::Obj(vm.new_array(out)))
}

fn joined(parts: &[(&str, String)]) -> String {
    parts.iter().map(|(_, v)| v.as_str()).collect()
}

// ---------------------------------------------------------------------------------------------- numbers

/// Numbering systems with digits of their own (ECMA-402's simple digit mappings): name, and the code point of its 0
/// (its other digits follow it); hanidec's are HANIDEC.
const NUMBERING: &[(&str, u32)] = &[
    ("adlm", 0x1E950), ("ahom", 0x11730), ("arab", 0x660), ("arabext", 0x6F0), ("bali", 0x1B50), ("beng", 0x9E6),
    ("bhks", 0x11C50), ("brah", 0x11066), ("cakm", 0x11136), ("cham", 0xAA50), ("deva", 0x966), ("diak", 0x11950),
    ("fullwide", 0xFF10), ("gara", 0x10D40), ("gong", 0x11DA0), ("gonm", 0x11D50), ("gujr", 0xAE6), ("gukh", 0x16130),
    ("guru", 0xA66), ("hmng", 0x16B50), ("hmnp", 0x1E140), ("java", 0xA9D0), ("kali", 0xA900), ("kawi", 0x11F50),
    ("khmr", 0x17E0), ("knda", 0xCE6), ("krai", 0x16D70), ("lana", 0x1A80), ("lanatham", 0x1A90), ("laoo", 0xED0),
    ("latn", 0x30), ("lepc", 0x1C40), ("limb", 0x1946), ("mathbold", 0x1D7CE), ("mathdbl", 0x1D7D8),
    ("mathmono", 0x1D7F6), ("mathsanb", 0x1D7EC), ("mathsans", 0x1D7E2), ("mlym", 0xD66), ("modi", 0x11650),
    ("mong", 0x1810), ("mroo", 0x16A60), ("mtei", 0xABF0), ("mymr", 0x1040), ("mymrepka", 0x116DA),
    ("mymrpao", 0x116D0), ("mymrshan", 0x1090), ("mymrtlng", 0xA9F0), ("nagm", 0x1E4F0), ("newa", 0x11450),
    ("nkoo", 0x7C0), ("olck", 0x1C50), ("onao", 0x1E5F1), ("orya", 0xB66), ("osma", 0x104A0), ("outlined", 0x1CCF0),
    ("rohg", 0x10D30), ("saur", 0xA8D0), ("segment", 0x1FBF0), ("shrd", 0x111D0), ("sind", 0x112F0), ("sinh", 0xDE6),
    ("sora", 0x110F0), ("sund", 0x1BB0), ("sunu", 0x11BF0), ("takr", 0x116C0), ("talu", 0x19D0), ("tamldec", 0xBE6),
    ("telu", 0xC66), ("thai", 0xE50), ("tibt", 0xF20), ("tirh", 0x114D0), ("tnsa", 0x16AC0), ("tols", 0x11DE0),
    ("vaii", 0xA620), ("wara", 0x118E0), ("wcho", 0x1E2F0),
];

const HANIDEC: [char; 10] = ['〇', '一', '二', '三', '四', '五', '六', '七', '八', '九'];

/// The numbering systems there are (for -u-nu- and the numberingSystem option).
fn numbering_names() -> Vec<&'static str> {
    NUMBERING.iter().map(|x| x.0).chain(core::iter::once("hanidec")).collect()
}

/// Text's digits written in a numbering system's.
fn in_digits(s: &str, nu: &str) -> String {
    if nu == "latn" {
        return String::from(s);
    }
    let zero = NUMBERING.iter().find(|x| x.0 == nu).map(|x| x.1);
    s.chars()
        .map(|c| match (c.is_ascii_digit(), zero) {
            (true, _) if nu == "hanidec" => HANIDEC[(c as u32 - '0' as u32) as usize],
            (true, Some(z)) => char::from_u32(z + (c as u32 - '0' as u32)).unwrap_or(c),
            _ => c,
        })
        .collect()
}

/// Parts' digits written in a numbering system's.
fn parts_in_digits(parts: Parts, nu: &str) -> Parts {
    if nu == "latn" {
        return parts;
    }
    parts.into_iter().map(|(t, v)| (t, in_digits(&v, nu))).collect()
}

/// A number as it's written in decimal: 0.d₁d₂… × 10^exp (no zeros at either end of the digits; none for 0), and
/// whether it's negative (negative zero too).
#[derive(Clone)]
enum Num {
    NaN,
    Inf(bool),
    Fin(Vec<u8>, i32, bool),
}

/// A decimal literal ("-12.5e3", " 0.1 ") exactly, if the string is one.
fn parse_decimal(s: &str) -> Option<Num> {
    let s = s.trim();
    let (neg, body) = match s.as_bytes().first()? {
        b'-' => (true, &s[1..]),
        b'+' => (false, &s[1..]),
        _ => (false, s),
    };
    let (mant, e) = match body.find(['e', 'E']) {
        Some(i) => {
            let t = &body[i + 1..];
            let t = t.strip_prefix('+').unwrap_or(t);
            if t.is_empty() || !t.trim_start_matches('-').bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            (&body[..i], t.parse::<i32>().ok()?)
        }
        None => (body, 0),
    };
    let (int, frac) = mant.split_once('.').unwrap_or((mant, ""));
    if (int.is_empty() && frac.is_empty()) || !int.bytes().all(|b| b.is_ascii_digit()) || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut digits: Vec<u8> = int.bytes().chain(frac.bytes()).map(|b| b - b'0').collect();
    let mut exp = (int.len() as i32).checked_add(e)?;
    let lead = digits.iter().take_while(|&&d| d == 0).count();
    digits.drain(..lead);
    exp -= lead as i32;
    while digits.last() == Some(&0) {
        digits.pop();
    }
    if digits.is_empty() {
        exp = 0;
    }
    Some(Num::Fin(digits, exp, neg))
}

impl Num {
    fn of(x: f64) -> Num {
        if x.is_nan() {
            return Num::NaN;
        }
        let neg = x < 0.0 || (x == 0.0 && x.is_sign_negative());
        if x.is_infinite() {
            return Num::Inf(neg);
        }
        match parse_decimal(&number_to_string(x.abs())) {
            Some(Num::Fin(d, e, _)) => Num::Fin(d, e, neg),
            _ => Num::NaN,
        }
    }
}

/// ToIntlMathematicalValue: a Number as it's written; a BigInt, or a long string of a decimal number, exactly.
fn num_of(vm: &mut Vm, v: &Value) -> Result<Num, Value> {
    let p = if matches!(v, Value::Obj(_)) { vm.to_primitive(v, Hint::Number)? } else { v.clone() };
    match &p {
        Value::Big(b) => Ok(parse_decimal(&b.to_string_radix(10)).unwrap_or(Num::NaN)),
        Value::Str(s) => {
            let t = s.to_rust();
            match parse_decimal(&t) {
                Some(x) if t.trim().len() > 15 => Ok(x),
                _ => Ok(Num::of(vm.to_number(&p)?)),
            }
        }
        _ => Ok(Num::of(vm.to_number(&p)?)),
    }
}

/// Currencies: code, digits, symbol, narrow symbol, and names (one, other) as en-US writes them.
const CURRENCIES: &[(&str, u32, &str, &str, &str, &str)] = &[
    ("AED", 2, "AED", "AED", "UAE dirham", "UAE dirhams"),
    ("ARS", 2, "ARS", "$", "Argentine peso", "Argentine pesos"),
    ("AUD", 2, "A$", "$", "Australian dollar", "Australian dollars"),
    ("BDT", 2, "BDT", "৳", "Bangladeshi taka", "Bangladeshi takas"),
    ("BHD", 3, "BHD", "BHD", "Bahraini dinar", "Bahraini dinars"),
    ("BRL", 2, "R$", "R$", "Brazilian real", "Brazilian reals"),
    ("CAD", 2, "CA$", "$", "Canadian dollar", "Canadian dollars"),
    ("CHF", 2, "CHF", "CHF", "Swiss franc", "Swiss francs"),
    ("CLP", 0, "CLP", "$", "Chilean peso", "Chilean pesos"),
    ("CNY", 2, "CN¥", "¥", "Chinese yuan", "Chinese yuan"),
    ("COP", 2, "COP", "$", "Colombian peso", "Colombian pesos"),
    ("CZK", 2, "CZK", "Kč", "Czech koruna", "Czech korunas"),
    ("DKK", 2, "DKK", "kr", "Danish krone", "Danish kroner"),
    ("EGP", 2, "EGP", "E£", "Egyptian pound", "Egyptian pounds"),
    ("EUR", 2, "€", "€", "euro", "euros"),
    ("GBP", 2, "£", "£", "British pound", "British pounds"),
    ("HKD", 2, "HK$", "$", "Hong Kong dollar", "Hong Kong dollars"),
    ("HUF", 2, "HUF", "Ft", "Hungarian forint", "Hungarian forints"),
    ("IDR", 2, "IDR", "Rp", "Indonesian rupiah", "Indonesian rupiahs"),
    ("ILS", 2, "₪", "₪", "Israeli new shekel", "Israeli new shekels"),
    ("INR", 2, "₹", "₹", "Indian rupee", "Indian rupees"),
    ("ISK", 0, "ISK", "kr", "Icelandic króna", "Icelandic krónur"),
    ("JOD", 3, "JOD", "JOD", "Jordanian dinar", "Jordanian dinars"),
    ("JPY", 0, "¥", "¥", "Japanese yen", "Japanese yen"),
    ("KES", 2, "KES", "KES", "Kenyan shilling", "Kenyan shillings"),
    ("KRW", 0, "₩", "₩", "South Korean won", "South Korean won"),
    ("KWD", 3, "KWD", "KWD", "Kuwaiti dinar", "Kuwaiti dinars"),
    ("MXN", 2, "MX$", "$", "Mexican peso", "Mexican pesos"),
    ("MYR", 2, "MYR", "RM", "Malaysian ringgit", "Malaysian ringgits"),
    ("NGN", 2, "NGN", "₦", "Nigerian naira", "Nigerian nairas"),
    ("NOK", 2, "NOK", "kr", "Norwegian krone", "Norwegian kroner"),
    ("NZD", 2, "NZ$", "$", "New Zealand dollar", "New Zealand dollars"),
    ("OMR", 3, "OMR", "OMR", "Omani rial", "Omani rials"),
    ("PHP", 2, "₱", "₱", "Philippine peso", "Philippine pesos"),
    ("PKR", 2, "PKR", "Rs", "Pakistani rupee", "Pakistani rupees"),
    ("PLN", 2, "PLN", "zł", "Polish zloty", "Polish zlotys"),
    ("QAR", 2, "QAR", "QAR", "Qatari riyal", "Qatari riyals"),
    ("RUB", 2, "RUB", "₽", "Russian ruble", "Russian rubles"),
    ("SAR", 2, "SAR", "SAR", "Saudi riyal", "Saudi riyals"),
    ("SEK", 2, "SEK", "kr", "Swedish krona", "Swedish kronor"),
    ("SGD", 2, "SGD", "$", "Singapore dollar", "Singapore dollars"),
    ("THB", 2, "THB", "฿", "Thai baht", "Thai baht"),
    ("TRY", 2, "TRY", "₺", "Turkish lira", "Turkish Lira"),
    ("TWD", 2, "NT$", "$", "New Taiwan dollar", "New Taiwan dollars"),
    ("UAH", 2, "UAH", "₴", "Ukrainian hryvnia", "Ukrainian hryvnias"),
    ("USD", 2, "$", "$", "US dollar", "US dollars"),
    ("VND", 0, "₫", "₫", "Vietnamese dong", "Vietnamese dong"),
    ("XAF", 0, "FCFA", "FCFA", "Central African CFA franc", "Central African CFA francs"),
    ("XOF", 0, "F\u{202f}CFA", "F\u{202f}CFA", "West African CFA franc", "West African CFA francs"),
    ("ZAR", 2, "ZAR", "R", "South African rand", "South African rand"),
];

/// Units ECMA-402 has: identifier, short (one, other), narrow, long (one, other).
const UNITS: &[(&str, &str, &str, &str, &str, &str)] = &[
    ("acre", "ac", "ac", "ac", "acre", "acres"),
    ("bit", "bit", "bit", "bit", "bit", "bits"),
    ("byte", "byte", "byte", "B", "byte", "bytes"),
    ("celsius", "°C", "°C", "°C", "degree Celsius", "degrees Celsius"),
    ("centimeter", "cm", "cm", "cm", "centimeter", "centimeters"),
    ("day", "day", "days", "d", "day", "days"),
    ("degree", "deg", "deg", "°", "degree", "degrees"),
    ("fahrenheit", "°F", "°F", "°", "degree Fahrenheit", "degrees Fahrenheit"),
    ("fluid-ounce", "fl oz", "fl oz", "fl oz", "fluid ounce", "fluid ounces"),
    ("foot", "ft", "ft", "′", "foot", "feet"),
    ("gallon", "gal", "gal", "gal", "gallon", "gallons"),
    ("gigabit", "Gb", "Gb", "Gb", "gigabit", "gigabits"),
    ("gigabyte", "GB", "GB", "GB", "gigabyte", "gigabytes"),
    ("gram", "g", "g", "g", "gram", "grams"),
    ("hectare", "ha", "ha", "ha", "hectare", "hectares"),
    ("hour", "hr", "hr", "h", "hour", "hours"),
    ("inch", "in", "in", "″", "inch", "inches"),
    ("kilobit", "kb", "kb", "kb", "kilobit", "kilobits"),
    ("kilobyte", "kB", "kB", "kB", "kilobyte", "kilobytes"),
    ("kilogram", "kg", "kg", "kg", "kilogram", "kilograms"),
    ("kilometer", "km", "km", "km", "kilometer", "kilometers"),
    ("liter", "L", "L", "L", "liter", "liters"),
    ("megabit", "Mb", "Mb", "Mb", "megabit", "megabits"),
    ("megabyte", "MB", "MB", "MB", "megabyte", "megabytes"),
    ("meter", "m", "m", "m", "meter", "meters"),
    ("microsecond", "μs", "μs", "μs", "microsecond", "microseconds"),
    ("mile", "mi", "mi", "mi", "mile", "miles"),
    ("mile-scandinavian", "smi", "smi", "smi", "mile-scandinavian", "miles-scandinavian"),
    ("milliliter", "mL", "mL", "mL", "milliliter", "milliliters"),
    ("millimeter", "mm", "mm", "mm", "millimeter", "millimeters"),
    ("millisecond", "ms", "ms", "ms", "millisecond", "milliseconds"),
    ("minute", "min", "min", "m", "minute", "minutes"),
    ("month", "mth", "mths", "m", "month", "months"),
    ("nanosecond", "ns", "ns", "ns", "nanosecond", "nanoseconds"),
    ("ounce", "oz", "oz", "oz", "ounce", "ounces"),
    ("percent", "%", "%", "%", "percent", "percent"),
    ("petabyte", "PB", "PB", "PB", "petabyte", "petabytes"),
    ("pound", "lb", "lb", "lb", "pound", "pounds"),
    ("second", "sec", "sec", "s", "second", "seconds"),
    ("stone", "st", "st", "st", "stone", "stones"),
    ("terabit", "Tb", "Tb", "Tb", "terabit", "terabits"),
    ("terabyte", "TB", "TB", "TB", "terabyte", "terabytes"),
    ("week", "wk", "wks", "w", "week", "weeks"),
    ("yard", "yd", "yd", "yd", "yard", "yards"),
    ("year", "yr", "yrs", "y", "year", "years"),
];

/// The compound units CLDR names (others are written "a/b", "a per b").
const NAMED_COMPOUNDS: &[(&str, &str, &str, &str, &str, &str)] = &[
    ("kilometer-per-hour", "km/h", "km/h", "km/h", "kilometer per hour", "kilometers per hour"),
    ("liter-per-kilometer", "L/km", "L/km", "L/km", "liter per kilometer", "liters per kilometer"),
    ("meter-per-second", "m/s", "m/s", "m/s", "meter per second", "meters per second"),
    ("mile-per-gallon", "mpg", "mpg", "mpg", "mile per gallon", "miles per gallon"),
    ("mile-per-hour", "mph", "mph", "mph", "mile per hour", "miles per hour"),
];

type UnitNames = (&'static str, &'static str, &'static str, &'static str, &'static str, &'static str);

fn simple_unit(u: &str) -> Option<&'static UnitNames> {
    UNITS.iter().find(|x| x.0 == u)
}

fn well_formed_unit(u: &str) -> bool {
    match u.split_once("-per-") {
        Some((a, b)) => simple_unit(a).is_some() && simple_unit(b).is_some(),
        None => simple_unit(u).is_some(),
    }
}

/// A unit's words for a number (one: as en-US writes 1), at a width (0 short, 1 long, 2 narrow).
fn unit_words(unit: &str, display: u8, one: bool) -> String {
    let pick = |x: &UnitNames| -> String {
        String::from(match (display, one) {
            (1, true) => x.4,
            (1, false) => x.5,
            (2, _) => x.3,
            (_, true) => x.1,
            _ => x.2,
        })
    };
    if let Some(x) = simple_unit(unit).or_else(|| NAMED_COMPOUNDS.iter().find(|x| x.0 == unit)) {
        return pick(x);
    }
    if let Some((a, b)) = unit.split_once("-per-")
        && let (Some(xa), Some(xb)) = (simple_unit(a), simple_unit(b))
    {
        return match display {
            1 => format!("{} per {}", pick(xa), xb.4),
            2 => format!("{}/{}", xa.3, xb.3),
            _ => format!("{}/{}", pick(xa), xb.1),
        };
    }
    String::from(unit)
}

#[derive(Clone)]
pub struct NumberOpts {
    /// 0 decimal, 1 percent, 2 currency, 3 unit.
    style: u8,
    currency: String,
    /// 0 symbol, 1 narrowSymbol, 2 code, 3 name.
    currency_display: u8,
    accounting: bool,
    unit: String,
    /// 0 short, 1 long, 2 narrow.
    unit_display: u8,
    min_int: u32,
    min_frac: u32,
    max_frac: u32,
    /// The least and most significant digits, if they round it.
    sig: Option<(u32, u32)>,
    /// When both fraction and significant digits are given: 0 auto (significant), 1 morePrecision, 2 lessPrecision.
    priority: u8,
    increment: u32,
    /// The rounding mode: its place in ROUNDING_MODES.
    mode: u8,
    /// trailingZeroDisplay: stripIfInteger.
    strip_integer: bool,
    /// 0 false, 1 auto, 2 always, 3 min2.
    grouping: u8,
    /// 0 standard, 1 scientific, 2 engineering, 3 compact.
    notation: u8,
    compact_long: bool,
    /// 0 auto, 1 never, 2 always, 3 exceptZero, 4 negative.
    sign: u8,
    /// Its numbering system.
    nu: String,
}

const STYLES: &[&str] = &["decimal", "percent", "currency", "unit"];
const CURRENCY_DISPLAYS: &[&str] = &["symbol", "narrowSymbol", "code", "name"];
const UNIT_DISPLAYS: &[&str] = &["short", "long", "narrow"];
const NOTATIONS: &[&str] = &["standard", "scientific", "engineering", "compact"];
const SIGNS: &[&str] = &["auto", "never", "always", "exceptZero", "negative"];
const ROUNDING_MODES: &[&str] = &["ceil", "floor", "expand", "trunc", "halfCeil", "halfFloor", "halfExpand", "halfTrunc", "halfEven"];
const PRIORITIES: &[&str] = &["auto", "morePrecision", "lessPrecision"];
const INCREMENTS: [u32; 15] = [1, 2, 5, 10, 20, 25, 50, 100, 200, 250, 500, 1000, 2000, 2500, 5000];

impl NumberOpts {
    /// Plain numbers with these fraction digits, grouped (plural rules' and relative times' numbers).
    fn plain(min_frac: u32, max_frac: u32) -> NumberOpts {
        NumberOpts {
            style: 0,
            currency: String::new(),
            currency_display: 0,
            accounting: false,
            unit: String::new(),
            unit_display: 0,
            min_int: 1,
            min_frac,
            max_frac,
            sig: None,
            priority: 0,
            increment: 1,
            mode: 6,
            strip_integer: false,
            grouping: 1,
            notation: 0,
            compact_long: false,
            sign: 0,
            nu: String::from("latn"),
        }
    }
}

/// SetNumberFormatDigitOptions: the digits' options read into `o`, with these default fraction digits (and compact
/// notation's own default: two significant digits or none after the point, whichever is more precise).
fn digit_opts(vm: &mut Vm, opts: Option<Gc>, o: &mut NumberOpts, def_min: u32, def_max: u32, compact: bool) -> Result<(), Value> {
    o.min_int = opt_num(vm, opts, "minimumIntegerDigits", 1, 21)?.unwrap_or(1);
    let mnfd = get(vm, opts, "minimumFractionDigits")?;
    let mxfd = get(vm, opts, "maximumFractionDigits")?;
    let mnsd = get(vm, opts, "minimumSignificantDigits")?;
    let mxsd = get(vm, opts, "maximumSignificantDigits")?;
    let increment = opt_num(vm, opts, "roundingIncrement", 1, 5000)?.unwrap_or(1);
    if !INCREMENTS.contains(&increment) {
        return Err(vm.range_error("roundingIncrement value is out of range."));
    }
    o.increment = increment;
    o.mode = opt_str(vm, opts, "roundingMode", ROUNDING_MODES, Some(6))?.unwrap_or(6) as u8;
    let priority = opt_str(vm, opts, "roundingPriority", PRIORITIES, Some(0))?.unwrap_or(0) as u8;
    o.strip_integer = opt_str(vm, opts, "trailingZeroDisplay", &["auto", "stripIfInteger"], Some(0))? == Some(1);
    let has_sd = !mnsd.is_undefined() || !mxsd.is_undefined();
    let has_fd = !mnfd.is_undefined() || !mxfd.is_undefined();
    let (mut need_sd, mut need_fd) = (true, true);
    if priority == 0 {
        need_sd = has_sd;
        if need_sd || (!has_fd && compact) {
            need_fd = false;
        }
    }
    o.sig = None;
    if need_sd {
        o.sig = Some(if has_sd {
            let mn = number_option(vm, &mnsd, 1, 21, "minimumSignificantDigits")?.unwrap_or(1);
            let mx = number_option(vm, &mxsd, mn, 21, "maximumSignificantDigits")?.unwrap_or(21);
            (mn, mx)
        } else {
            (1, 21)
        });
    }
    if need_fd {
        let mn = number_option(vm, &mnfd, 0, 100, "minimumFractionDigits")?;
        let mx = number_option(vm, &mxfd, 0, 100, "maximumFractionDigits")?;
        (o.min_frac, o.max_frac) = match (mn, mx) {
            (None, Some(mx)) => (def_min.min(mx), mx),
            (Some(mn), None) => (mn, def_max.max(mn)),
            (Some(mn), Some(mx)) if mn > mx => return Err(vm.range_error("maximumFractionDigits value is out of range.")),
            (Some(mn), Some(mx)) => (mn, mx),
            (None, None) => (def_min, def_max),
        };
    }
    if !need_sd && !need_fd {
        (o.min_frac, o.max_frac, o.sig, o.priority) = (0, 0, Some((1, 2)), 1);
    } else {
        o.priority = priority;
    }
    if increment != 1 {
        if o.sig.is_some() {
            return Err(vm.type_error("roundingIncrement can only be used with fraction digits"));
        }
        if o.min_frac != o.max_frac {
            return Err(vm.range_error("maximumFractionDigits must equal minimumFractionDigits with roundingIncrement"));
        }
    }
    Ok(())
}

/// A NumberFormat's options, read in ECMA-402's order.
fn number_opts(vm: &mut Vm, o: Option<Gc>, res: &mut Resolution) -> Result<NumberOpts, Value> {
    opt_str(vm, o, "localeMatcher", MATCHERS, Some(1))?;
    let nu_option = opt_keyword(vm, o, "numberingSystem")?;
    let nu = res.key("nu", nu_option.as_deref(), &numbering_names()).unwrap_or_else(|| String::from("latn"));
    let style = opt_str(vm, o, "style", STYLES, Some(0))?.unwrap_or(0) as u8;
    let currency = opt_text(vm, o, "currency")?;
    match &currency {
        None if style == 2 => return Err(vm.type_error("Currency code is required with currency style.")),
        Some(c) if c.len() != 3 || !c.bytes().all(|b| b.is_ascii_alphabetic()) => return Err(vm.range_error(&format!("Invalid currency code : {c}"))),
        _ => {}
    }
    let currency_display = opt_str(vm, o, "currencyDisplay", CURRENCY_DISPLAYS, Some(0))?.unwrap_or(0) as u8;
    let accounting = opt_str(vm, o, "currencySign", &["standard", "accounting"], Some(0))? == Some(1);
    let unit = opt_text(vm, o, "unit")?;
    match &unit {
        None if style == 3 => return Err(vm.type_error("Unit is required with unit style.")),
        Some(u) if !well_formed_unit(u) => return Err(vm.range_error(&format!("Invalid unit argument for Intl.NumberFormat() '{u}'"))),
        _ => {}
    }
    let unit_display = opt_str(vm, o, "unitDisplay", UNIT_DISPLAYS, Some(0))?.unwrap_or(0) as u8;
    let currency = currency.map(|c| c.to_ascii_uppercase()).unwrap_or_default();
    let notation = opt_str(vm, o, "notation", NOTATIONS, Some(0))?.unwrap_or(0) as u8;
    let (def_min, def_max) = match style {
        2 if notation == 0 => {
            let d = CURRENCIES.iter().find(|c| c.0 == currency).map_or(2, |c| c.1);
            (d, d)
        }
        1 => (0, 0),
        _ => (0, 3),
    };
    let mut opts = NumberOpts { style, currency, currency_display, accounting, unit: unit.unwrap_or_default(), unit_display, notation, nu, ..NumberOpts::plain(0, 3) };
    digit_opts(vm, o, &mut opts, def_min, def_max, notation == 3)?;
    opts.compact_long = opt_str(vm, o, "compactDisplay", &["short", "long"], Some(0))? == Some(1);
    let default_grouping = if notation == 3 { 3 } else { 1 };
    let g = get(vm, o, "useGrouping")?;
    opts.grouping = match &g {
        Value::Undefined => default_grouping,
        Value::Bool(true) => 2,
        v if !v.truthy() => 0,
        v => match vm.to_string(v)?.to_rust().as_str() {
            "true" | "false" => default_grouping,
            "always" => 2,
            "auto" => 1,
            "min2" => 3,
            s => return Err(vm.range_error(&format!("Value {s} out of range for Intl.NumberFormat options property useGrouping"))),
        },
    };
    opts.sign = opt_str(vm, o, "signDisplay", SIGNS, Some(0))?.unwrap_or(0) as u8;
    Ok(opts)
}

fn number_format_new(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let p = vm.proto_from_constructor(nt, I::IntlNumberFormatProto)?;
    let mut res = resolution(vm, &arg(args, 0))?;
    let o = options(vm, &arg(args, 1))?;
    let opts = number_opts(vm, o, &mut res)?;
    Ok(alloc_intl(vm, p, res.locale(), IntlData::Number(Box::new(opts))))
}

/// Whether to round up (away from zero), as a rounding mode says, given what's cut off: more than half a unit, just
/// half, or anything; and whether the last digit kept is odd.
fn rounds_up(mode: u8, neg: bool, above: bool, half: bool, any: bool, odd: bool) -> bool {
    any && match mode {
        0 => !neg,
        1 => neg,
        2 => true,
        3 => false,
        4 => above || (half && !neg),
        5 => above || (half && neg),
        7 => above,
        8 => above || (half && odd),
        _ => above || half,
    }
}

/// Digits rounded to their first `keep` (0 or less: to a unit above the first digit), the way the mode says.
fn round_to(d: &mut Vec<u8>, exp: &mut i32, keep: i32, mode: u8, neg: bool) {
    if keep >= d.len() as i32 {
        return;
    }
    // What's cut off is never 0 (the digits end in no zeros).
    let (above, half) = if keep < 0 {
        (false, false)
    } else {
        let rest = &d[keep as usize..];
        (rest[0] > 5 || (rest[0] == 5 && rest.len() > 1), rest[0] == 5 && rest.len() == 1)
    };
    let odd = keep > 0 && d[keep as usize - 1] % 2 == 1;
    let up = rounds_up(mode, neg, above, half, true, odd);
    if keep <= 0 {
        let unit = *exp - keep;
        d.clear();
        *exp = if up { unit + 1 } else { 0 };
        if up {
            d.push(1);
        }
        return;
    }
    d.truncate(keep as usize);
    if up {
        let mut i = d.len();
        loop {
            if i == 0 {
                d.insert(0, 1);
                *exp += 1;
                break;
            }
            i -= 1;
            if d[i] == 9 {
                d[i] = 0;
            } else {
                d[i] += 1;
                break;
            }
        }
    }
    while d.last() == Some(&0) {
        d.pop();
    }
    if d.is_empty() {
        *exp = 0;
    }
}

/// Rounded to a multiple of `inc` units of the `frac`-th place after the point (roundingIncrement: 5 cents), in a
/// double, which holds the numbers it's used for.
fn round_increment(d: &[u8], exp: i32, frac: u32, inc: u32, mode: u8, neg: bool) -> (Vec<u8>, i32) {
    let digits: String = d.iter().map(|x| (b'0' + x) as char).collect();
    let scaled = format!("0.{}e{}", if digits.is_empty() { "0" } else { &digits }, exp as i64 + frac as i64);
    let q = crate::value::string_to_number(&JsStr::from_str(&scaled)) / inc as f64;
    if !q.is_finite() || q > 9.0e15 {
        let (mut d, mut e) = (d.to_vec(), exp);
        round_to(&mut d, &mut e, exp + frac as i32, mode, neg);
        return (d, e);
    }
    let fl = crate::math::floor(q);
    let f = q - fl;
    let up = rounds_up(mode, neg, f > 0.5, f == 0.5, f > 0.0, fl % 2.0 == 1.0);
    let m = (fl as u64 + up as u64) * inc as u64;
    match parse_decimal(&format!("{m}e-{frac}")) {
        Some(Num::Fin(d, e, _)) => (d, e),
        _ => (Vec::new(), 0),
    }
}

/// A finite number's integer and fraction digits (of its magnitude), rounded and padded as the options say.
fn rounded(d0: &[u8], e0: i32, neg: bool, o: &NumberOpts) -> (String, String) {
    let by_frac = || {
        if o.increment > 1 {
            return round_increment(d0, e0, o.max_frac, o.increment, o.mode, neg);
        }
        let (mut d, mut e) = (d0.to_vec(), e0);
        round_to(&mut d, &mut e, e0 + o.max_frac as i32, o.mode, neg);
        (d, e)
    };
    let by_sig = |most: u32| {
        let (mut d, mut e) = (d0.to_vec(), e0);
        round_to(&mut d, &mut e, most as i32, o.mode, neg);
        (d, e)
    };
    let ((d, e), min_frac, min_sig) = match o.sig {
        None => (by_frac(), o.min_frac, 0),
        Some((mn, mx)) => {
            // (morePrecision: whichever rounds at the lower place; lessPrecision: the higher.)
            let fixed_finer = -(o.max_frac as i32) <= e0 - mx as i32;
            let fixed = match o.priority {
                1 => fixed_finer,
                2 => !fixed_finer,
                _ => false,
            };
            if fixed { (by_frac(), o.min_frac, 0) } else { (by_sig(mx), 0, mn) }
        }
    };
    let mut int = String::new();
    if e <= 0 {
        int.push('0');
    } else {
        for i in 0..e as usize {
            int.push((b'0' + d.get(i).copied().unwrap_or(0)) as char);
        }
    }
    let mut frac = String::new();
    for _ in 0..(-e).max(0) {
        frac.push('0');
    }
    for &x in d.iter().skip(e.max(0) as usize) {
        frac.push((b'0' + x) as char);
    }
    let mut want = min_frac as usize;
    if min_sig > 0 {
        let shown = if d.is_empty() {
            1 + frac.len()
        } else if e > 0 {
            e as usize + frac.len()
        } else {
            d.len()
        };
        if min_sig as usize > shown {
            want = want.max(frac.len() + min_sig as usize - shown);
        }
    }
    while frac.len() < want {
        frac.push('0');
    }
    if o.strip_integer && frac.bytes().all(|b| b == b'0') {
        frac.clear();
    }
    while int.len() < o.min_int as usize {
        int.insert(0, '0');
    }
    (int, frac)
}

/// The integer part in groups of three (en-US's commas), as the grouping option says; then the point and fraction.
fn push_number(int: &str, frac: &str, grouping: u8, parts: &mut Parts) {
    let group = match grouping {
        0 => false,
        3 => int.len() >= 5,
        _ => int.len() >= 4,
    };
    if group {
        let first = match int.len() % 3 {
            0 => 3,
            r => r,
        };
        parts.push(("integer", String::from(&int[..first])));
        let mut at = first;
        while at < int.len() {
            parts.push(("group", String::from(",")));
            parts.push(("integer", String::from(&int[at..at + 3])));
            at += 3;
        }
    } else {
        parts.push(("integer", String::from(int)));
    }
    if !frac.is_empty() {
        parts.push(("decimal", String::from(".")));
        parts.push(("fraction", String::from(frac)));
    }
}

fn is_zero(int: &str, frac: &str) -> bool {
    int.bytes().all(|b| b == b'0') && frac.bytes().all(|b| b == b'0')
}

/// Whether a number's written digits are English's "one" (1, with nothing after its point).
fn is_one(int: &str, frac: &str) -> bool {
    int.trim_start_matches('0') == "1" && frac.is_empty()
}

/// A finite number's body (its digits, and its exponent or compact suffix): whether it shows as zero, and as one.
fn body_parts(d: &[u8], e: i32, neg: bool, o: &NumberOpts, parts: &mut Parts) -> (bool, bool) {
    match o.notation {
        1 | 2 => {
            let (mut d, mut e) = (d.to_vec(), e);
            loop {
                let mut e10 = if d.is_empty() { 0 } else { e - 1 };
                if o.notation == 2 {
                    e10 = e10.div_euclid(3) * 3;
                }
                let places = e - e10;
                let (int, frac) = rounded(&d, places, neg, o);
                // (9.9995 rounds to 10.000: it's 1E1.)
                if !d.is_empty() && int.trim_start_matches('0').len() > places.max(1) as usize {
                    d = vec![1];
                    e += 1;
                    continue;
                }
                push_number(&int, &frac, 0, parts);
                parts.push(("exponentSeparator", String::from("E")));
                if e10 < 0 {
                    parts.push(("exponentMinusSign", String::from("-")));
                }
                parts.push(("exponentInteger", format!("{}", e10.unsigned_abs())));
                return (is_zero(&int, &frac), false);
            }
        }
        3 => {
            const SHORT: [&str; 4] = ["K", "M", "B", "T"];
            const LONG: [&str; 4] = ["thousand", "million", "billion", "trillion"];
            let mut k: usize = if !d.is_empty() && e > 3 { (((e - 1) / 3) as usize).min(4) } else { 0 };
            let (int, frac) = loop {
                let (int, frac) = rounded(d, e - 3 * k as i32, neg, o);
                // (999,999 rounds to 1000K: it's 1M.)
                if k < 4 && int.trim_start_matches('0').len() > 3 && (k > 0 || e == 3) {
                    k += 1;
                    continue;
                }
                break (int, frac);
            };
            push_number(&int, &frac, o.grouping, parts);
            if k > 0 {
                if o.compact_long {
                    parts.push(("literal", String::from(" ")));
                    parts.push(("compact", String::from(LONG[k - 1])));
                } else {
                    parts.push(("compact", String::from(SHORT[k - 1])));
                }
            }
            (is_zero(&int, &frac), k == 0 && is_one(&int, &frac))
        }
        _ => {
            let (int, frac) = rounded(d, e, neg, o);
            push_number(&int, &frac, o.grouping, parts);
            (is_zero(&int, &frac), is_one(&int, &frac))
        }
    }
}

/// A number formatted as the options say, as its parts.
fn format_parts(num: &Num, o: &NumberOpts) -> Parts {
    let mut body: Parts = Vec::new();
    let (neg, zero, one) = match num {
        Num::NaN => {
            body.push(("nan", String::from("NaN")));
            (false, true, false)
        }
        Num::Inf(neg) => {
            body.push(("infinity", String::from("∞")));
            (*neg, false, false)
        }
        Num::Fin(d, e, neg) => {
            // (A percent is the number a hundred times over: its point two places on, exactly.)
            let e = if o.style == 1 && !d.is_empty() { e + 2 } else { *e };
            let (zero, one) = body_parts(d, e, *neg, o, &mut body);
            (*neg, zero, one)
        }
    };
    let sign = match o.sign {
        1 => None,
        2 => Some(neg),
        3 if zero => None,
        3 => Some(neg),
        4 if neg && !zero => Some(true),
        4 => None,
        _ if neg => Some(true),
        _ => None,
    };
    let accounting = o.style == 2 && o.accounting && sign == Some(true);
    let mut parts: Parts = Vec::new();
    if accounting {
        parts.push(("literal", String::from("(")));
    } else if let Some(minus) = sign {
        parts.push(if minus { ("minusSign", String::from("-")) } else { ("plusSign", String::from("+")) });
    }
    match o.style {
        2 => {
            let c = CURRENCIES.iter().find(|c| c.0 == o.currency);
            match o.currency_display {
                3 => {
                    parts.extend(body);
                    parts.push(("literal", String::from(" ")));
                    let name = match c {
                        Some(c) => String::from(if one { c.4 } else { c.5 }),
                        None => o.currency.clone(),
                    };
                    parts.push(("currency", name));
                }
                2 => {
                    parts.push(("currency", o.currency.clone()));
                    parts.push(("literal", String::from("\u{a0}")));
                    parts.extend(body);
                }
                d => {
                    let sym = match c {
                        Some(c) => String::from(if d == 1 { c.3 } else { c.2 }),
                        None => o.currency.clone(),
                    };
                    let spaced = sym.chars().last().is_some_and(|ch| ch.is_alphabetic());
                    parts.push(("currency", sym));
                    if spaced {
                        parts.push(("literal", String::from("\u{a0}")));
                    }
                    parts.extend(body);
                }
            }
        }
        1 => {
            parts.extend(body);
            parts.push(("percentSign", String::from("%")));
        }
        3 => {
            parts.extend(body);
            let tight = o.unit_display == 2 || (o.unit_display == 0 && matches!(o.unit.as_str(), "percent" | "celsius" | "fahrenheit"));
            if !tight {
                parts.push(("literal", String::from(" ")));
            }
            parts.push(("unit", unit_words(&o.unit, o.unit_display, one)));
        }
        _ => parts.extend(body),
    }
    if accounting {
        parts.push(("literal", String::from(")")));
    }
    parts_in_digits(parts, &o.nu)
}

fn this_number_format(vm: &mut Vm, this: &Value, what: &str) -> Result<NumberOpts, Value> {
    match data_of(vm, this) {
        Some(IntlData::Number(o)) => Ok((**o).clone()),
        _ => Err(incompatible(vm, &format!("NumberFormat.prototype.{what}"))),
    }
}

fn nf_format_getter(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    this_number_format(vm, this, "format")?;
    bound(vm, this, 1, nf_format_bound)
}

fn nf_format_bound(vm: &mut Vm, _this: &Value, args: &[Value], f: Gc) -> R {
    let nf = vm.data(f);
    let o = this_number_format(vm, &nf, "format")?;
    let x = num_of(vm, &arg(args, 0))?;
    Ok(js_str(&joined(&format_parts(&x, &o))))
}

fn nf_format_to_parts(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let o = this_number_format(vm, this, "formatToParts")?;
    let x = num_of(vm, &arg(args, 0))?;
    parts_array(vm, &format_parts(&x, &o), None)
}

/// formatRange's two numbers' parts, and whether they're written the same.
fn number_range(vm: &mut Vm, this: &Value, args: &[Value], what: &str) -> Result<(Parts, Parts, bool), Value> {
    let o = this_number_format(vm, this, what)?;
    if arg(args, 0).is_undefined() || arg(args, 1).is_undefined() {
        return Err(vm.type_error("start or end is undefined"));
    }
    let a = num_of(vm, &arg(args, 0))?;
    let b = num_of(vm, &arg(args, 1))?;
    if matches!(a, Num::NaN) || matches!(b, Num::NaN) {
        return Err(vm.range_error("start or end is NaN"));
    }
    let (pa, pb) = (format_parts(&a, &o), format_parts(&b, &o));
    let same = joined(&pa) == joined(&pb);
    Ok((pa, pb, same))
}

/// "3–5", "$3 – $5", "+$2.90–3.10", "3–5 km", "~5": a range's parts, and where each came from (0 start, 1 end, 2
/// shared). As ICU does: an affix the two share is written once if it's more than one character, and the dash has
/// spaces round it unless digits are on both its sides.
fn range_parts(pa: Parts, pb: Parts, same: bool) -> (Parts, Vec<u8>) {
    if same {
        let mut parts = vec![("approximatelySign", String::from("~"))];
        let n = pa.len();
        parts.extend(pa);
        return (parts, vec![2; n + 1]);
    }
    let numeric = |t: &str| matches!(t, "integer" | "group" | "decimal" | "fraction" | "exponentSeparator" | "exponentMinusSign" | "exponentInteger" | "nan" | "infinity");
    let span = |p: &Parts| {
        let first = p.iter().position(|(t, _)| numeric(t)).unwrap_or(0);
        let last = p.iter().rposition(|(t, _)| numeric(t)).map_or(p.len(), |k| k + 1);
        (first, last)
    };
    let ((fa, la), (fb, lb)) = (span(&pa), span(&pb));
    let affix = |p: &Parts, from: usize, to: usize| -> String { p[from..to].iter().map(|(_, v)| v.as_str()).collect() };
    let (prefix_a, suffix_a) = (affix(&pa, 0, fa), affix(&pa, la, pa.len()));
    let (prefix_b, suffix_b) = (affix(&pb, 0, fb), affix(&pb, lb, pb.len()));
    let shared_chars = prefix_a.chars().chain(suffix_a.chars()).filter(|c| !c.is_whitespace()).count();
    if prefix_a == prefix_b && suffix_a == suffix_b && shared_chars > 1 {
        let mut parts: Parts = pa[..fa].to_vec();
        let mut from = vec![2u8; fa];
        parts.extend(pa[fa..la].iter().cloned());
        from.extend(core::iter::repeat_n(0u8, la - fa));
        parts.push(("literal", String::from("–")));
        from.push(2);
        parts.extend(pb[fb..lb].iter().cloned());
        from.extend(core::iter::repeat_n(1u8, lb - fb));
        parts.extend(pb[lb..].iter().cloned());
        from.extend(core::iter::repeat_n(2u8, pb.len() - lb));
        return (parts, from);
    }
    let digit_end = pa.last().is_some_and(|(_, v)| v.ends_with(|c: char| c.is_ascii_digit()));
    let digit_start = pb.first().is_some_and(|(_, v)| v.starts_with(|c: char| c.is_ascii_digit()));
    let dash = if digit_end && digit_start { "–" } else { " – " };
    let mut from = vec![0u8; pa.len()];
    from.push(2);
    from.extend(core::iter::repeat_n(1u8, pb.len()));
    let mut parts = pa;
    parts.push(("literal", String::from(dash)));
    parts.extend(pb);
    (parts, from)
}

fn nf_format_range(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (pa, pb, same) = number_range(vm, this, args, "formatRange")?;
    Ok(js_str(&joined(&range_parts(pa, pb, same).0)))
}

fn nf_format_range_to_parts(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (pa, pb, same) = number_range(vm, this, args, "formatRangeToParts")?;
    let (parts, from) = range_parts(pa, pb, same);
    let source = |k: usize, _t: &str| Some(String::from(["startRange", "endRange", "shared"][from[k] as usize]));
    parts_array(vm, &parts, Some(("source", &source)))
}

/// The digit options as resolvedOptions gives them.
fn digit_items(o: &NumberOpts, items: &mut Vec<(&str, Value)>) {
    items.push(("minimumIntegerDigits", Value::Num(o.min_int as f64)));
    if o.sig.is_none() || o.priority != 0 {
        items.push(("minimumFractionDigits", Value::Num(o.min_frac as f64)));
        items.push(("maximumFractionDigits", Value::Num(o.max_frac as f64)));
    }
    if let Some((mn, mx)) = o.sig {
        items.push(("minimumSignificantDigits", Value::Num(mn as f64)));
        items.push(("maximumSignificantDigits", Value::Num(mx as f64)));
    }
}

fn rounding_items(o: &NumberOpts, items: &mut Vec<(&str, Value)>) {
    items.push(("roundingIncrement", Value::Num(o.increment as f64)));
    items.push(("roundingMode", js_str(ROUNDING_MODES[o.mode as usize])));
    items.push(("roundingPriority", js_str(PRIORITIES[o.priority as usize])));
    items.push(("trailingZeroDisplay", js_str(if o.strip_integer { "stripIfInteger" } else { "auto" })));
}

fn nf_resolved(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let o = this_number_format(vm, this, "resolvedOptions")?;
    let locale = locale_of(vm, this);
    let mut items: Vec<(&str, Value)> = vec![("locale", js_str(&locale)), ("numberingSystem", js_str(&o.nu)), ("style", js_str(STYLES[o.style as usize]))];
    if o.style == 2 {
        items.push(("currency", js_str(&o.currency)));
        items.push(("currencyDisplay", js_str(CURRENCY_DISPLAYS[o.currency_display as usize])));
        items.push(("currencySign", js_str(if o.accounting { "accounting" } else { "standard" })));
    }
    if o.style == 3 {
        items.push(("unit", js_str(&o.unit)));
        items.push(("unitDisplay", js_str(UNIT_DISPLAYS[o.unit_display as usize])));
    }
    digit_items(&o, &mut items);
    items.push((
        "useGrouping",
        match o.grouping {
            0 => Value::Bool(false),
            2 => js_str("always"),
            3 => js_str("min2"),
            _ => js_str("auto"),
        },
    ));
    items.push(("notation", js_str(NOTATIONS[o.notation as usize])));
    if o.notation == 3 {
        items.push(("compactDisplay", js_str(if o.compact_long { "long" } else { "short" })));
    }
    items.push(("signDisplay", js_str(SIGNS[o.sign as usize])));
    rounding_items(&o, &mut items);
    resolved(vm, items)
}

/// Number.prototype.toLocaleString and BigInt.prototype.toLocaleString: as a NumberFormat of these locales and
/// options formats the value.
pub fn number_to_locale(vm: &mut Vm, v: &Value, locales_v: &Value, options_v: &Value) -> R {
    let mut res = resolution(vm, locales_v)?;
    let o = options(vm, options_v)?;
    let opts = number_opts(vm, o, &mut res)?;
    let x = num_of(vm, v)?;
    Ok(js_str(&joined(&format_parts(&x, &opts))))
}

// ------------------------------------------------------------------------------------------------ time zones

/// Time zones by name: their standard offset (minutes east of UTC), their daylight saving rule (0 none, 1 the
/// United States' and Canada's, 2 the European Union's and Britain's, 3 south-east Australia's, 4 New Zealand's),
/// and their names (standard, daylight) as en-US writes them ("": none, written as an offset from GMT).
const ZONES: &[(&str, i32, u8, &str, &str)] = &[
    ("Africa/Cairo", 120, 0, "Eastern European Standard Time", ""),
    ("Africa/Johannesburg", 120, 0, "South Africa Standard Time", ""),
    ("Africa/Lagos", 60, 0, "West Africa Standard Time", ""),
    ("Africa/Nairobi", 180, 0, "East Africa Time", ""),
    ("America/Anchorage", -540, 1, "Alaska Standard Time", "Alaska Daylight Time"),
    ("America/Argentina/Buenos_Aires", -180, 0, "Argentina Standard Time", ""),
    ("America/Bogota", -300, 0, "Colombia Standard Time", ""),
    ("America/Chicago", -360, 1, "Central Standard Time", "Central Daylight Time"),
    ("America/Denver", -420, 1, "Mountain Standard Time", "Mountain Daylight Time"),
    ("America/Edmonton", -420, 1, "Mountain Standard Time", "Mountain Daylight Time"),
    ("America/Halifax", -240, 1, "Atlantic Standard Time", "Atlantic Daylight Time"),
    ("America/Lima", -300, 0, "Peru Standard Time", ""),
    ("America/Los_Angeles", -480, 1, "Pacific Standard Time", "Pacific Daylight Time"),
    ("America/Mexico_City", -360, 0, "Central Standard Time", ""),
    ("America/New_York", -300, 1, "Eastern Standard Time", "Eastern Daylight Time"),
    ("America/Phoenix", -420, 0, "Mountain Standard Time", ""),
    ("America/Santiago", -240, 0, "Chile Standard Time", ""),
    ("America/Sao_Paulo", -180, 0, "Brasilia Standard Time", ""),
    ("America/St_Johns", -210, 1, "Newfoundland Standard Time", "Newfoundland Daylight Time"),
    ("America/Toronto", -300, 1, "Eastern Standard Time", "Eastern Daylight Time"),
    ("America/Vancouver", -480, 1, "Pacific Standard Time", "Pacific Daylight Time"),
    ("Asia/Bangkok", 420, 0, "Indochina Time", ""),
    ("Asia/Dhaka", 360, 0, "Bangladesh Standard Time", ""),
    ("Asia/Dubai", 240, 0, "Gulf Standard Time", ""),
    ("Asia/Ho_Chi_Minh", 420, 0, "Indochina Time", ""),
    ("Asia/Hong_Kong", 480, 0, "Hong Kong Standard Time", ""),
    ("Asia/Jakarta", 420, 0, "Western Indonesia Time", ""),
    ("Asia/Jerusalem", 120, 0, "Israel Standard Time", ""),
    ("Asia/Karachi", 300, 0, "Pakistan Standard Time", ""),
    ("Asia/Kathmandu", 345, 0, "Nepal Time", ""),
    ("Asia/Kolkata", 330, 0, "India Standard Time", ""),
    ("Asia/Kuala_Lumpur", 480, 0, "Malaysia Time", ""),
    ("Asia/Manila", 480, 0, "Philippine Standard Time", ""),
    ("Asia/Riyadh", 180, 0, "Arabian Standard Time", ""),
    ("Asia/Seoul", 540, 0, "Korean Standard Time", ""),
    ("Asia/Shanghai", 480, 0, "China Standard Time", ""),
    ("Asia/Singapore", 480, 0, "Singapore Standard Time", ""),
    ("Asia/Taipei", 480, 0, "Taipei Standard Time", ""),
    ("Asia/Tehran", 210, 0, "Iran Standard Time", ""),
    ("Asia/Tokyo", 540, 0, "Japan Standard Time", ""),
    ("Asia/Yangon", 390, 0, "Myanmar Time", ""),
    ("Atlantic/Reykjavik", 0, 0, "Greenwich Mean Time", ""),
    ("Australia/Adelaide", 570, 3, "Australian Central Standard Time", "Australian Central Daylight Time"),
    ("Australia/Brisbane", 600, 0, "Australian Eastern Standard Time", ""),
    ("Australia/Darwin", 570, 0, "Australian Central Standard Time", ""),
    ("Australia/Melbourne", 600, 3, "Australian Eastern Standard Time", "Australian Eastern Daylight Time"),
    ("Australia/Perth", 480, 0, "Australian Western Standard Time", ""),
    ("Australia/Sydney", 600, 3, "Australian Eastern Standard Time", "Australian Eastern Daylight Time"),
    ("Europe/Amsterdam", 60, 2, "Central European Standard Time", "Central European Summer Time"),
    ("Europe/Athens", 120, 2, "Eastern European Standard Time", "Eastern European Summer Time"),
    ("Europe/Berlin", 60, 2, "Central European Standard Time", "Central European Summer Time"),
    ("Europe/Brussels", 60, 2, "Central European Standard Time", "Central European Summer Time"),
    ("Europe/Bucharest", 120, 2, "Eastern European Standard Time", "Eastern European Summer Time"),
    ("Europe/Copenhagen", 60, 2, "Central European Standard Time", "Central European Summer Time"),
    ("Europe/Dublin", 0, 2, "Greenwich Mean Time", "Irish Standard Time"),
    ("Europe/Helsinki", 120, 2, "Eastern European Standard Time", "Eastern European Summer Time"),
    ("Europe/Istanbul", 180, 0, "", ""),
    ("Europe/Kyiv", 120, 2, "Eastern European Standard Time", "Eastern European Summer Time"),
    ("Europe/Lisbon", 0, 2, "Western European Standard Time", "Western European Summer Time"),
    ("Europe/London", 0, 2, "Greenwich Mean Time", "British Summer Time"),
    ("Europe/Madrid", 60, 2, "Central European Standard Time", "Central European Summer Time"),
    ("Europe/Moscow", 180, 0, "Moscow Standard Time", ""),
    ("Europe/Oslo", 60, 2, "Central European Standard Time", "Central European Summer Time"),
    ("Europe/Paris", 60, 2, "Central European Standard Time", "Central European Summer Time"),
    ("Europe/Prague", 60, 2, "Central European Standard Time", "Central European Summer Time"),
    ("Europe/Rome", 60, 2, "Central European Standard Time", "Central European Summer Time"),
    ("Europe/Stockholm", 60, 2, "Central European Standard Time", "Central European Summer Time"),
    ("Europe/Vienna", 60, 2, "Central European Standard Time", "Central European Summer Time"),
    ("Europe/Warsaw", 60, 2, "Central European Standard Time", "Central European Summer Time"),
    ("Europe/Zurich", 60, 2, "Central European Standard Time", "Central European Summer Time"),
    ("Pacific/Auckland", 720, 4, "New Zealand Standard Time", "New Zealand Daylight Time"),
    ("Pacific/Honolulu", -600, 0, "Hawaii-Aleutian Standard Time", ""),
    ("UTC", 0, 0, "Coordinated Universal Time", ""),
];

/// Other names for the same zones.
const ZONE_LINKS: &[(&str, &str)] = &[
    ("Asia/Calcutta", "Asia/Kolkata"),
    ("Asia/Saigon", "Asia/Ho_Chi_Minh"),
    ("Asia/Rangoon", "Asia/Yangon"),
    ("Asia/Katmandu", "Asia/Kathmandu"),
    ("Europe/Kiev", "Europe/Kyiv"),
    ("US/Eastern", "America/New_York"),
    ("US/Central", "America/Chicago"),
    ("US/Mountain", "America/Denver"),
    ("US/Pacific", "America/Los_Angeles"),
    ("US/Alaska", "America/Anchorage"),
    ("US/Hawaii", "Pacific/Honolulu"),
    ("US/Arizona", "America/Phoenix"),
    ("Canada/Eastern", "America/Toronto"),
    ("Canada/Pacific", "America/Vancouver"),
    ("Japan", "Asia/Tokyo"),
    ("Singapore", "Asia/Singapore"),
    ("Hongkong", "Asia/Hong_Kong"),
    ("PRC", "Asia/Shanghai"),
    ("ROK", "Asia/Seoul"),
    ("GB", "Europe/London"),
    ("NZ", "Pacific/Auckland"),
    ("GMT", "UTC"),
    ("Etc/UTC", "UTC"),
    ("Etc/GMT", "UTC"),
    ("Etc/UCT", "UTC"),
    ("Etc/Universal", "UTC"),
    ("Etc/Zulu", "UTC"),
    ("Universal", "UTC"),
    ("Zulu", "UTC"),
    ("UCT", "UTC"),
];

/// A time zone: its name as resolvedOptions gives it, its standard offset, and its daylight saving rule.
#[derive(Clone)]
pub struct Zone {
    name: String,
    offset: i32,
    rule: u8,
}

/// The n-th given weekday (0 Sunday) of a month (0 January), as days since 1970; n = 0: the month's last.
fn nth_weekday(year: f64, month: f64, weekday: f64, n: u32) -> f64 {
    use super::date::{make_day, week_day};
    if n == 0 {
        let next = make_day(year, month + 1.0, 1.0);
        let back = (week_day(next * 86_400_000.0) - weekday + 7.0) % 7.0;
        return next - if back == 0.0 { 7.0 } else { back };
    }
    let first = make_day(year, month, 1.0);
    first + (weekday - week_day(first * 86_400_000.0) + 7.0) % 7.0 + 7.0 * (n as f64 - 1.0)
}

impl Zone {
    /// Its offset (minutes east of UTC) at time t (milliseconds since 1970).
    fn offset_at(&self, t: f64) -> i32 {
        if self.rule == 0 || !t.is_finite() {
            return self.offset;
        }
        const DAY: f64 = 86_400_000.0;
        const HOUR: f64 = 3_600_000.0;
        let std = self.offset as f64 * 60_000.0;
        let year = super::date::year_from_time(t + std);
        // When summer time starts and ends that year, in UTC.
        let (start, end) = match self.rule {
            1 => (nth_weekday(year, 2.0, 0.0, 2) * DAY + 2.0 * HOUR - std, nth_weekday(year, 10.0, 0.0, 1) * DAY + HOUR - std),
            2 => (nth_weekday(year, 2.0, 0.0, 0) * DAY + HOUR, nth_weekday(year, 9.0, 0.0, 0) * DAY + HOUR),
            3 => (nth_weekday(year, 9.0, 0.0, 1) * DAY + 2.0 * HOUR - std, nth_weekday(year, 3.0, 0.0, 1) * DAY + 2.0 * HOUR - std),
            _ => (nth_weekday(year, 8.0, 0.0, 0) * DAY + 2.0 * HOUR - std, nth_weekday(year, 3.0, 0.0, 1) * DAY + 2.0 * HOUR - std),
        };
        let summer = if start < end { t >= start && t < end } else { t >= start || t < end };
        self.offset + if summer { 60 } else { 0 }
    }

    fn names(&self) -> (&'static str, &'static str) {
        ZONES.iter().find(|z| z.0 == self.name).map_or(("", ""), |z| (z.3, z.4))
    }

    /// Its name at time t, at a width (timeZoneName: 1 short, 2 long, 3 shortOffset, 4 longOffset, 5 shortGeneric,
    /// 6 longGeneric).
    fn name_at(&self, t: f64, width: u8) -> String {
        let off = self.offset_at(t);
        let summer = off != self.offset;
        let (standard, daylight) = self.names();
        let long = if summer && !daylight.is_empty() { daylight } else { standard };
        let short = match standard {
            "Eastern Standard Time" => Some(("EST", "EDT", "ET")),
            "Central Standard Time" => Some(("CST", "CDT", "CT")),
            "Mountain Standard Time" => Some(("MST", "MDT", "MT")),
            "Pacific Standard Time" => Some(("PST", "PDT", "PT")),
            "Alaska Standard Time" => Some(("AKST", "AKDT", "AKT")),
            "Hawaii-Aleutian Standard Time" => Some(("HST", "HDT", "HST")),
            "Atlantic Standard Time" => Some(("AST", "ADT", "AT")),
            "Coordinated Universal Time" => Some(("UTC", "UTC", "UTC")),
            _ => None,
        };
        match width {
            1 => short.map_or_else(|| gmt(off, false), |s| String::from(if summer { s.1 } else { s.0 })),
            2 if !long.is_empty() => String::from(long),
            3 => gmt(off, false),
            4 => gmt(off, true),
            5 => short.map_or_else(|| gmt(off, false), |s| String::from(s.2)),
            6 if !standard.is_empty() => {
                if daylight.is_empty() || standard.ends_with("Standard Time") {
                    standard.replace(" Standard Time", " Time")
                } else {
                    String::from(standard)
                }
            }
            _ => gmt(off, true),
        }
    }
}

/// "GMT+8", "GMT-3:30" (long: "GMT+08:00", "GMT-03:30"); "GMT" at 0.
fn gmt(off: i32, long: bool) -> String {
    if off == 0 {
        return String::from("GMT");
    }
    let sign = if off < 0 { '-' } else { '+' };
    let (h, m) = (off.abs() / 60, off.abs() % 60);
    if long {
        format!("GMT{sign}{h:02}:{m:02}")
    } else if m == 0 {
        format!("GMT{sign}{h}")
    } else {
        format!("GMT{sign}{h}:{m:02}")
    }
}

/// The PC's own zone: by its name if the embedder gave one this knows, else a zone of its offset.
pub fn local_zone(vm: &Vm) -> Zone {
    if let Some(z) = vm.tz_name.as_deref().and_then(zone_named) {
        return z;
    }
    let off = vm.tz_offset;
    let named = match off {
        0 => Some("UTC"),
        330 => Some("Asia/Kolkata"),
        345 => Some("Asia/Kathmandu"),
        210 => Some("Asia/Tehran"),
        390 => Some("Asia/Yangon"),
        570 => Some("Australia/Darwin"),
        _ => None,
    };
    let name = match named {
        Some(n) => String::from(n),
        // (Etc/GMT-8 is 8 hours ahead: its sign is the other way.)
        None if off % 60 == 0 && (-720..=840).contains(&off) => format!("Etc/GMT{}{}", if off > 0 { '-' } else { '+' }, (off / 60).abs()),
        None => format!("{}{:02}:{:02}", if off < 0 { '-' } else { '+' }, off.abs() / 60, off.abs() % 60),
    };
    Zone { name, offset: off, rule: 0 }
}

/// The local time's offset (minutes) at time t: the named zone's, with its summer time, when the embedder named a
/// zone this knows; else the embedder's offset.
pub fn local_offset_at(vm: &Vm, t: f64) -> i32 {
    match vm.tz_name.as_deref().and_then(zone_named) {
        Some(z) => z.offset_at(t),
        None => vm.tz_offset,
    }
}

/// A zone by its name (any case), or an offset ("+08:00", "-0330").
fn zone_named(name: &str) -> Option<Zone> {
    let name = ZONE_LINKS.iter().find(|(a, _)| a.eq_ignore_ascii_case(name)).map_or(name, |(_, b)| *b);
    if let Some(z) = ZONES.iter().find(|z| z.0.eq_ignore_ascii_case(name)) {
        return Some(Zone { name: String::from(z.0), offset: z.1, rule: z.2 });
    }
    let lower = name.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("etc/gmt") {
        let h: i32 = rest.strip_prefix('+').unwrap_or(rest).parse().ok()?;
        if !(-14..=12).contains(&h) {
            return None;
        }
        return Some(Zone { name: format!("Etc/GMT{}{}", if h < 0 { '-' } else { '+' }, h.abs()), offset: -h * 60, rule: 0 });
    }
    let b = name.as_bytes();
    if let Some(&s) = b.first()
        && (s == b'+' || s == b'-')
    {
        let rest = &name[1..];
        let (h, m) = match rest.split_once(':') {
            Some((h, m)) if h.len() == 2 && m.len() == 2 => (h, m),
            None if rest.len() == 2 => (rest, "00"),
            None if rest.len() == 4 => (&rest[..2], &rest[2..]),
            _ => return None,
        };
        if !h.bytes().chain(m.bytes()).all(|d| d.is_ascii_digit()) {
            return None;
        }
        let (h, m): (i32, i32) = (h.parse().ok()?, m.parse().ok()?);
        if h > 23 || m > 59 {
            return None;
        }
        let off = (h * 60 + m) * if s == b'-' { -1 } else { 1 };
        return Some(Zone { name: format!("{}{h:02}:{m:02}", if off < 0 { '-' } else { '+' }), offset: off, rule: 0 });
    }
    None
}

// -------------------------------------------------------------------------------------------- DateTimeFormat

#[derive(Clone)]
pub struct DateOpts {
    zone: Zone,
    /// The fields: 0 none; weekday, era, dayPeriod: 1 narrow, 2 short, 3 long; year, day, hour, minute, second: 1
    /// numeric, 2 2-digit; month: those, then 3 narrow, 4 short, 5 long; timeZoneName: TZ_NAMES's, from 1.
    weekday: u8,
    era: u8,
    year: u8,
    month: u8,
    day: u8,
    day_period: u8,
    hour: u8,
    minute: u8,
    second: u8,
    frac: u8,
    tz_name: u8,
    /// 0 h11, 1 h12, 2 h23, 3 h24.
    cycle: u8,
    /// 0 none, 1 full, 2 long, 3 medium, 4 short.
    date_style: u8,
    time_style: u8,
    /// Its numbering system.
    nu: String,
}

const TEXT_WIDTHS: &[&str] = &["narrow", "short", "long"];
const NUMERIC: &[&str] = &["numeric", "2-digit"];
const MONTH_STYLES: &[&str] = &["numeric", "2-digit", "narrow", "short", "long"];
const STYLE_LENGTHS: &[&str] = &["full", "long", "medium", "short"];
const TZ_NAMES: &[&str] = &["short", "long", "shortOffset", "longOffset", "shortGeneric", "longGeneric"];
const CYCLES: &[&str] = &["h11", "h12", "h23", "h24"];

/// A DateTimeFormat's options (CreateDateTimeFormat), with the fields it has by default: `need` 0 date, 1 time, 2
/// any; `defaults` 0 date, 1 time, 2 all.
fn date_opts(vm: &mut Vm, o: Option<Gc>, need: u8, defaults: u8, res: &mut Resolution) -> Result<DateOpts, Value> {
    opt_str(vm, o, "localeMatcher", MATCHERS, Some(1))?;
    let ca = opt_keyword(vm, o, "calendar")?;
    let nu = opt_keyword(vm, o, "numberingSystem")?;
    let hour12 = opt_bool(vm, o, "hour12")?;
    let cycle = opt_str(vm, o, "hourCycle", CYCLES, None)?;
    res.key("ca", ca.as_deref(), &["gregory"]);
    let nu = res.key("nu", nu.as_deref(), &numbering_names()).unwrap_or_else(|| String::from("latn"));
    // (hour12 overrides any hour cycle, the locale's too.)
    let hc_option = if hour12.is_some() { Some("null") } else { cycle.map(|c| CYCLES[c]) };
    let hc = res.key("hc", hc_option, &["h11", "h12", "h23", "h24", "null"]);
    let zone = match get(vm, o, "timeZone")? {
        Value::Undefined => local_zone(vm),
        v => {
            let name = vm.to_string(&v)?.to_rust();
            match zone_named(&name) {
                Some(z) => z,
                None => return Err(vm.range_error(&format!("Invalid time zone specified: {name}"))),
            }
        }
    };
    let at = |v: Option<usize>| v.map_or(0, |i| i as u8 + 1);
    let weekday = at(opt_str(vm, o, "weekday", TEXT_WIDTHS, None)?);
    let era = at(opt_str(vm, o, "era", TEXT_WIDTHS, None)?);
    let year = at(opt_str(vm, o, "year", NUMERIC, None)?);
    let month = at(opt_str(vm, o, "month", MONTH_STYLES, None)?);
    let day = at(opt_str(vm, o, "day", NUMERIC, None)?);
    let day_period = at(opt_str(vm, o, "dayPeriod", TEXT_WIDTHS, None)?);
    let hour = at(opt_str(vm, o, "hour", NUMERIC, None)?);
    let minute = at(opt_str(vm, o, "minute", NUMERIC, None)?);
    let second = at(opt_str(vm, o, "second", NUMERIC, None)?);
    let frac = opt_num(vm, o, "fractionalSecondDigits", 1, 3)?.unwrap_or(0) as u8;
    let tz_name = at(opt_str(vm, o, "timeZoneName", TZ_NAMES, None)?);
    opt_str(vm, o, "formatMatcher", &["basic", "best fit"], Some(1))?;
    let date_style = at(opt_str(vm, o, "dateStyle", STYLE_LENGTHS, None)?);
    let time_style = at(opt_str(vm, o, "timeStyle", STYLE_LENGTHS, None)?);
    let has_date = weekday + year + month + day > 0;
    let has_time = day_period + hour + minute + second + frac > 0;
    if (date_style > 0 || time_style > 0) && (has_date || has_time || era + tz_name > 0) {
        return Err(vm.type_error("Can't set option dateStyle or timeStyle together with other date and time options"));
    }
    if need == 0 && time_style > 0 {
        return Err(vm.type_error("Invalid option : timeStyle"));
    }
    if need == 1 && date_style > 0 {
        return Err(vm.type_error("Invalid option : dateStyle"));
    }
    let cycle = match (hour12, hc.as_deref()) {
        (Some(true), _) => 1,
        (Some(false), _) => 2,
        (None, Some(c)) => CYCLES.iter().position(|x| *x == c).unwrap_or(1) as u8,
        (None, None) => 1,
    };
    let mut d = DateOpts { zone, weekday, era, year, month, day, day_period, hour, minute, second, frac, tz_name, cycle, date_style, time_style, nu };
    let wanted = date_style == 0
        && time_style == 0
        && match need {
            0 => !has_date,
            1 => !has_time,
            _ => !has_date && !has_time,
        };
    if wanted && defaults != 1 {
        (d.year, d.month, d.day) = (1, 1, 1);
    }
    if wanted && defaults != 0 {
        (d.hour, d.minute, d.second) = (1, 1, 1);
    }
    Ok(d)
}

fn date_format_new(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let p = vm.proto_from_constructor(nt, I::IntlDateTimeFormatProto)?;
    let mut res = resolution(vm, &arg(args, 0))?;
    let o = options(vm, &arg(args, 1))?;
    let opts = date_opts(vm, o, 2, 0, &mut res)?;
    Ok(alloc_intl(vm, p, res.locale(), IntlData::Date(Box::new(opts))))
}

const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const WEEKDAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/// A word at a width: 1 its first letter, 2 its first three, else the whole.
fn width(word: &str, w: u8) -> String {
    String::from(match w {
        1 => &word[..1],
        2 => &word[..3],
        _ => word,
    })
}

impl DateOpts {
    /// Its fields, with the styles' put in.
    fn fields(&self) -> DateOpts {
        let mut o = self.clone();
        match self.date_style {
            1 => (o.weekday, o.month, o.day, o.year) = (3, 5, 1, 1),
            2 => (o.month, o.day, o.year) = (5, 1, 1),
            3 => (o.month, o.day, o.year) = (4, 1, 1),
            4 => (o.month, o.day, o.year) = (1, 1, 2),
            _ => {}
        }
        if self.time_style > 0 {
            (o.hour, o.minute) = (1, 2);
            if self.time_style <= 3 {
                o.second = 2;
            }
            o.tz_name = match self.time_style {
                1 => 2,
                2 => 1,
                _ => 0,
            };
        }
        o
    }
}

/// A date formatted as the options say (t: milliseconds since 1970), as its parts.
fn format_date_parts(t: f64, opts: &DateOpts) -> Parts {
    use super::date::{date_from_time, month_from_time, week_day, year_from_time};
    let o = opts.fields();
    let off = o.zone.offset_at(t);
    let l = t + off as f64 * 60_000.0;
    let (y, mo, d, wd) = (year_from_time(l) as i64, month_from_time(l) as usize, date_from_time(l) as i64, week_day(l) as usize);
    let in_day = {
        let r = l % 86_400_000.0;
        (if r < 0.0 { r + 86_400_000.0 } else { r }) as i64
    };
    let (h, mi, s, ms) = (in_day / 3_600_000, in_day / 60_000 % 60, in_day / 1000 % 60, in_day % 1000);
    let pad = |n: i64, two: bool| if two { format!("{:02}", n.rem_euclid(100)) } else { format!("{n}") };
    let lit = |s: &str| ("literal", String::from(s));
    let mut date: Parts = Vec::new();
    if o.weekday > 0 {
        date.push(("weekday", width(WEEKDAYS[wd], o.weekday)));
    }
    if o.month >= 3 {
        // "Friday, November 6, 2026", "Nov 6, 2026", "November 2026", "November 6".
        if !date.is_empty() {
            date.push(lit(", "));
        }
        date.push(("month", width(MONTHS[mo], o.month - 2)));
        if o.day > 0 {
            date.push(lit(" "));
            date.push(("day", pad(d, o.day == 2)));
        }
        if o.year > 0 {
            date.push(lit(if o.day > 0 { ", " } else { " " }));
            date.push(("year", pad(y, o.year == 2)));
        }
    } else if o.month + o.day + o.year > 0 {
        // "11/6/2026", "11/6", "11/2026", "2026".
        if !date.is_empty() {
            date.push(lit(", "));
        }
        let mut first = true;
        for (kind, on, v, two) in [("month", o.month > 0, mo as i64 + 1, o.month == 2), ("day", o.day > 0, d, o.day == 2), ("year", o.year > 0, y, o.year == 2)] {
            if on {
                if !first {
                    date.push(lit("/"));
                }
                date.push((kind, pad(v, two)));
                first = false;
            }
        }
    }
    if o.era > 0 {
        let era = match (y > 0, o.era) {
            (true, 1) => "A",
            (true, 3) => "Anno Domini",
            (true, _) => "AD",
            (false, 1) => "B",
            (false, 3) => "Before Christ",
            (false, _) => "BC",
        };
        if !date.is_empty() {
            date.push(lit(" "));
        }
        date.push(("era", String::from(era)));
    }
    let mut time: Parts = Vec::new();
    let twelve = o.cycle < 2;
    let shown_h = match o.cycle {
        0 => h % 12,
        1 => (h + 11) % 12 + 1,
        2 => h,
        _ => (h + 23) % 24 + 1,
    };
    if o.hour > 0 {
        time.push(("hour", pad(shown_h, o.hour == 2 || (!twelve && (o.minute > 0 || o.second > 0)))));
    }
    if o.minute > 0 {
        if !time.is_empty() {
            time.push(lit(":"));
        }
        time.push(("minute", if o.hour > 0 || o.second > 0 { format!("{mi:02}") } else { pad(mi, o.minute == 2) }));
    }
    if o.second > 0 {
        if !time.is_empty() {
            time.push(lit(":"));
        }
        time.push(("second", if o.minute > 0 { format!("{s:02}") } else { pad(s, o.second == 2) }));
    }
    if o.frac > 0 {
        if !time.is_empty() {
            time.push(lit("."));
        }
        let f = format!("{ms:03}");
        time.push(("fractionalSecond", String::from(&f[..o.frac as usize])));
    }
    // (The day's periods: noon only at 12:00, written "n" when narrow.)
    let period = match (h, mi) {
        (12, 0) if o.day_period == 1 => "n",
        (12, 0) => "noon",
        (6..=11, _) => "in the morning",
        (12..=17, _) => "in the afternoon",
        (18..=20, _) => "in the evening",
        _ => "at night",
    };
    if o.hour > 0 && twelve {
        time.push(lit(" "));
        time.push(("dayPeriod", String::from(if o.day_period > 0 { period } else if h < 12 { "AM" } else { "PM" })));
    } else if o.day_period > 0 && o.hour == 0 {
        if !time.is_empty() {
            time.push(lit(" "));
        }
        time.push(("dayPeriod", String::from(period)));
    }
    if o.tz_name > 0 {
        let name = o.zone.name_at(t, o.tz_name);
        if !time.is_empty() {
            time.push(lit(" "));
            time.push(("timeZoneName", name));
        } else if !date.is_empty() {
            date.push(lit(", "));
            date.push(("timeZoneName", name));
        } else {
            time.push(("timeZoneName", name));
        }
    }
    if date.is_empty() {
        return parts_in_digits(time, &o.nu);
    }
    if !time.is_empty() {
        date.push(lit(if opts.date_style == 1 || opts.date_style == 2 { " at " } else { ", " }));
        date.extend(time);
    }
    parts_in_digits(date, &o.nu)
}

/// A time's local year, month and day in a zone.
fn local_day(t: f64, zone: &Zone) -> (i64, i64, i64) {
    use super::date::{date_from_time, month_from_time, year_from_time};
    let l = t + zone.offset_at(t) as f64 * 60_000.0;
    (year_from_time(l) as i64, month_from_time(l) as i64, date_from_time(l) as i64)
}

/// formatRange's parts, and where each came from (0 start, 1 end, 2 shared): what the two share written once, as
/// en-US's interval formats have it ("Jan 3 – 5, 2019", "Jan 3 – Mar 4, 2019", "10:00 – 11:00 AM").
fn date_range_parts(a: f64, b: f64, o: &DateOpts) -> (Parts, Vec<u8>) {
    let (pa, pb) = (format_date_parts(a, o), format_date_parts(b, o));
    if joined(&pa) == joined(&pb) {
        let n = pa.len();
        return (pa, vec![2; n]);
    }
    let f = o.fields();
    let ((ya, ma, da), (yb, mb, db)) = (local_day(a, &o.zone), local_day(b, &o.zone));
    let has_time = f.hour + f.minute + f.second + f.frac + f.day_period > 0;
    let sep = ("literal", String::from(" – "));
    let shared_at_end = |t: &str| matches!(t, "dayPeriod" | "timeZoneName" | "year" | "era");
    if has_time && (ya, ma, da) == (yb, mb, db) {
        // The same day: its date once, then the two times; the same half of the day: AM or PM once, at the end.
        let time_at = |p: &Parts| p.iter().position(|(t, _)| matches!(*t, "hour" | "minute" | "second" | "fractionalSecond" | "dayPeriod")).unwrap_or(0);
        let period = |p: &Parts| p.iter().find(|(t, _)| *t == "dayPeriod").map(|(_, v)| v.clone());
        let mut start = pa.clone();
        let cut = if f.cycle < 2 && f.hour > 0 && period(&pa).is_some() && period(&pa) == period(&pb) {
            start.iter().position(|(t, _)| *t == "dayPeriod")
        } else {
            start.iter().position(|(t, _)| *t == "timeZoneName")
        };
        if let Some(i) = cut {
            start.truncate(i.saturating_sub(1));
        }
        let date_end = time_at(&pa);
        let mut from: Vec<u8> = (0..start.len()).map(|k| if k < date_end { 2 } else { 0 }).collect();
        let end: Parts = pb[time_at(&pb)..].to_vec();
        from.push(2);
        let trailing = end.iter().rposition(|(t, _)| !shared_at_end(t) && *t != "literal").map_or(0, |k| k + 1);
        from.extend((0..end.len()).map(|k| if k >= trailing && cut.is_some() { 2 } else { 1 }));
        let mut parts = start;
        parts.push(sep);
        parts.extend(end);
        return (parts, from);
    }
    if !has_time && f.month >= 3 && ya == yb && f.tz_name == 0 && f.era == 0 {
        // A month by name in the same year: the year once; in the same month (without weekdays), the month too.
        let mut start = pa.clone();
        if let Some(i) = start.iter().position(|(t, _)| *t == "year") {
            start.truncate(i.saturating_sub(1));
        }
        let end_from = if ma == mb && f.weekday == 0 && f.day > 0 { pb.iter().position(|(t, _)| *t == "day").unwrap_or(0) } else { 0 };
        let end: Parts = pb[end_from..].to_vec();
        let year_at = end.iter().position(|(t, _)| *t == "year");
        let mut from = vec![0u8; start.len()];
        from.push(2);
        from.extend((0..end.len()).map(|k| if year_at.is_some_and(|y| k + 1 >= y) { 2 } else { 1 }));
        let mut parts = start;
        parts.push(sep);
        parts.extend(end);
        return (parts, from);
    }
    let mut from = vec![0u8; pa.len()];
    from.push(2);
    from.extend(core::iter::repeat_n(1u8, pb.len()));
    let mut parts = pa;
    parts.push(sep);
    parts.extend(pb);
    (parts, from)
}

fn this_date_format(vm: &mut Vm, this: &Value, what: &str) -> Result<DateOpts, Value> {
    match data_of(vm, this) {
        Some(IntlData::Date(o)) => Ok((**o).clone()),
        _ => Err(incompatible(vm, &format!("DateTimeFormat.prototype.{what}"))),
    }
}

/// A date argument's time (now, if it's undefined); a RangeError for an invalid one.
fn date_value(vm: &mut Vm, v: &Value) -> Result<f64, Value> {
    let t = match v {
        Value::Undefined => (vm.now)(),
        _ => vm.to_number(v)?,
    };
    if !t.is_finite() || t.abs() > 8.64e15 {
        return Err(vm.range_error("Invalid time value"));
    }
    Ok(crate::math::trunc(t) + 0.0)
}

fn df_format_getter(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    this_date_format(vm, this, "format")?;
    bound(vm, this, 1, df_format_bound)
}

fn df_format_bound(vm: &mut Vm, _this: &Value, args: &[Value], f: Gc) -> R {
    let df = vm.data(f);
    let o = this_date_format(vm, &df, "format")?;
    let t = date_value(vm, &arg(args, 0))?;
    Ok(js_str(&joined(&format_date_parts(t, &o))))
}

fn df_format_to_parts(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let o = this_date_format(vm, this, "formatToParts")?;
    let t = date_value(vm, &arg(args, 0))?;
    parts_array(vm, &format_date_parts(t, &o), None)
}

fn date_range(vm: &mut Vm, this: &Value, args: &[Value], what: &str) -> Result<(Parts, Vec<u8>), Value> {
    let o = this_date_format(vm, this, what)?;
    if arg(args, 0).is_undefined() || arg(args, 1).is_undefined() {
        return Err(vm.type_error("startDate or endDate is undefined"));
    }
    let a = date_value(vm, &arg(args, 0))?;
    let b = date_value(vm, &arg(args, 1))?;
    Ok(date_range_parts(a, b, &o))
}

fn df_format_range(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (parts, _) = date_range(vm, this, args, "formatRange")?;
    Ok(js_str(&joined(&parts)))
}

fn df_format_range_to_parts(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (parts, from) = date_range(vm, this, args, "formatRangeToParts")?;
    let source = |k: usize, _t: &str| Some(String::from(["startRange", "endRange", "shared"][from[k] as usize]));
    parts_array(vm, &parts, Some(("source", &source)))
}

fn df_resolved(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let o = this_date_format(vm, this, "resolvedOptions")?;
    let locale = locale_of(vm, this);
    let pick = |n: u8, names: &[&str]| if n == 0 { Value::Undefined } else { js_str(names[n as usize - 1]) };
    let mut items: Vec<(&str, Value)> = vec![("locale", js_str(&locale)), ("calendar", js_str("gregory")), ("numberingSystem", js_str(&o.nu)), ("timeZone", js_str(&o.zone.name))];
    if o.hour > 0 || o.time_style > 0 {
        items.push(("hourCycle", js_str(CYCLES[o.cycle as usize])));
        items.push(("hour12", Value::Bool(o.cycle < 2)));
    }
    if o.date_style == 0 && o.time_style == 0 {
        items.push(("weekday", pick(o.weekday, TEXT_WIDTHS)));
        items.push(("era", pick(o.era, TEXT_WIDTHS)));
        items.push(("year", pick(o.year, NUMERIC)));
        items.push(("month", pick(o.month, MONTH_STYLES)));
        items.push(("day", pick(o.day, NUMERIC)));
        items.push(("dayPeriod", pick(o.day_period, TEXT_WIDTHS)));
        items.push(("hour", pick(o.hour, NUMERIC)));
        items.push(("minute", if o.minute == 0 { Value::Undefined } else if o.hour > 0 || o.second > 0 { js_str("2-digit") } else { pick(o.minute, NUMERIC) }));
        items.push(("second", if o.second == 0 { Value::Undefined } else if o.minute > 0 { js_str("2-digit") } else { pick(o.second, NUMERIC) }));
        items.push(("fractionalSecondDigits", if o.frac == 0 { Value::Undefined } else { Value::Num(o.frac as f64) }));
        items.push(("timeZoneName", pick(o.tz_name, TZ_NAMES)));
    } else {
        items.push(("dateStyle", pick(o.date_style, STYLE_LENGTHS)));
        items.push(("timeStyle", pick(o.time_style, STYLE_LENGTHS)));
    }
    resolved(vm, items)
}

/// Date.prototype.toLocaleString, toLocaleDateString, toLocaleTimeString: `need` and `defaults` as
/// ToDateTimeOptions takes them (0 date, 1 time, 2 any or all).
pub fn date_to_locale(vm: &mut Vm, t: f64, locales_v: &Value, options_v: &Value, need: u8, defaults: u8) -> R {
    if t.is_nan() {
        return Ok(js_str("Invalid Date"));
    }
    let mut res = resolution(vm, locales_v)?;
    let o = options(vm, options_v)?;
    let opts = date_opts(vm, o, need, defaults, &mut res)?;
    Ok(js_str(&joined(&format_date_parts(t, &opts))))
}

// --------------------------------------------------------------------------------------------------- Collator

#[derive(Clone, Copy)]
pub struct CollatorOpts {
    /// 0 base, 1 accent, 2 case, 3 variant.
    sensitivity: u8,
    numeric: bool,
    ignore_punctuation: bool,
    /// 0 false, 1 upper, 2 lower.
    case_first: u8,
    search: bool,
}

const SENSITIVITIES: &[&str] = &["base", "accent", "case", "variant"];
const CASE_FIRSTS: &[&str] = &["false", "upper", "lower"];

fn collator_opts(vm: &mut Vm, o: Option<Gc>, res: &mut Resolution) -> Result<CollatorOpts, Value> {
    let search = opt_str(vm, o, "usage", &["sort", "search"], Some(0))? == Some(1);
    opt_str(vm, o, "localeMatcher", MATCHERS, Some(1))?;
    let co = opt_keyword(vm, o, "collation")?;
    let numeric = opt_bool(vm, o, "numeric")?;
    let case_first = opt_str(vm, o, "caseFirst", CASE_FIRSTS, None)?;
    res.key("co", co.as_deref(), &[]);
    let kn = res.key("kn", numeric.map(|b| if b { "true" } else { "false" }), &["true", "false"]);
    let kf = res.key("kf", case_first.map(|i| CASE_FIRSTS[i]), CASE_FIRSTS);
    let numeric = kn.as_deref() == Some("true");
    let case_first = kf.and_then(|k| CASE_FIRSTS.iter().position(|x| *x == k)).unwrap_or(0) as u8;
    let sensitivity = opt_str(vm, o, "sensitivity", SENSITIVITIES, Some(3))?.unwrap_or(3) as u8;
    let ignore_punctuation = opt_bool(vm, o, "ignorePunctuation")?.unwrap_or(false);
    Ok(CollatorOpts { sensitivity, numeric, ignore_punctuation, case_first, search })
}

fn collator_new(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let p = vm.proto_from_constructor(nt, I::IntlCollatorProto)?;
    let mut res = resolution(vm, &arg(args, 0))?;
    let o = options(vm, &arg(args, 1))?;
    let opts = collator_opts(vm, o, &mut res)?;
    Ok(alloc_intl(vm, p, res.locale(), IntlData::Collator(opts)))
}

/// Letters Unicode doesn't decompose that collate as another with a mark (ø as o with a stroke), and those that
/// collate as two letters (ß as ss).
fn letter_parts(c: char) -> Option<(char, Option<char>, u32)> {
    Some(match c {
        'ø' | 'Ø' => ('o', None, 0x338),
        'ł' | 'Ł' => ('l', None, 0x338),
        'đ' | 'Đ' => ('d', None, 0x335),
        'ħ' | 'Ħ' => ('h', None, 0x335),
        'ŧ' | 'Ŧ' => ('t', None, 0x335),
        'ı' => ('i', None, 0x1000),
        'ß' => ('s', Some('s'), 0),
        'ẞ' => ('s', Some('s'), 0),
        'æ' | 'Æ' => ('a', Some('e'), 0x1001),
        'œ' | 'Œ' => ('o', Some('e'), 0x1001),
        _ => return None,
    })
}

/// The marks in Unicode's default order (é before è before ê before ë...): a mark's secondary weight.
const MARK_ORDER: &[u32] = &[0x301, 0x300, 0x306, 0x302, 0x30C, 0x30A, 0x308, 0x30B, 0x303, 0x307, 0x338, 0x335, 0x327, 0x328, 0x304, 0x309, 0x30F, 0x311, 0x323, 0x326, 0x325];

fn mark_weight(m: u32) -> u64 {
    MARK_ORDER.iter().position(|x| *x == m).map_or(0x100 + m as u64, |i| i as u64 + 1)
}

/// ASCII's punctuation and symbols in the order Unicode's default collation gives them (all before digits, digits
/// before letters); the first 23 are punctuation (ignorePunctuation ignores them), the rest symbols.
const PUNCTUATION: &str = "_-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";

/// A collation element: primary (its group, and its place there), secondary (its marks), tertiary (its case).
type Element = ((u8, u64), u64, u8);

/// A string's collation elements: decomposed, each letter's marks folded into its secondary weight.
fn elements(s: &str, o: &CollatorOpts) -> Vec<Element> {
    let text = crate::normalize::nfd(s, false);
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<Element> = Vec::new();
    let upper_first = o.case_first == 1;
    let case_of = |c: char| (c.is_uppercase() != upper_first) as u8;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        i += 1;
        let cp = c as u32;
        if (0x300..=0x36F).contains(&cp) || (0x1AB0..=0x1AFF).contains(&cp) || (0x1DC0..=0x1DFF).contains(&cp) || (0x20D0..=0x20FF).contains(&cp) {
            if let Some(last) = out.last_mut() {
                last.1 = last.1 * 64 + mark_weight(cp);
            }
            continue;
        }
        if o.numeric && c.is_ascii_digit() {
            let mut v: u64 = cp as u64 - '0' as u64;
            while i < chars.len() && chars[i].is_ascii_digit() {
                v = v.saturating_mul(10).saturating_add(chars[i] as u64 - '0' as u64);
                i += 1;
            }
            out.push(((3, v), 0, 0));
            continue;
        }
        if c.is_whitespace() {
            if !o.ignore_punctuation {
                out.push(((0, cp as u64), 0, 0));
            }
            continue;
        }
        if let Some(k) = PUNCTUATION.find(c) {
            if !(o.ignore_punctuation && k < 23) {
                out.push(((1, k as u64), 0, 0));
            }
            continue;
        }
        if c.is_ascii_digit() {
            out.push(((3, cp as u64 - '0' as u64), 0, 0));
            continue;
        }
        if let Some((a, b, mark)) = letter_parts(c) {
            let case = case_of(c);
            out.push(((4, a as u64), if mark > 0 { mark_weight(mark) } else { 0 }, case));
            if let Some(b) = b {
                out.push(((4, b as u64), 0, case + 2));
            }
            continue;
        }
        if c.is_alphabetic() {
            let lower = c.to_lowercase().next().unwrap_or(c) as u64;
            // (Hangul's syllables come before kana and Han ideographs.)
            let place = if (0xAC00..=0xD7A3).contains(&lower) { 0x3000 + (lower - 0xAC00) / 0x1000 } else { lower };
            out.push(((4, place), 0, case_of(c)));
            continue;
        }
        if o.ignore_punctuation && matches!(cp, 0x2010..=0x2027 | 0x3000..=0x303F | 0xA1 | 0xBF | 0xAB | 0xBB) {
            continue;
        }
        out.push(((2, cp as u64), 0, 0));
    }
    out
}

/// Two strings compared as the collator does: by their letters (accents and case set aside), then their accents,
/// then their case (lower before upper unless caseFirst says); digits by their numbers' values if `numeric`; strings
/// Unicode says are the same (é, and e with ◌́) the same.
pub fn collate(a: &str, b: &str, o: &CollatorOpts) -> core::cmp::Ordering {
    use core::cmp::Ordering;
    let (ea, eb) = (elements(a, o), elements(b, o));
    let primary = ea.iter().map(|e| e.0).cmp(eb.iter().map(|e| e.0));
    if primary != Ordering::Equal || o.sensitivity == 0 {
        return primary;
    }
    if o.sensitivity != 2 {
        let secondary = ea.iter().map(|e| e.1).cmp(eb.iter().map(|e| e.1));
        if secondary != Ordering::Equal {
            return secondary;
        }
    }
    if o.sensitivity >= 2 {
        return ea.iter().map(|e| e.2).cmp(eb.iter().map(|e| e.2));
    }
    Ordering::Equal
}

fn this_collator(vm: &mut Vm, this: &Value, what: &str) -> Result<CollatorOpts, Value> {
    match data_of(vm, this) {
        Some(IntlData::Collator(o)) => Ok(*o),
        _ => Err(incompatible(vm, &format!("Collator.prototype.{what}"))),
    }
}

fn collator_compare_getter(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    this_collator(vm, this, "compare")?;
    bound(vm, this, 2, collator_compare_bound)
}

fn ordering(o: core::cmp::Ordering) -> Value {
    Value::Num(match o {
        core::cmp::Ordering::Less => -1.0,
        core::cmp::Ordering::Equal => 0.0,
        core::cmp::Ordering::Greater => 1.0,
    })
}

fn collator_compare_bound(vm: &mut Vm, _this: &Value, args: &[Value], f: Gc) -> R {
    let c = vm.data(f);
    let o = this_collator(vm, &c, "compare")?;
    let a = vm.to_string(&arg(args, 0))?.to_rust();
    let b = vm.to_string(&arg(args, 1))?.to_rust();
    Ok(ordering(collate(&a, &b, &o)))
}

fn collator_resolved(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let o = this_collator(vm, this, "resolvedOptions")?;
    let locale = locale_of(vm, this);
    let items = vec![
        ("locale", js_str(&locale)),
        ("usage", js_str(if o.search { "search" } else { "sort" })),
        ("sensitivity", js_str(SENSITIVITIES[o.sensitivity as usize])),
        ("ignorePunctuation", Value::Bool(o.ignore_punctuation)),
        ("collation", js_str("default")),
        ("numeric", Value::Bool(o.numeric)),
        ("caseFirst", js_str(CASE_FIRSTS[o.case_first as usize])),
    ];
    resolved(vm, items)
}

/// String.prototype.localeCompare: as a Collator of these locales and options compares the two.
pub fn locale_compare(vm: &mut Vm, a: &str, b: &str, locales_v: &Value, options_v: &Value) -> R {
    let mut res = resolution(vm, locales_v)?;
    let o = options(vm, options_v)?;
    let opts = collator_opts(vm, o, &mut res)?;
    Ok(ordering(collate(a, b, &opts)))
}

// ------------------------------------------------------------------------------------------------ PluralRules

#[derive(Clone)]
pub struct PluralOpts {
    ordinal: bool,
    digits: NumberOpts,
}

fn plural_new(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let p = vm.proto_from_constructor(nt, I::IntlPluralRulesProto)?;
    let res = resolution(vm, &arg(args, 0))?;
    let o = options(vm, &arg(args, 1))?;
    opt_str(vm, o, "localeMatcher", MATCHERS, Some(1))?;
    let ordinal = opt_str(vm, o, "type", &["cardinal", "ordinal"], Some(0))? == Some(1);
    let mut digits = NumberOpts::plain(0, 3);
    digit_opts(vm, o, &mut digits, 0, 3, false)?;
    Ok(alloc_intl(vm, p, res.locale(), IntlData::Plural(Box::new(PluralOpts { ordinal, digits }))))
}

/// English's plural category for a number: "one" (1, with nothing shown after its point), else "other"; ordinals'
/// "one" (1st, 21st), "two" (2nd), "few" (3rd), else "other" (4th, 11th, 12th, 13th).
fn plural_of(x: &Num, o: &PluralOpts) -> &'static str {
    let Num::Fin(d, e, neg) = x else { return "other" };
    let (int, frac) = rounded(d, *e, *neg, &o.digits);
    if o.ordinal {
        if !frac.bytes().all(|b| b == b'0') {
            return "other";
        }
        let last2: u32 = int[int.len().saturating_sub(2)..].parse().unwrap_or(0);
        return match (last2 % 10, last2) {
            (1, r) if r != 11 => "one",
            (2, r) if r != 12 => "two",
            (3, r) if r != 13 => "few",
            _ => "other",
        };
    }
    if is_one(&int, &frac) { "one" } else { "other" }
}

fn this_plural(vm: &mut Vm, this: &Value, what: &str) -> Result<PluralOpts, Value> {
    match data_of(vm, this) {
        Some(IntlData::Plural(o)) => Ok((**o).clone()),
        _ => Err(incompatible(vm, &format!("PluralRules.prototype.{what}"))),
    }
}

fn plural_select(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let o = this_plural(vm, this, "select")?;
    let x = vm.to_number(&arg(args, 0))?;
    Ok(js_str(plural_of(&Num::of(x), &o)))
}

fn plural_select_range(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let o = this_plural(vm, this, "selectRange")?;
    if arg(args, 0).is_undefined() || arg(args, 1).is_undefined() {
        return Err(vm.type_error("start or end is undefined"));
    }
    let a = vm.to_number(&arg(args, 0))?;
    let b = vm.to_number(&arg(args, 1))?;
    if a.is_nan() || b.is_nan() {
        return Err(vm.range_error("start or end is NaN"));
    }
    Ok(js_str(plural_of(&Num::of(b), &o)))
}

fn plural_resolved(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let o = this_plural(vm, this, "resolvedOptions")?;
    let cats: Vec<Value> = if o.ordinal { ["few", "one", "two", "other"].into_iter().map(js_str).collect() } else { ["one", "other"].into_iter().map(js_str).collect() };
    let cats = Value::Obj(vm.new_array(cats));
    let locale = locale_of(vm, this);
    let mut items = vec![("locale", js_str(&locale)), ("type", js_str(if o.ordinal { "ordinal" } else { "cardinal" }))];
    digit_items(&o.digits, &mut items);
    items.push(("pluralCategories", cats));
    rounding_items(&o.digits, &mut items);
    resolved(vm, items)
}

// ----------------------------------------------------------------------------------------- RelativeTimeFormat

#[derive(Clone)]
pub struct RelativeOpts {
    /// 0 long, 1 short, 2 narrow.
    style: u8,
    auto: bool,
    nu: String,
}

const WIDTHS_LSN: &[&str] = &["long", "short", "narrow"];

fn relative_new(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let p = vm.proto_from_constructor(nt, I::IntlRelativeTimeFormatProto)?;
    let mut res = resolution(vm, &arg(args, 0))?;
    let o = options(vm, &arg(args, 1))?;
    opt_str(vm, o, "localeMatcher", MATCHERS, Some(1))?;
    let nu = opt_keyword(vm, o, "numberingSystem")?;
    let nu = res.key("nu", nu.as_deref(), &numbering_names()).unwrap_or_else(|| String::from("latn"));
    let style = opt_str(vm, o, "style", WIDTHS_LSN, Some(0))?.unwrap_or(0) as u8;
    let auto = opt_str(vm, o, "numeric", &["always", "auto"], Some(0))? == Some(1);
    Ok(alloc_intl(vm, p, res.locale(), IntlData::Relative(Box::new(RelativeOpts { style, auto, nu }))))
}

fn this_relative(vm: &mut Vm, this: &Value, what: &str) -> Result<RelativeOpts, Value> {
    match data_of(vm, this) {
        Some(IntlData::Relative(o)) => Ok((**o).clone()),
        _ => Err(incompatible(vm, &format!("RelativeTimeFormat.prototype.{what}"))),
    }
}

/// The words numeric: "auto" has for -1, 0 and 1 of a unit.
fn relative_word(unit: &str, x: f64) -> Option<&'static str> {
    Some(match (unit, x as i64) {
        ("day", 0) => "today",
        ("day", 1) => "tomorrow",
        ("day", -1) => "yesterday",
        ("second", 0) => "now",
        ("minute", 0) => "this minute",
        ("hour", 0) => "this hour",
        ("week", 0) => "this week",
        ("week", 1) => "next week",
        ("week", -1) => "last week",
        ("month", 0) => "this month",
        ("month", 1) => "next month",
        ("month", -1) => "last month",
        ("quarter", 0) => "this quarter",
        ("quarter", 1) => "next quarter",
        ("quarter", -1) => "last quarter",
        ("year", 0) => "this year",
        ("year", 1) => "next year",
        ("year", -1) => "last year",
        _ => return None,
    })
}

/// "in 3 days", "3 days ago", "yesterday" (numeric: auto), as parts, and the unit (singular).
fn relative_parts(vm: &mut Vm, o: &RelativeOpts, v: &Value, unit_v: &Value) -> Result<(Parts, &'static str), Value> {
    let x = vm.to_number(v)?;
    let unit_s = vm.to_string(unit_v)?.to_rust();
    if !x.is_finite() {
        return Err(vm.range_error("Invalid value: the number must be finite"));
    }
    // (Its name: long, short, narrow, and short's plural.)
    let (unit, names): (&'static str, (&str, &str, &str, &str)) = match unit_s.strip_suffix('s').unwrap_or(&unit_s) {
        "second" => ("second", ("second", "sec.", "s", "sec.")),
        "minute" => ("minute", ("minute", "min.", "m", "min.")),
        "hour" => ("hour", ("hour", "hr.", "h", "hr.")),
        "day" => ("day", ("day", "day", "d", "days")),
        "week" => ("week", ("week", "wk.", "w", "wk.")),
        "month" => ("month", ("month", "mo.", "mo", "mo.")),
        "quarter" => ("quarter", ("quarter", "qtr.", "q", "qtrs.")),
        "year" => ("year", ("year", "yr.", "y", "yr.")),
        _ => return Err(vm.range_error(&format!("Invalid unit argument for format() '{unit_s}'"))),
    };
    if o.auto
        && x == crate::math::trunc(x)
        && let Some(w) = relative_word(unit, x)
    {
        return Ok((vec![("literal", String::from(w))], unit));
    }
    let past = x < 0.0 || (x == 0.0 && x.is_sign_negative());
    let num = format_parts(&Num::of(x.abs()), &NumberOpts::plain(0, 3));
    let one = joined(&num) == "1";
    let num = parts_in_digits(num, &o.nu);
    let word = match o.style {
        0 => format!(" {}{}", names.0, if one { "" } else { "s" }),
        1 => format!(" {}", if one { names.1 } else { names.3 }),
        _ => String::from(names.2),
    };
    let mut parts: Parts = Vec::new();
    if past {
        parts.extend(num);
        parts.push(("literal", format!("{word} ago")));
    } else {
        parts.push(("literal", String::from("in ")));
        parts.extend(num);
        parts.push(("literal", word));
    }
    Ok((parts, unit))
}

fn relative_format(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let o = this_relative(vm, this, "format")?;
    let (parts, _) = relative_parts(vm, &o, &arg(args, 0), &arg(args, 1))?;
    Ok(js_str(&joined(&parts)))
}

fn relative_format_to_parts(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let o = this_relative(vm, this, "formatToParts")?;
    let (parts, unit) = relative_parts(vm, &o, &arg(args, 0), &arg(args, 1))?;
    let with_unit = |_k: usize, t: &str| if t == "literal" { None } else { Some(String::from(unit)) };
    parts_array(vm, &parts, Some(("unit", &with_unit)))
}

fn relative_resolved(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let o = this_relative(vm, this, "resolvedOptions")?;
    let locale = locale_of(vm, this);
    let items = vec![
        ("locale", js_str(&locale)),
        ("style", js_str(WIDTHS_LSN[o.style as usize])),
        ("numeric", js_str(if o.auto { "auto" } else { "always" })),
        ("numberingSystem", js_str(&o.nu)),
    ];
    resolved(vm, items)
}

// ------------------------------------------------------------------------------------------------- ListFormat

#[derive(Clone, Copy)]
pub struct ListOpts {
    /// 0 conjunction, 1 disjunction, 2 unit.
    kind: u8,
    /// 0 long, 1 short, 2 narrow.
    style: u8,
}

const LIST_KINDS: &[&str] = &["conjunction", "disjunction", "unit"];

fn list_new(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let p = vm.proto_from_constructor(nt, I::IntlListFormatProto)?;
    let res = resolution(vm, &arg(args, 0))?;
    let o = options_object(vm, &arg(args, 1))?;
    opt_str(vm, o, "localeMatcher", MATCHERS, Some(1))?;
    let kind = opt_str(vm, o, "type", LIST_KINDS, Some(0))?.unwrap_or(0) as u8;
    let style = opt_str(vm, o, "style", WIDTHS_LSN, Some(0))?.unwrap_or(0) as u8;
    Ok(alloc_intl(vm, p, res.locale(), IntlData::List(ListOpts { kind, style })))
}

fn this_list(vm: &mut Vm, this: &Value, what: &str) -> Result<ListOpts, Value> {
    match data_of(vm, this) {
        Some(IntlData::List(o)) => Ok(*o),
        _ => Err(incompatible(vm, &format!("ListFormat.prototype.{what}"))),
    }
}

/// "A, B, and C", "A & B", "A, B, or C", "A B C" (as its type and style say), as parts.
fn list_parts(vm: &mut Vm, o: &ListOpts, v: &Value) -> Result<Parts, Value> {
    let mut words = Vec::new();
    if !v.is_undefined() {
        let (it, next) = vm.get_iterator(v, false)?;
        while let Some(x) = vm.iter_step(&it, &next)? {
            match x {
                Value::Str(s) => words.push(s.to_rust()),
                _ => {
                    let e = vm.type_error("Iterable yielded a value that isn't a string");
                    vm.iterator_close(&it)?;
                    return Err(e);
                }
            }
        }
    }
    // (Between two; between the last two of more; between the others.)
    let (two, last, between) = match (o.kind, o.style) {
        (0, 0) => (" and ", ", and ", ", "),
        (0, 1) => (" & ", ", & ", ", "),
        (0, _) => (", ", ", ", ", "),
        (1, _) => (" or ", ", or ", ", "),
        (_, 2) => (" ", " ", " "),
        _ => (", ", ", ", ", "),
    };
    let mut parts: Parts = Vec::new();
    let count = words.len();
    for (i, w) in words.into_iter().enumerate() {
        if i > 0 {
            parts.push(("literal", String::from(if count == 2 { two } else if i == count - 1 { last } else { between })));
        }
        parts.push(("element", w));
    }
    Ok(parts)
}

fn list_format(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let o = this_list(vm, this, "format")?;
    let parts = list_parts(vm, &o, &arg(args, 0))?;
    Ok(js_str(&joined(&parts)))
}

fn list_format_to_parts(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let o = this_list(vm, this, "formatToParts")?;
    let parts = list_parts(vm, &o, &arg(args, 0))?;
    parts_array(vm, &parts, None)
}

fn list_resolved(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let o = this_list(vm, this, "resolvedOptions")?;
    let locale = locale_of(vm, this);
    let items = vec![("locale", js_str(&locale)), ("type", js_str(LIST_KINDS[o.kind as usize])), ("style", js_str(WIDTHS_LSN[o.style as usize]))];
    resolved(vm, items)
}

// -------------------------------------------------------------------------------------------------- Segmenter

const GRANULARITIES: &[&str] = &["grapheme", "word", "sentence"];

fn segmenter_new(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let p = vm.proto_from_constructor(nt, I::IntlSegmenterProto)?;
    let res = resolution(vm, &arg(args, 0))?;
    let o = options_object(vm, &arg(args, 1))?;
    opt_str(vm, o, "localeMatcher", MATCHERS, Some(1))?;
    let g = opt_str(vm, o, "granularity", GRANULARITIES, Some(0))?.unwrap_or(0) as u8;
    Ok(alloc_intl(vm, p, res.locale(), IntlData::Segmenter(g)))
}

fn segmenter_segment(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let gran = match data_of(vm, this) {
        Some(IntlData::Segmenter(g)) => *g,
        _ => return Err(incompatible(vm, "Segmenter.prototype.segment")),
    };
    let s = vm.to_string(&arg(args, 0))?;
    let p = vm.proto(I::IntlSegmentsProto);
    Ok(alloc_intl(vm, p, String::new(), IntlData::Segments(s, gran)))
}

fn segmenter_resolved(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let gran = match data_of(vm, this) {
        Some(IntlData::Segmenter(g)) => *g,
        _ => return Err(incompatible(vm, "Segmenter.prototype.resolvedOptions")),
    };
    let locale = locale_of(vm, this);
    let items = vec![("locale", js_str(&locale)), ("granularity", js_str(GRANULARITIES[gran as usize]))];
    resolved(vm, items)
}

/// The code point at a place in UTF-16 units, and how many units it takes.
fn cp_at(u: &[u16], i: usize) -> (u32, usize) {
    let a = u[i] as u32;
    if (0xD800..0xDC00).contains(&a)
        && let Some(&b) = u.get(i + 1)
        && (0xDC00..0xE000).contains(&(b as u32))
    {
        return (0x10000 + ((a - 0xD800) << 10) + (b as u32 - 0xDC00), 2);
    }
    (a, 1)
}

/// Whether a code point joins the one before it in a grapheme (a combining mark, a vowel sign, a joiner, a variation
/// selector, an emoji's skin tone or tag).
fn is_extend(cp: u32) -> bool {
    matches!(cp,
        0x0300..=0x036F | 0x0483..=0x0489 | 0x0591..=0x05BD | 0x05BF | 0x05C1..=0x05C2 | 0x05C4..=0x05C5 | 0x05C7
        | 0x0610..=0x061A | 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06DC | 0x06DF..=0x06E4 | 0x06E7..=0x06E8
        | 0x06EA..=0x06ED | 0x0900..=0x0903 | 0x093A..=0x093C | 0x093E..=0x094F | 0x0951..=0x0957 | 0x0962..=0x0963
        | 0x0981..=0x0983 | 0x09BC | 0x09BE..=0x09CD | 0x09D7 | 0x09E2..=0x09E3 | 0x0A01..=0x0A03 | 0x0A3C..=0x0A51
        | 0x0A70..=0x0A71 | 0x0A75 | 0x0A81..=0x0A83 | 0x0ABC | 0x0ABE..=0x0ACD | 0x0B01..=0x0B03 | 0x0B3C
        | 0x0B3E..=0x0B57 | 0x0B82 | 0x0BBE..=0x0BCD | 0x0BD7 | 0x0C00..=0x0C04 | 0x0C3E..=0x0C56 | 0x0C81..=0x0C83
        | 0x0CBC | 0x0CBE..=0x0CD6 | 0x0D00..=0x0D03 | 0x0D3B..=0x0D3C | 0x0D3E..=0x0D4D | 0x0D57 | 0x0E31
        | 0x0E34..=0x0E3A | 0x0E47..=0x0E4E | 0x0EB1 | 0x0EB4..=0x0EBC | 0x0EC8..=0x0ECE | 0x1AB0..=0x1AFF
        | 0x1DC0..=0x1DFF | 0x200C..=0x200D | 0x20D0..=0x20FF | 0x302A..=0x302F | 0x3099..=0x309A | 0xFE00..=0xFE0F
        | 0xFE20..=0xFE2F | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F | 0xE0100..=0xE01EF)
}

fn is_regional(cp: u32) -> bool {
    (0x1F1E6..=0x1F1FF).contains(&cp)
}

fn is_pictographic(cp: u32) -> bool {
    matches!(cp, 0xA9 | 0xAE | 0x203C | 0x2049 | 0x2122 | 0x2139 | 0x2194..=0x21AA | 0x231A..=0x23FF | 0x24C2 | 0x25AA..=0x27BF | 0x2934..=0x2935 | 0x2B05..=0x2B55 | 0x3030 | 0x303D | 0x3297 | 0x3299 | 0x1F000..=0x1FAFF)
}

fn is_control(cp: u32) -> bool {
    cp < 0x20 || (0x7F..0xA0).contains(&cp) || cp == 0x2028 || cp == 0x2029
}

/// Where the grapheme starting at `start` ends.
fn grapheme_end(u: &[u16], start: usize) -> usize {
    let (first, l) = cp_at(u, start);
    let mut at = start + l;
    if first == 0x0D {
        return if u.get(at) == Some(&0x0A) { at + 1 } else { at };
    }
    if is_control(first) {
        return at;
    }
    let (mut prev, mut regional) = (first, is_regional(first) as u32);
    while at < u.len() {
        let (cp, l) = cp_at(u, at);
        let joins = if is_regional(cp) { is_regional(prev) && regional % 2 == 1 } else { is_extend(cp) || (prev == 0x200D && is_pictographic(cp)) };
        if !joins {
            break;
        }
        regional += is_regional(cp) as u32;
        prev = cp;
        at += l;
    }
    at
}

/// A character's kind for words (Unicode's Word_Break, roughly).
#[derive(Clone, Copy, PartialEq, Eq)]
enum WordKind {
    Letter,
    Number,
    Katakana,
    Ideograph,
    Joiner,
    MidLetter,
    MidNumLet,
    MidNum,
    Space,
    Other,
}

fn word_kind(cp: u32) -> WordKind {
    use WordKind::*;
    match cp {
        0x5F | 0x203F | 0x2040 | 0x2054 | 0xFE33 | 0xFE34 | 0xFE4D..=0xFE4F | 0xFF3F => Joiner,
        0x3A | 0xB7 | 0x0387 | 0x055F | 0x05F4 | 0x2027 | 0xFE13 | 0xFE55 | 0xFF1A => MidLetter,
        0x27 | 0x2E | 0x2018 | 0x2019 | 0x2024 | 0xFE52 | 0xFF07 | 0xFF0E => MidNumLet,
        0x2C | 0x3B | 0x037E | 0x0589 | 0x060C | 0x060D | 0x066C | 0x07F8 | 0x2044 | 0xFE10 | 0xFE14 | 0xFE50 | 0xFE54 | 0xFF0C | 0xFF1B => MidNum,
        0x20 | 0x1680 | 0x2000..=0x2006 | 0x2008..=0x200A | 0x205F | 0x3000 => Space,
        0x30A0..=0x30FF | 0x31F0..=0x31FF | 0x32D0..=0x32FE | 0x3300..=0x3357 | 0xFF66..=0xFF9F => Katakana,
        0x3040..=0x309F | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x3FFFF => Ideograph,
        _ => match char::from_u32(cp) {
            Some(c) if c.is_numeric() => Number,
            Some(c) if c.is_alphabetic() => Letter,
            _ => Other,
        },
    }
}

/// Where the word (or the space, or the mark) starting at `start` ends, and whether it's word-like.
fn word_end(u: &[u16], start: usize) -> (usize, bool) {
    use WordKind::*;
    let first = word_kind(cp_at(u, start).0);
    let mut end = grapheme_end(u, start);
    match first {
        Space => {
            while end < u.len() && word_kind(cp_at(u, end).0) == Space {
                end = grapheme_end(u, end);
            }
            (end, false)
        }
        Letter | Number | Katakana | Joiner => {
            let mut last = first;
            let mut word_like = first != Joiner;
            while end < u.len() {
                let next = word_kind(cp_at(u, end).0);
                let joins = match (last, next) {
                    (Katakana, Katakana | Joiner) | (Joiner, Katakana) => true,
                    (Letter | Number | Joiner, Letter | Number | Joiner) => true,
                    _ => false,
                };
                if joins {
                    end = grapheme_end(u, end);
                    word_like |= next != Joiner;
                    last = next;
                    continue;
                }
                // A letter or number's own middle ("can't", "3.14", "1,000").
                let after = grapheme_end(u, end);
                if after < u.len() {
                    let third = word_kind(cp_at(u, after).0);
                    let fits = match next {
                        MidLetter => last == Letter && third == Letter,
                        MidNumLet => (last == Letter && third == Letter) || (last == Number && third == Number),
                        MidNum => last == Number && third == Number,
                        _ => false,
                    };
                    if fits {
                        end = grapheme_end(u, after);
                        last = third;
                        continue;
                    }
                }
                break;
            }
            (end, word_like)
        }
        Ideograph => (end, true),
        _ => (end, false),
    }
}

/// Where the sentence starting at `start` ends: after its . ! or ? (and the quotes, brackets and spaces after them),
/// unless what follows goes on with it ("3.14", "U.S.A.", "e.g. the"); or after a line's end.
fn sentence_end(u: &[u16], start: usize) -> usize {
    let is_term = |cp: u32| matches!(cp, 0x21 | 0x3F | 0x2E | 0x2024 | 0x203C | 0x203D | 0x2047..=0x2049 | 0x3002 | 0xFE52 | 0xFE56 | 0xFE57 | 0xFF01 | 0xFF0E | 0xFF1F | 0xFF61);
    let is_close = |cp: u32| matches!(cp, 0x22 | 0x27 | 0x29 | 0x5D | 0x7D | 0xBB | 0x2019 | 0x201D | 0x203A | 0x300D | 0x300F | 0xFF09);
    let is_space = |cp: u32| matches!(cp, 0x09 | 0x20 | 0xA0 | 0x2000..=0x200A | 0x3000);
    let is_sep = |cp: u32| matches!(cp, 0x0A | 0x0D | 0x85 | 0x2028 | 0x2029);
    let ch = |cp: u32| char::from_u32(cp).unwrap_or(' ');
    let mut at = start;
    while at < u.len() {
        let (cp, _) = cp_at(u, at);
        let next = grapheme_end(u, at);
        if is_sep(cp) {
            return next;
        }
        if !is_term(cp) {
            at = next;
            continue;
        }
        let period = matches!(cp, 0x2E | 0x2024 | 0xFE52 | 0xFF0E);
        // Right after a period: a digit ("3.14"), or a capital after a letter ("U.S"), goes on.
        if period && next < u.len() {
            let c2 = ch(cp_at(u, next).0);
            let before = (at > start).then(|| ch(u[at - 1] as u32));
            if c2.is_ascii_digit() || (c2.is_uppercase() && before.is_some_and(char::is_alphabetic)) {
                at = next;
                continue;
            }
        }
        let mut end = next;
        while end < u.len() && is_term(cp_at(u, end).0) {
            end = grapheme_end(u, end);
        }
        while end < u.len() && is_close(cp_at(u, end).0) {
            end = grapheme_end(u, end);
        }
        while end < u.len() && is_space(cp_at(u, end).0) {
            end = grapheme_end(u, end);
        }
        if end < u.len() && is_sep(cp_at(u, end).0) {
            return grapheme_end(u, end);
        }
        // What follows goes on with the sentence if it starts with , ; or : or, after a period, if a lower case letter
        // comes before any other letter.
        let mut look = end;
        let mut goes_on = false;
        while look < u.len() {
            let c = cp_at(u, look).0;
            if matches!(c, 0x2C | 0x3B | 0x3A) && look == end {
                goes_on = true;
                break;
            }
            if ch(c).is_lowercase() {
                goes_on = period;
                break;
            }
            if ch(c).is_alphabetic() || is_term(c) || is_sep(c) || ch(c).is_ascii_digit() {
                break;
            }
            look = grapheme_end(u, look);
        }
        if goes_on {
            at = end;
            continue;
        }
        return end;
    }
    u.len()
}

/// The segment starting at `start`: where it ends, and whether it's word-like (words only).
fn segment_at(u: &[u16], gran: u8, start: usize) -> (usize, bool) {
    let (end, word_like) = match gran {
        1 => word_end(u, start),
        2 => (sentence_end(u, start), false),
        _ => (grapheme_end(u, start), false),
    };
    (end.max(start + 1).min(u.len()), word_like)
}

/// A segment's data: { segment, index, input } (and isWordLike, for words).
fn segment_object(vm: &mut Vm, s: &JsStr, start: usize, end: usize, word_like: bool, gran: u8) -> Value {
    let o = vm.new_object();
    let piece = JsStr::from_units(s.units()[start..end].to_vec());
    vm.put(o, "segment", Value::Str(piece), ALL);
    vm.put(o, "index", Value::Num(start as f64), ALL);
    vm.put(o, "input", Value::Str(s.clone()), ALL);
    if gran == 1 {
        vm.put(o, "isWordLike", Value::Bool(word_like), ALL);
    }
    Value::Obj(o)
}

fn segments_containing(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (s, gran) = match data_of(vm, this) {
        Some(IntlData::Segments(s, g)) => (s.clone(), *g),
        _ => return Err(incompatible(vm, "Segments.prototype.containing")),
    };
    let k = vm.to_integer(&arg(args, 0))?;
    let len = s.units().len();
    if k < 0.0 || k >= len as f64 {
        return Ok(Value::Undefined);
    }
    let k = k as usize;
    let mut start = 0;
    loop {
        let (end, word_like) = segment_at(s.units(), gran, start);
        if k < end {
            return Ok(segment_object(vm, &s, start, end, word_like, gran));
        }
        start = end;
    }
}

fn segments_iterator(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (s, gran) = match data_of(vm, this) {
        Some(IntlData::Segments(s, g)) => (s.clone(), *g),
        _ => return Err(incompatible(vm, "Segments.prototype[Symbol.iterator]")),
    };
    let p = vm.proto(I::IntlSegmentIterProto);
    Ok(alloc_intl(vm, p, String::new(), IntlData::SegmentIter(s, gran, 0)))
}

fn segment_iter_next(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (s, gran, at) = match data_of(vm, this) {
        Some(IntlData::SegmentIter(s, g, at)) => (s.clone(), *g, *at),
        _ => return Err(incompatible(vm, "SegmentIterator.prototype.next")),
    };
    let result = vm.new_object();
    if at >= s.units().len() {
        vm.put(result, "value", Value::Undefined, ALL);
        vm.put(result, "done", Value::Bool(true), ALL);
        return Ok(Value::Obj(result));
    }
    let (end, word_like) = segment_at(s.units(), gran, at);
    if let Value::Obj(g) = this
        && let Class::Intl(i) = &mut vm.heap.get_mut(*g).class
        && let IntlData::SegmentIter(_, _, pos) = &mut i.data
    {
        *pos = end;
    }
    let v = segment_object(vm, &s, at, end, word_like, gran);
    vm.put(result, "value", v, ALL);
    vm.put(result, "done", Value::Bool(false), ALL);
    Ok(Value::Obj(result))
}

// ----------------------------------------------------------------------------------------------- DisplayNames

const REGIONS: &[(&str, &str)] = &[
    ("AE", "United Arab Emirates"), ("AR", "Argentina"), ("AT", "Austria"), ("AU", "Australia"), ("BD", "Bangladesh"),
    ("BE", "Belgium"), ("BR", "Brazil"), ("CA", "Canada"), ("CH", "Switzerland"), ("CL", "Chile"), ("CN", "China"),
    ("CO", "Colombia"), ("CZ", "Czechia"), ("DE", "Germany"), ("DK", "Denmark"), ("EG", "Egypt"), ("ES", "Spain"),
    ("FI", "Finland"), ("FR", "France"), ("GB", "United Kingdom"), ("GR", "Greece"), ("HK", "Hong Kong SAR China"),
    ("HU", "Hungary"), ("ID", "Indonesia"), ("IE", "Ireland"), ("IL", "Israel"), ("IN", "India"), ("IT", "Italy"),
    ("JP", "Japan"), ("KE", "Kenya"), ("KR", "South Korea"), ("MX", "Mexico"), ("MY", "Malaysia"), ("NG", "Nigeria"),
    ("NL", "Netherlands"), ("NO", "Norway"), ("NZ", "New Zealand"), ("PE", "Peru"), ("PH", "Philippines"),
    ("PK", "Pakistan"), ("PL", "Poland"), ("PT", "Portugal"), ("RO", "Romania"), ("RU", "Russia"),
    ("SA", "Saudi Arabia"), ("SE", "Sweden"), ("SG", "Singapore"), ("TH", "Thailand"), ("TR", "Türkiye"),
    ("TW", "Taiwan"), ("UA", "Ukraine"), ("US", "United States"), ("VN", "Vietnam"), ("ZA", "South Africa"),
    ("001", "world"), ("150", "Europe"), ("419", "Latin America"),
];

const LANGUAGES: &[(&str, &str)] = &[
    ("ar", "Arabic"), ("bn", "Bangla"), ("cs", "Czech"), ("da", "Danish"), ("de", "German"), ("de-AT", "Austrian German"),
    ("de-CH", "Swiss High German"), ("el", "Greek"), ("en", "English"), ("en-AU", "Australian English"),
    ("en-CA", "Canadian English"), ("en-GB", "British English"), ("en-US", "American English"), ("es", "Spanish"),
    ("es-419", "Latin American Spanish"), ("es-ES", "European Spanish"), ("es-MX", "Mexican Spanish"), ("fa", "Persian"),
    ("fi", "Finnish"), ("fil", "Filipino"), ("fr", "French"), ("fr-CA", "Canadian French"), ("fr-CH", "Swiss French"),
    ("he", "Hebrew"), ("hi", "Hindi"), ("hu", "Hungarian"), ("id", "Indonesian"), ("it", "Italian"), ("ja", "Japanese"),
    ("ko", "Korean"), ("ms", "Malay"), ("nb", "Norwegian Bokmål"), ("nl", "Dutch"), ("nl-BE", "Flemish"), ("no", "Norwegian"),
    ("pl", "Polish"), ("pt", "Portuguese"), ("pt-BR", "Brazilian Portuguese"), ("pt-PT", "European Portuguese"),
    ("ro", "Romanian"), ("ru", "Russian"), ("sv", "Swedish"), ("sw", "Swahili"), ("ta", "Tamil"), ("th", "Thai"),
    ("tl", "Tagalog"), ("tr", "Turkish"), ("uk", "Ukrainian"), ("ur", "Urdu"), ("vi", "Vietnamese"), ("zh", "Chinese"),
    ("zh-Hans", "Simplified Chinese"), ("zh-Hant", "Traditional Chinese"),
];

const SCRIPTS: &[(&str, &str)] = &[
    ("Arab", "Arabic"), ("Beng", "Bangla"), ("Cyrl", "Cyrillic"), ("Deva", "Devanagari"), ("Grek", "Greek"),
    ("Hang", "Hangul"), ("Hani", "Han"), ("Hans", "Simplified Han"), ("Hant", "Traditional Han"), ("Hebr", "Hebrew"),
    ("Hira", "Hiragana"), ("Jpan", "Japanese"), ("Kana", "Katakana"), ("Kore", "Korean"), ("Latn", "Latin"),
    ("Taml", "Tamil"), ("Thai", "Thai"),
];

const FIELDS: &[(&str, &str)] = &[
    ("era", "era"), ("year", "year"), ("quarter", "quarter"), ("month", "month"), ("weekOfYear", "week"),
    ("weekday", "day of the week"), ("day", "day"), ("dayPeriod", "AM/PM"), ("hour", "hour"), ("minute", "minute"),
    ("second", "second"), ("timeZoneName", "time zone"),
];

const DISPLAY_KINDS: &[&str] = &["language", "region", "script", "currency", "calendar", "dateTimeField"];

fn display_names_new(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let p = vm.proto_from_constructor(nt, I::IntlDisplayNamesProto)?;
    let res = resolution(vm, &arg(args, 0))?;
    let Some(o) = options_object(vm, &arg(args, 1))? else { return Err(vm.type_error("Intl.DisplayNames needs options with a type")) };
    opt_str(vm, Some(o), "localeMatcher", MATCHERS, Some(1))?;
    let style = opt_str(vm, Some(o), "style", &["narrow", "short", "long"], Some(2))?.unwrap_or(2) as u8;
    let Some(kind) = opt_str(vm, Some(o), "type", DISPLAY_KINDS, None)? else { return Err(vm.type_error("Required option 'type' is missing")) };
    let fallback_none = opt_str(vm, Some(o), "fallback", &["code", "none"], Some(0))? == Some(1);
    let standard = opt_str(vm, Some(o), "languageDisplay", &["dialect", "standard"], Some(0))? == Some(1);
    Ok(alloc_intl(vm, p, res.locale(), IntlData::DisplayNames(kind as u8, style, fallback_none, standard)))
}

/// A currency's name as DisplayNames gives it ("US Dollar").
fn currency_title(name: &str) -> String {
    let words: Vec<String> = name
        .split(' ')
        .map(|w| {
            let mut ch = w.chars();
            match ch.next() {
                Some(f) => f.to_uppercase().chain(ch).collect::<String>(),
                None => String::new(),
            }
        })
        .collect();
    words.join(" ")
}

fn display_names_of(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (kind, none, standard) = match data_of(vm, this) {
        Some(IntlData::DisplayNames(k, _, none, standard)) => (*k, *none, *standard),
        _ => return Err(incompatible(vm, "DisplayNames.prototype.of")),
    };
    let code = vm.to_string(&arg(args, 0))?.to_rust();
    let find = |table: &[(&str, &str)], k: &str| table.iter().find(|(c, _)| c.eq_ignore_ascii_case(k)).map(|(_, v)| String::from(*v));
    let alpha = |n: usize| code.len() == n && code.bytes().all(|b| b.is_ascii_alphabetic());
    let (found, valid) = match kind {
        0 => match canonical(&code).filter(|_| !code.split('-').any(|p| p.len() == 1)) {
            Some(c) => {
                let dialect = if standard { None } else { find(LANGUAGES, &c) };
                let name = dialect.or_else(|| {
                    // "German (Switzerland)": the language, and the rest of the tag.
                    let (l, s, r) = subtags(&c);
                    let lang = find(LANGUAGES, &l)?;
                    let more: Vec<String> = [s.and_then(|s| find(SCRIPTS, &s)), r.and_then(|r| find(REGIONS, &r))].into_iter().flatten().collect();
                    Some(if more.is_empty() { lang } else { format!("{lang} ({})", more.join(", ")) })
                });
                (name, true)
            }
            None => (None, false),
        },
        1 => (find(REGIONS, &code), alpha(2) || (code.len() == 3 && code.bytes().all(|b| b.is_ascii_digit()))),
        2 => (find(SCRIPTS, &code), alpha(4)),
        3 => (CURRENCIES.iter().find(|c| c.0.eq_ignore_ascii_case(&code)).map(|c| currency_title(c.4)), alpha(3)),
        4 => ((code == "gregory").then(|| String::from("Gregorian Calendar")), code.split('-').all(|p| (3..=8).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric()))),
        _ => (find(FIELDS, &code), FIELDS.iter().any(|(c, _)| *c == code)),
    };
    if !valid {
        return Err(vm.range_error(&format!("invalid code for Intl.DisplayNames.prototype.of: {code}")));
    }
    Ok(match found {
        Some(f) => js_str(&f),
        None if none => Value::Undefined,
        None if kind == 1 || kind == 3 => js_str(&code.to_ascii_uppercase()),
        None => js_str(&canonical(&code).unwrap_or(code)),
    })
}

fn display_names_resolved(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (kind, style, none, standard) = match data_of(vm, this) {
        Some(IntlData::DisplayNames(k, s, none, standard)) => (*k, *s, *none, *standard),
        _ => return Err(incompatible(vm, "DisplayNames.prototype.resolvedOptions")),
    };
    let locale = locale_of(vm, this);
    let mut items = vec![
        ("locale", js_str(&locale)),
        ("style", js_str(["narrow", "short", "long"][style as usize])),
        ("type", js_str(DISPLAY_KINDS[kind as usize])),
        ("fallback", js_str(if none { "none" } else { "code" })),
    ];
    if kind == 0 {
        items.push(("languageDisplay", js_str(if standard { "standard" } else { "dialect" })));
    }
    resolved(vm, items)
}

// ---------------------------------------------------------------------------------------------------- Locale

/// A tag put together from its language, script and region, and its extensions.
fn tag_of(l: String, s: Option<String>, r: Option<String>, ext: &str) -> String {
    let mut t = l;
    for x in [s, r].into_iter().flatten() {
        t.push('-');
        t.push_str(&x);
    }
    t.push_str(ext);
    t
}

/// A tag's extensions ("-u-ca-gregory"), from where they start.
fn extensions(tag: &str) -> &str {
    let mut at = 0;
    for p in tag.split('-') {
        if p.len() == 1 && at > 0 {
            return &tag[at - 1..];
        }
        at += p.len() + 1;
    }
    ""
}

fn locale_new(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let p = vm.proto_from_constructor(nt, I::IntlLocaleProto)?;
    let first = arg(args, 0);
    let tag = match (&first, data_of(vm, &first)) {
        (_, Some(IntlData::Locale(l))) => l.clone(),
        (Value::Str(s), _) => s.to_rust(),
        (Value::Obj(_), _) => vm.to_string(&first)?.to_rust(),
        _ => return Err(vm.type_error("First argument to Intl.Locale constructor can't be empty or missing")),
    };
    let Some(mut c) = canonical(&tag) else { return Err(vm.range_error(&format!("Incorrect locale information provided: {tag}"))) };
    // Options that change the tag: its language, script and region.
    let o = options(vm, &arg(args, 1))?;
    let (mut l, mut s, mut r) = subtags(&c);
    let mut changed = false;
    if let Some(v) = opt_text(vm, o, "language")? {
        (l, changed) = (v, true);
    }
    if let Some(v) = opt_text(vm, o, "script")? {
        (s, changed) = (Some(v), true);
    }
    if let Some(v) = opt_text(vm, o, "region")? {
        (r, changed) = (Some(v), true);
    }
    if changed {
        let t = tag_of(l, s, r, extensions(&c));
        c = match canonical(&t) {
            Some(c) => c,
            None => return Err(vm.range_error(&format!("Incorrect locale information provided: {t}"))),
        };
    }
    // Options for the -u- keywords: calendar, collation, hourCycle, caseFirst, numeric, numberingSystem.
    let mut set: Vec<(&str, String)> = Vec::new();
    for (name, key) in [("calendar", "ca"), ("collation", "co")] {
        if let Some(v) = opt_keyword(vm, o, name)? {
            set.push((key, v));
        }
    }
    if let Some(i) = opt_str(vm, o, "hourCycle", CYCLES, None)? {
        set.push(("hc", String::from(CYCLES[i])));
    }
    if let Some(i) = opt_str(vm, o, "caseFirst", CASE_FIRSTS, None)? {
        set.push(("kf", String::from(CASE_FIRSTS[i])));
    }
    if let Some(b) = opt_bool(vm, o, "numeric")? {
        set.push(("kn", String::from(if b { "true" } else { "false" })));
    }
    if let Some(v) = opt_keyword(vm, o, "numberingSystem")? {
        set.push(("nu", v));
    }
    if !set.is_empty() {
        c = with_keywords(&c, &set);
    }
    Ok(alloc_intl(vm, p, String::new(), IntlData::Locale(c)))
}

fn this_locale(vm: &mut Vm, this: &Value) -> Result<String, Value> {
    match data_of(vm, this) {
        Some(IntlData::Locale(l)) => Ok(l.clone()),
        _ => Err(incompatible(vm, "Locale.prototype method")),
    }
}

fn locale_to_string(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(js_str(&this_locale(vm, this)?))
}

/// The likely script and region of the common languages (Locale's maximize and minimize).
const LIKELY: &[(&str, &str, &str)] = &[
    ("ar", "Arab", "EG"), ("de", "Latn", "DE"), ("en", "Latn", "US"), ("es", "Latn", "ES"), ("fr", "Latn", "FR"),
    ("hi", "Deva", "IN"), ("id", "Latn", "ID"), ("it", "Latn", "IT"), ("ja", "Jpan", "JP"), ("ko", "Kore", "KR"),
    ("ms", "Latn", "MY"), ("nl", "Latn", "NL"), ("pt", "Latn", "BR"), ("ru", "Cyrl", "RU"), ("th", "Thai", "TH"),
    ("tr", "Latn", "TR"), ("vi", "Latn", "VN"), ("zh", "Hans", "CN"),
];

fn locale_maximize(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let tag = this_locale(vm, this)?;
    let (l, s, r) = subtags(&tag);
    let likely = LIKELY.iter().find(|x| x.0 == l);
    let s = s.or_else(|| likely.map(|x| String::from(x.1)));
    let r = r.or_else(|| likely.map(|x| String::from(x.2)));
    let t = tag_of(l, s, r, extensions(&tag));
    let p = vm.proto(I::IntlLocaleProto);
    Ok(alloc_intl(vm, p, String::new(), IntlData::Locale(t)))
}

fn locale_minimize(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let tag = this_locale(vm, this)?;
    let (l, s, r) = subtags(&tag);
    let likely = LIKELY.iter().find(|x| x.0 == l);
    let s = s.filter(|s| likely.is_none_or(|x| x.1 != s));
    let r = r.filter(|r| likely.is_none_or(|x| x.2 != r));
    let t = tag_of(l, s, r, extensions(&tag));
    let p = vm.proto(I::IntlLocaleProto);
    Ok(alloc_intl(vm, p, String::new(), IntlData::Locale(t)))
}

/// A tag's language, script and region (those before its extensions).
fn subtags(tag: &str) -> (String, Option<String>, Option<String>) {
    let mut parts = tag.split('-');
    let lang = String::from(parts.next().unwrap_or(""));
    let (mut script, mut region) = (None, None);
    for p in parts {
        if p.len() == 1 {
            break;
        }
        if p.len() == 4 && p.bytes().all(|b| b.is_ascii_alphabetic()) && script.is_none() && region.is_none() {
            script = Some(String::from(p));
        } else if region.is_none() && ((p.len() == 2 && p.bytes().all(|b| b.is_ascii_alphabetic())) || (p.len() == 3 && p.bytes().all(|b| b.is_ascii_digit()))) {
            region = Some(String::from(p));
        }
    }
    (lang, script, region)
}

/// A tag with these -u- keywords put in (in place of any it had).
fn with_keywords(tag: &str, set: &[(&str, String)]) -> String {
    let (head, private) = match tag.find("-x-") {
        Some(i) => (&tag[..i], &tag[i..]),
        None => (tag, ""),
    };
    let mut keywords: Vec<(String, String)> = Vec::new();
    let mut attributes: Vec<String> = Vec::new();
    let mut others = String::new();
    let base_end = head.find("-u-").unwrap_or(head.len());
    let mut base = String::from(&head[..base_end]);
    if base_end < head.len() {
        let rest = &head[base_end + 3..];
        let mut words = rest.split('-').peekable();
        while let Some(w) = words.peek() {
            if w.len() <= 2 {
                break;
            }
            attributes.push(String::from(*w));
            words.next();
        }
        while let Some(k) = words.next() {
            if k.len() == 1 {
                // (Another extension after -u-.)
                others.push('-');
                others.push_str(k);
                for w in words.by_ref() {
                    others.push('-');
                    others.push_str(w);
                }
                break;
            }
            let mut values: Vec<&str> = Vec::new();
            while let Some(v) = words.peek() {
                if v.len() <= 2 {
                    break;
                }
                values.push(v);
                words.next();
            }
            keywords.push((String::from(k), values.join("-")));
        }
    }
    for (k, v) in set {
        keywords.retain(|(x, _)| x != k);
        keywords.push((String::from(*k), v.clone()));
    }
    let mut u: Vec<String> = attributes;
    for (k, v) in keywords {
        u.push(k);
        if !v.is_empty() {
            u.push(v);
        }
    }
    base.push_str("-u-");
    base.push_str(&u.join("-"));
    base.push_str(&others);
    base.push_str(private);
    canonical(&base).unwrap_or(base)
}

/// A -u- extension keyword's value ("gregory" for "ca" in "en-US-u-ca-gregory").
fn keyword(tag: &str, key: &str) -> Option<String> {
    let i = tag.find("-u-")?;
    let parts: Vec<&str> = tag[i + 3..].split('-').collect();
    let at = parts.iter().position(|p| *p == key)?;
    let v: Vec<&str> = parts[at + 1..].iter().take_while(|p| p.len() > 2).copied().collect();
    Some(if v.is_empty() { String::from("true") } else { v.join("-") })
}

fn locale_base_name(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (l, s, r) = subtags(&this_locale(vm, this)?);
    Ok(js_str(&tag_of(l, s, r, "")))
}

fn locale_language(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(js_str(&subtags(&this_locale(vm, this)?).0))
}

fn locale_region(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(subtags(&this_locale(vm, this)?).2.map_or(Value::Undefined, |r| js_str(&r)))
}

fn locale_script(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(subtags(&this_locale(vm, this)?).1.map_or(Value::Undefined, |s| js_str(&s)))
}

fn locale_calendar(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(keyword(&this_locale(vm, this)?, "ca").map_or(Value::Undefined, |s| js_str(&s)))
}

fn locale_case_first(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(keyword(&this_locale(vm, this)?, "kf").map_or(Value::Undefined, |s| js_str(if s == "true" { "" } else { &s })))
}

fn locale_collation(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(keyword(&this_locale(vm, this)?, "co").map_or(Value::Undefined, |s| js_str(&s)))
}

fn locale_numeric(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Bool(keyword(&this_locale(vm, this)?, "kn").is_some_and(|s| s == "true")))
}

fn locale_hour_cycle(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(keyword(&this_locale(vm, this)?, "hc").map_or(Value::Undefined, |s| js_str(&s)))
}

fn locale_numbering(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(keyword(&this_locale(vm, this)?, "nu").map_or(Value::Undefined, |s| js_str(&s)))
}

#[cfg(test)]
mod tests {
    use crate::value::Value;
    use crate::vm::Vm;
    use alloc::string::String;

    /// What a script gives, as a string (its exception's, if it throws).
    fn run(vm: &mut Vm, src: &str) -> String {
        match vm.run_script(src, "test") {
            Ok(v) => vm.to_string(&v).map(|s| s.to_rust()).unwrap_or_default(),
            Err(e) => {
                let v = if let Value::Obj(_) = e { vm.to_string(&e).map(|s| s.to_rust()).unwrap_or_default() } else { String::from("?") };
                alloc::format!("threw {v}")
            }
        }
    }

    fn check(cases: &[(&str, &str)]) {
        let mut vm = Vm::new();
        vm.now = || 1_794_000_000_000.0;
        vm.tz_offset = 480;
        for (src, want) in cases {
            assert_eq!(run(&mut vm, src), *want, "{src}");
        }
    }

    #[test]
    fn numbers_are_written_as_en_us_writes_them() {
        check(&[
            ("new Intl.NumberFormat().format(1234567.891)", "1,234,567.891"),
            ("(1234.5).toLocaleString()", "1,234.5"),
            ("(1234.5).toLocaleString('de-DE')", "1,234.5"),
            ("(0.1 + 0.2).toLocaleString()", "0.3"),
            ("(1.005).toLocaleString(undefined, { maximumFractionDigits: 2 })", "1.01"),
            ("(-0).toLocaleString()", "-0"),
            ("(1e21).toLocaleString()", "1,000,000,000,000,000,000,000"),
            ("(12345678901234567890n).toLocaleString()", "12,345,678,901,234,567,890"),
            ("new Intl.NumberFormat().format('12345678901234567890.5')", "12,345,678,901,234,567,890.5"),
            ("(1234.5678).toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 })", "1,234.57"),
            ("new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD' }).format(-1234.5)", "-$1,234.50"),
            ("new Intl.NumberFormat('en-US', { style: 'currency', currency: 'EUR' }).format(5)", "€5.00"),
            ("new Intl.NumberFormat('en-US', { style: 'currency', currency: 'JPY' }).format(1234.5)", "¥1,235"),
            ("new Intl.NumberFormat('en-US', { style: 'currency', currency: 'SGD' }).format(5)", "SGD\u{a0}5.00"),
            ("new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', currencySign: 'accounting' }).format(-5)", "($5.00)"),
            ("new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', currencyDisplay: 'name' }).format(1)", "1.00 US dollars"),
            ("new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', currencyDisplay: 'code' }).format(1)", "USD\u{a0}1.00"),
            ("new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', maximumFractionDigits: 0 }).format(1234.5)", "$1,235"),
            ("new Intl.NumberFormat('en-US', { style: 'percent' }).format(0.256)", "26%"),
            ("new Intl.NumberFormat('en-US', { style: 'percent', maximumFractionDigits: 1 }).format(0.0123456)", "1.2%"),
            ("new Intl.NumberFormat('en-US', { style: 'unit', unit: 'kilometer' }).format(5)", "5 km"),
            ("new Intl.NumberFormat('en-US', { style: 'unit', unit: 'kilometer', unitDisplay: 'long' }).format(1)", "1 kilometer"),
            ("new Intl.NumberFormat('en-US', { style: 'unit', unit: 'kilometer-per-hour', unitDisplay: 'long' }).format(50)", "50 kilometers per hour"),
            ("new Intl.NumberFormat('en-US', { style: 'unit', unit: 'megabyte', unitDisplay: 'narrow' }).format(3.5)", "3.5MB"),
            ("new Intl.NumberFormat('en-US', { style: 'unit', unit: 'celsius' }).format(21)", "21°C"),
            ("new Intl.NumberFormat('en-US', { notation: 'compact' }).format(1234)", "1.2K"),
            ("new Intl.NumberFormat('en-US', { notation: 'compact' }).format(12345)", "12K"),
            ("new Intl.NumberFormat('en-US', { notation: 'compact' }).format(999999)", "1M"),
            ("new Intl.NumberFormat('en-US', { notation: 'compact' }).format(1500000000)", "1.5B"),
            ("new Intl.NumberFormat('en-US', { notation: 'compact', compactDisplay: 'long' }).format(2500000)", "2.5 million"),
            ("new Intl.NumberFormat('en-US', { notation: 'compact' }).format(999)", "999"),
            ("new Intl.NumberFormat('en-US', { notation: 'compact' }).format(0.01234)", "0.012"),
            ("new Intl.NumberFormat('en-US', { notation: 'scientific' }).format(123456)", "1.235E5"),
            ("new Intl.NumberFormat('en-US', { notation: 'scientific' }).format(0.00123)", "1.23E-3"),
            ("new Intl.NumberFormat('en-US', { notation: 'engineering' }).format(123456)", "123.456E3"),
            ("new Intl.NumberFormat('en-US', { notation: 'scientific' }).format(0)", "0E0"),
            ("new Intl.NumberFormat('en-US', { signDisplay: 'always' }).format(5)", "+5"),
            ("new Intl.NumberFormat('en-US', { signDisplay: 'exceptZero' }).format(0)", "0"),
            ("new Intl.NumberFormat('en-US', { minimumIntegerDigits: 3 }).format(5)", "005"),
            ("new Intl.NumberFormat('en-US', { maximumSignificantDigits: 3 }).format(123456)", "123,000"),
            ("new Intl.NumberFormat('en-US', { minimumSignificantDigits: 3 }).format(1)", "1.00"),
            ("new Intl.NumberFormat('en-US', { useGrouping: false }).format(12345)", "12345"),
            ("new Intl.NumberFormat('en-US', { roundingMode: 'floor', maximumFractionDigits: 0 }).format(-1.5)", "-2"),
            ("new Intl.NumberFormat('en-US', { roundingMode: 'halfEven', maximumFractionDigits: 0 }).format(2.5)", "2"),
            ("new Intl.NumberFormat('en-US', { maximumFractionDigits: 2, minimumFractionDigits: 2, roundingIncrement: 5 }).format(1.23)", "1.25"),
            ("new Intl.NumberFormat('en-US', { minimumFractionDigits: 2, trailingZeroDisplay: 'stripIfInteger' }).format(5)", "5"),
            ("new Intl.NumberFormat().format(NaN) + ' ' + new Intl.NumberFormat().format(-Infinity)", "NaN -∞"),
            ("new Intl.NumberFormat().formatToParts(-1234.5).map(p => p.type + ':' + p.value).join('|')", "minusSign:-|integer:1|group:,|integer:234|decimal:.|fraction:5"),
            ("new Intl.NumberFormat().formatRange(3, 5)", "3–5"),
            ("new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', maximumFractionDigits: 0 }).formatRange(3, 5)", "$3 – $5"),
            ("new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', signDisplay: 'always' }).formatRange(2.9, 3.1)", "+$2.90–3.10"),
            ("new Intl.NumberFormat('en-US', { style: 'unit', unit: 'kilometer' }).formatRange(3, 5)", "3–5 km"),
            ("new Intl.NumberFormat().formatRange(-5, -3)", "-5 – -3"),
            (
                "JSON.stringify(new Intl.NumberFormat().resolvedOptions())",
                r#"{"locale":"en-US","numberingSystem":"latn","style":"decimal","minimumIntegerDigits":1,"minimumFractionDigits":0,"maximumFractionDigits":3,"useGrouping":"auto","notation":"standard","signDisplay":"auto","roundingIncrement":1,"roundingMode":"halfExpand","roundingPriority":"auto","trailingZeroDisplay":"auto"}"#,
            ),
            ("const f = new Intl.NumberFormat(); f.format === f.format && [1, 2].map(f.format).join()", "1,2"),
            ("try { new Intl.NumberFormat('en', { style: 'currency' }) } catch (e) { e.name }", "TypeError"),
            ("try { new Intl.NumberFormat('en_US') } catch (e) { e.name }", "RangeError"),
            ("Intl.NumberFormat.supportedLocalesOf(['en-GB', 'de']).join()", "en-GB"),
            ("Object.prototype.toString.call(new Intl.NumberFormat())", "[object Intl.NumberFormat]"),
            ("Intl.NumberFormat() instanceof Intl.NumberFormat", "true"),
        ]);
    }

    #[test]
    fn dates_and_times_are_written_in_their_zone() {
        // 2026-11-06 09:04:05.123 UTC; the PC's zone 8 hours ahead.
        check(&[
            ("var d = new Date(Date.UTC(2026, 10, 6, 9, 4, 5, 123)); d.toLocaleString()", "11/6/2026, 5:04:05 PM"),
            ("d.toLocaleDateString()", "11/6/2026"),
            ("d.toLocaleTimeString()", "5:04:05 PM"),
            ("d.toLocaleString('en-US', { timeZone: 'UTC' })", "11/6/2026, 9:04:05 AM"),
            ("d.toLocaleString('en-US', { timeZone: 'America/New_York' })", "11/6/2026, 4:04:05 AM"),
            ("new Date(Date.UTC(2026, 6, 6, 9)).toLocaleString('en-US', { timeZone: 'America/New_York', timeZoneName: 'short' })", "7/6/2026, 5:00:00 AM EDT"),
            ("new Date(Date.UTC(2026, 6, 6, 9)).toLocaleString('en-US', { timeZone: 'Europe/London', timeZoneName: 'long' })", "7/6/2026, 10:00:00 AM British Summer Time"),
            ("new Date(Date.UTC(2026, 0, 6, 9)).toLocaleString('en-US', { timeZone: 'Australia/Sydney', hour: 'numeric', timeZoneName: 'shortOffset' })", "8 PM GMT+11"),
            ("d.toLocaleDateString('en-US', { dateStyle: 'full' })", "Friday, November 6, 2026"),
            ("d.toLocaleString('en-US', { dateStyle: 'long', timeStyle: 'short' })", "November 6, 2026 at 5:04 PM"),
            ("d.toLocaleString('en-US', { dateStyle: 'medium', timeStyle: 'medium' })", "Nov 6, 2026, 5:04:05 PM"),
            ("d.toLocaleDateString('en-US', { dateStyle: 'short' })", "11/6/26"),
            ("d.toLocaleDateString('en-US', { weekday: 'short', month: 'short', day: 'numeric' })", "Fri, Nov 6"),
            ("d.toLocaleDateString('en-US', { month: 'long', year: 'numeric' })", "November 2026"),
            ("d.toLocaleDateString('en-US', { year: 'numeric', month: '2-digit', day: '2-digit' })", "11/06/2026"),
            ("d.toLocaleDateString('en-US', { weekday: 'long' })", "Friday"),
            ("d.toLocaleTimeString('en-US', { hour: '2-digit', minute: '2-digit' })", "05:04 PM"),
            ("d.toLocaleTimeString('en-US', { hour12: false })", "17:04:05"),
            ("d.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit', hourCycle: 'h23', timeZone: 'UTC' })", "09:04"),
            ("d.toLocaleTimeString('en-US', { timeStyle: 'full', timeZone: 'Asia/Singapore' })", "5:04:05 PM Singapore Standard Time"),
            ("d.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit', second: '2-digit', fractionalSecondDigits: 3 })", "5:04:05.123 PM"),
            ("new Intl.DateTimeFormat('en-US', { month: 'short' }).formatToParts(d).map(p => p.type).join()", "month"),
            ("Intl.DateTimeFormat().resolvedOptions().timeZone", "Etc/GMT-8"),
            ("new Intl.DateTimeFormat().format(d)", "11/6/2026"),
            ("new Intl.DateTimeFormat('en-US', { timeZone: 'asia/tokyo' }).resolvedOptions().timeZone", "Asia/Tokyo"),
            ("try { new Intl.DateTimeFormat('en-US', { timeZone: 'Mars/Base' }) } catch (e) { e.name }", "RangeError"),
            ("try { d.toLocaleDateString('en-US', { timeStyle: 'short' }) } catch (e) { e.name }", "TypeError"),
            ("new Date(NaN).toLocaleString()", "Invalid Date"),
            ("try { new Intl.DateTimeFormat().format(NaN) } catch (e) { e.name }", "RangeError"),
            ("new Intl.DateTimeFormat('en-US', { dateStyle: 'medium' }).formatRange(d, new Date(Date.UTC(2026, 10, 9)))", "Nov 6 – 9, 2026"),
            ("new Intl.DateTimeFormat('en-US', { timeStyle: 'short', timeZone: 'UTC' }).formatRange(Date.UTC(2026, 0, 1, 10), Date.UTC(2026, 0, 1, 11, 30))", "10:00 – 11:30 AM"),
            ("new Intl.DateTimeFormat('en-US', { dateStyle: 'medium', timeStyle: 'short', timeZone: 'UTC' }).formatRange(Date.UTC(2026, 0, 1, 10), Date.UTC(2026, 0, 1, 14))", "Jan 1, 2026, 10:00 AM – 2:00 PM"),
            ("new Intl.DateTimeFormat('en-US', { month: 'short', day: 'numeric', year: 'numeric' }).formatRange(new Date(2019, 0, 3), new Date(2019, 2, 4))", "Jan 3 – Mar 4, 2019"),
            ("new Intl.DateTimeFormat('en-US', { hour: 'numeric', dayPeriod: 'long', timeZone: 'UTC' }).format(Date.UTC(2026, 0, 1, 12))", "12 noon"),
            ("new Intl.NumberFormat('en-US', { numberingSystem: 'arab' }).format(1234.5)", "١,٢٣٤.٥"),
            ("new Intl.DateTimeFormat('en-US-u-nu-thai', { timeZone: 'UTC' }).format(0)", "๑/๑/๑๙๗๐"),
            ("new Intl.DateTimeFormat('en-u-hc-h23', { hour: 'numeric', minute: 'numeric', timeZone: 'UTC' }).format(Date.UTC(2026, 0, 1, 15, 5))", "15:05"),
            ("new Intl.DateTimeFormat('en-u-hc-h23', { hour: 'numeric' }).resolvedOptions().locale", "en-u-hc-h23"),
        ]);
    }

    #[test]
    fn plurals_relative_times_and_lists() {
        check(&[
            ("var p = new Intl.PluralRules('en-US'); [0, 1, 2, 1.5].map(n => p.select(n)).join()", "other,one,other,other"),
            ("var o = new Intl.PluralRules('en-US', { type: 'ordinal' }); [1, 2, 3, 4, 11, 12, 13, 21, 22, 23, 101, 111].map(n => o.select(n)).join()", "one,two,few,other,other,other,other,one,two,few,one,other"),
            ("new Intl.PluralRules('en', { minimumFractionDigits: 1 }).select(1)", "other"),
            ("var r = new Intl.RelativeTimeFormat('en'); [r.format(-1, 'day'), r.format(2, 'days'), r.format(-3, 'month')].join('|')", "1 day ago|in 2 days|3 months ago"),
            ("var a = new Intl.RelativeTimeFormat('en', { numeric: 'auto' }); [a.format(-1, 'day'), a.format(0, 'day'), a.format(1, 'day'), a.format(-1, 'year'), a.format(0, 'second'), a.format(-2, 'day')].join('|')", "yesterday|today|tomorrow|last year|now|2 days ago"),
            ("var s = new Intl.RelativeTimeFormat('en', { style: 'short' }); [s.format(-3, 'hour'), s.format(5, 'minute'), s.format(-1, 'day')].join('|')", "3 hr. ago|in 5 min.|1 day ago"),
            ("new Intl.RelativeTimeFormat('en').formatToParts(10, 'seconds').map(p => p.type + (p.unit ? '/' + p.unit : '') + ':' + p.value).join('|')", "literal:in |integer/second:10|literal: seconds"),
            ("new Intl.ListFormat('en').format(['A', 'B', 'C'])", "A, B, and C"),
            ("new Intl.ListFormat('en').format(['A', 'B'])", "A and B"),
            ("new Intl.ListFormat('en', { type: 'disjunction' }).format(['A', 'B', 'C'])", "A, B, or C"),
            ("new Intl.ListFormat('en', { style: 'short' }).format(['A', 'B', 'C'])", "A, B, & C"),
            ("new Intl.ListFormat('en', { style: 'narrow', type: 'unit' }).format(['5 lb', '12 oz'])", "5 lb 12 oz"),
            ("try { Intl.PluralRules() } catch (e) { e.name }", "TypeError"),
        ]);
    }

    #[test]
    fn strings_are_compared_and_segmented() {
        check(&[
            ("['b', 'a', 'C', 'B', 'c', 'A'].sort((x, y) => x.localeCompare(y)).join('')", "aAbBcC"),
            ("['résumé', 'resume', 'Resume', 'resumes'].sort(new Intl.Collator('en').compare).join()", "resume,Resume,résumé,resumes"),
            ("['item10', 'item2', 'item1'].sort(new Intl.Collator(undefined, { numeric: true }).compare).join()", "item1,item2,item10"),
            ("'a'.localeCompare('A', undefined, { sensitivity: 'base' }) + ' ' + 'a'.localeCompare('á', undefined, { sensitivity: 'base' }) + ' ' + 'a'.localeCompare('á', undefined, { sensitivity: 'accent' })", "0 0 -1"),
            ("['a-b', 'ab', 'a b'].sort(new Intl.Collator().compare).join('|')", "a b|a-b|ab"),
            ("'ä'.localeCompare('z')", "-1"),
            ("'o\\u0308'.localeCompare('ö') + ' ' + ['ë', 'è', 'e', 'é', 'ê'].sort(Intl.Collator().compare).join('')", "0 eéèêë"),
            ("['Straße', 'Strasse', 'Strase'].sort(new Intl.Collator('en').compare).join()", "Strase,Strasse,Straße"),
            ("new Intl.Collator('en-u-kn').resolvedOptions().numeric + ' ' + new Intl.Collator('en-u-kn').resolvedOptions().locale", "true en-u-kn"),
            ("'Crème Brûlée'.normalize('NFD').replace(/[\\u0300-\\u036f]/g, '')", "Creme Brulee"),
            ("'e\\u0301'.normalize() === 'é' && 'é'.normalize('NFD').length", "2"),
            ("'istanbul'.toLocaleUpperCase('tr') + ' ' + 'I'.toLocaleLowerCase('tr-TR') + ' ' + 'i'.toLocaleUpperCase('en')", "İSTANBUL ı I"),
            ("[...new Intl.Segmenter('en', { granularity: 'grapheme' }).segment('e\\u0301👍🏽🇸🇬')].map(s => s.segment.length).join()", "2,4,4"),
            ("[...new Intl.Segmenter('en', { granularity: 'word' }).segment(\"Don't stop, 3.14 now!\")].filter(s => s.isWordLike).map(s => s.segment).join('|')", "Don't|stop|3.14|now"),
            ("[...new Intl.Segmenter('en', { granularity: 'sentence' }).segment('Hi there. It costs 3.50 now! Is it e.g. cheap? Yes.')].map(s => s.segment).join('|')", "Hi there. |It costs 3.50 now! |Is it e.g. cheap? |Yes."),
            ("new Intl.Segmenter('en', { granularity: 'word' }).segment('hello world').containing(7).segment", "world"),
        ]);
    }

    #[test]
    fn locales_and_names() {
        check(&[
            ("Intl.getCanonicalLocales(['EN-us', 'zh-hant-tw']).join()", "en-US,zh-Hant-TW"),
            ("var l = new Intl.Locale('en-Latn-US-u-ca-gregory'); [l.language, l.script, l.region, l.calendar, l.baseName].join()", "en,Latn,US,gregory,en-Latn-US"),
            ("new Intl.Locale('zh').maximize().toString()", "zh-Hans-CN"),
            ("new Intl.Locale('en', { calendar: 'gregory', hourCycle: 'h23', numeric: true }).toString()", "en-u-ca-gregory-hc-h23-kn"),
            ("Intl.getCanonicalLocales('EN-u-kn-true-ca-gregory').join()", "en-u-ca-gregory-kn"),
            ("try { Intl.getCanonicalLocales('de-gregory-gregory') } catch (e) { e.name }", "RangeError"),
            ("new Intl.NumberFormat('en').resolvedOptions().locale + ' ' + new Intl.NumberFormat('en-GB').resolvedOptions().locale + ' ' + new Intl.NumberFormat('fr').resolvedOptions().locale", "en en en-US"),
            ("new Intl.DisplayNames(['en'], { type: 'region' }).of('SG')", "Singapore"),
            ("new Intl.DisplayNames(['en'], { type: 'language' }).of('en-GB')", "British English"),
            ("new Intl.DisplayNames(['en'], { type: 'currency' }).of('USD')", "US Dollar"),
            ("Intl.supportedValuesOf('timeZone').includes('Asia/Singapore')", "true"),
            ("typeof Intl.DateTimeFormat.prototype.formatToParts", "function"),
        ]);
    }
}
