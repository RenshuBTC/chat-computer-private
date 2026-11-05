//! Date: time values (milliseconds since 1970, UTC), the calendar arithmetic the language specifies, local time by
//! the embedder's time zone offset, parsing (ISO, the formats Date writes, and the loose forms pages use), and the
//! string forms.

use super::{arg, constructor, js_str, sym_method};
use crate::math::floor;
use crate::object::*;
use crate::value::{Gc, Value};
use crate::vm::{I, Sym, Vm};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

type R = Result<Value, Value>;

const MS_PER_DAY: f64 = 86_400_000.0;
const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

pub fn day(t: f64) -> f64 {
    floor(t / MS_PER_DAY)
}

fn time_within_day(t: f64) -> f64 {
    let r = t % MS_PER_DAY;
    if r < 0.0 { r + MS_PER_DAY } else { r }
}

fn days_in_year(y: f64) -> f64 {
    if y % 4.0 != 0.0 {
        365.0
    } else if y % 100.0 != 0.0 {
        366.0
    } else if y % 400.0 != 0.0 {
        365.0
    } else {
        366.0
    }
}

fn day_from_year(y: f64) -> f64 {
    365.0 * (y - 1970.0) + floor((y - 1969.0) / 4.0) - floor((y - 1901.0) / 100.0) + floor((y - 1601.0) / 400.0)
}

fn time_from_year(y: f64) -> f64 {
    MS_PER_DAY * day_from_year(y)
}

pub fn year_from_time(t: f64) -> f64 {
    let mut y = floor(t / (MS_PER_DAY * 365.2425)) + 1970.0;
    while time_from_year(y) > t {
        y -= 1.0;
    }
    while time_from_year(y + 1.0) <= t {
        y += 1.0;
    }
    y
}

fn in_leap_year(t: f64) -> bool {
    days_in_year(year_from_time(t)) == 366.0
}

fn day_within_year(t: f64) -> f64 {
    day(t) - day_from_year(year_from_time(t))
}

const CUMULATIVE: [f64; 13] = [0.0, 31.0, 59.0, 90.0, 120.0, 151.0, 181.0, 212.0, 243.0, 273.0, 304.0, 334.0, 365.0];

fn month_start(m: usize, leap: bool) -> f64 {
    CUMULATIVE[m] + if leap && m >= 2 { 1.0 } else { 0.0 }
}

pub fn month_from_time(t: f64) -> f64 {
    let d = day_within_year(t);
    let leap = in_leap_year(t);
    for m in 0..12 {
        if d < month_start(m + 1, leap) {
            return m as f64;
        }
    }
    11.0
}

pub fn date_from_time(t: f64) -> f64 {
    let d = day_within_year(t);
    let m = month_from_time(t) as usize;
    d - month_start(m, in_leap_year(t)) + 1.0
}

pub fn week_day(t: f64) -> f64 {
    let r = (day(t) + 4.0) % 7.0;
    if r < 0.0 { r + 7.0 } else { r }
}

fn hour_from_time(t: f64) -> f64 {
    floor(time_within_day(t) / 3_600_000.0)
}

fn min_from_time(t: f64) -> f64 {
    floor(time_within_day(t) / 60_000.0) % 60.0
}

fn sec_from_time(t: f64) -> f64 {
    floor(time_within_day(t) / 1000.0) % 60.0
}

fn ms_from_time(t: f64) -> f64 {
    time_within_day(t) % 1000.0
}

fn to_int(x: f64) -> f64 {
    crate::value::to_integer(x)
}

/// MakeTime.
pub fn make_time(h: f64, m: f64, s: f64, ms: f64) -> f64 {
    if !h.is_finite() || !m.is_finite() || !s.is_finite() || !ms.is_finite() {
        return f64::NAN;
    }
    to_int(h) * 3_600_000.0 + to_int(m) * 60_000.0 + to_int(s) * 1000.0 + to_int(ms)
}

/// MakeDay.
pub fn make_day(y: f64, m: f64, d: f64) -> f64 {
    if !y.is_finite() || !m.is_finite() || !d.is_finite() {
        return f64::NAN;
    }
    let (y, m, d) = (to_int(y), to_int(m), to_int(d));
    let ym = y + floor(m / 12.0);
    if ym.abs() > 400_000.0 {
        return f64::NAN;
    }
    let mn = ((m % 12.0) + 12.0) % 12.0;
    let leap = days_in_year(ym) == 366.0;
    let t = day_from_year(ym) + month_start(mn as usize, leap);
    t + d - 1.0
}

pub fn make_date(day: f64, time: f64) -> f64 {
    if !day.is_finite() || !time.is_finite() {
        return f64::NAN;
    }
    let tv = day * MS_PER_DAY + time;
    if !tv.is_finite() { f64::NAN } else { tv }
}

pub fn time_clip(t: f64) -> f64 {
    if !t.is_finite() || t.abs() > 8.64e15 {
        return f64::NAN;
    }
    to_int(t) + 0.0
}

/// The local time's offset from UTC at time t (milliseconds since 1970), in milliseconds: the time zone's (with its
/// summer time, if the embedder named a zone that has one).
fn offset_ms(vm: &Vm, t: f64) -> f64 {
    super::intl::local_offset_at(vm, t) as f64 * 60_000.0
}

fn local(vm: &Vm, t: f64) -> f64 {
    t + offset_ms(vm, t)
}

fn utc(vm: &Vm, t: f64) -> f64 {
    t - offset_ms(vm, t - vm.tz_offset as f64 * 60_000.0)
}

pub fn init(vm: &mut Vm) {
    let dp = vm.proto(I::DateProto);
    let c = constructor(vm, "Date", 7, date_call, Some(date_construct), Some(dp));
    vm.method(c, "now", 0, date_now);
    vm.method(c, "parse", 1, date_parse);
    vm.method(c, "UTC", 7, date_utc);
    let getters: [(&str, NativeFn); 19] = [
        ("getDate", get_date),
        ("getDay", get_day),
        ("getFullYear", get_full_year),
        ("getHours", get_hours),
        ("getMilliseconds", get_ms),
        ("getMinutes", get_minutes),
        ("getMonth", get_month),
        ("getSeconds", get_seconds),
        ("getTime", get_time),
        ("getTimezoneOffset", get_tz),
        ("getUTCDate", get_utc_date),
        ("getUTCDay", get_utc_day),
        ("getUTCFullYear", get_utc_full_year),
        ("getUTCHours", get_utc_hours),
        ("getUTCMilliseconds", get_utc_ms),
        ("getUTCMinutes", get_utc_minutes),
        ("getUTCMonth", get_utc_month),
        ("getUTCSeconds", get_utc_seconds),
        ("getYear", get_year),
    ];
    for (name, f) in getters {
        vm.method(dp, name, 0, f);
    }
    let setters: [(&str, u32, NativeFn); 16] = [
        ("setDate", 1, set_date),
        ("setFullYear", 3, set_full_year),
        ("setHours", 4, set_hours),
        ("setMilliseconds", 1, set_ms),
        ("setMinutes", 3, set_minutes),
        ("setMonth", 2, set_month),
        ("setSeconds", 2, set_seconds),
        ("setTime", 1, set_time),
        ("setUTCDate", 1, set_utc_date),
        ("setUTCFullYear", 3, set_utc_full_year),
        ("setUTCHours", 4, set_utc_hours),
        ("setUTCMilliseconds", 1, set_utc_ms),
        ("setUTCMinutes", 3, set_utc_minutes),
        ("setUTCMonth", 2, set_utc_month),
        ("setUTCSeconds", 2, set_utc_seconds),
        ("setYear", 1, set_year),
    ];
    for (name, len, f) in setters {
        vm.method(dp, name, len, f);
    }
    let strings: [(&str, u32, NativeFn); 10] = [
        ("toDateString", 0, to_date_string),
        ("toISOString", 0, to_iso_string),
        ("toJSON", 1, to_json),
        ("toLocaleDateString", 0, to_locale_date_string),
        ("toLocaleString", 0, to_locale_string),
        ("toLocaleTimeString", 0, to_locale_time_string),
        ("toString", 0, to_string),
        ("toTimeString", 0, to_time_string),
        ("toUTCString", 0, to_utc_string),
        ("valueOf", 0, get_time),
    ];
    for (name, len, f) in strings {
        let g = vm.method(dp, name, len, f);
        if name == "toUTCString" {
            vm.put(dp, "toGMTString", Value::Obj(g), HIDDEN);
        }
    }
    let tp = sym_method(vm, dp, Sym::ToPrimitive, "[Symbol.toPrimitive]", 1, to_primitive);
    let k = Key::Sym(vm.sym(Sym::ToPrimitive));
    vm.put_key(dp, k, Value::Obj(tp), CONFIGURABLE);
}

fn now(vm: &Vm) -> f64 {
    time_clip(floor((vm.now)()))
}

fn date_call(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let t = now(vm);
    Ok(js_str(&full_string(vm, t)))
}

fn date_construct(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let tv = match args.len() {
        0 => now(vm),
        1 => {
            let v = &args[0];
            if let Value::Obj(g) = v
                && let Class::Date(t) = vm.heap.get(*g).class
            {
                t
            } else {
                let p = vm.to_primitive(v, crate::convert::Hint::Default)?;
                match &p {
                    Value::Str(s) => parse(vm, &s.to_rust()),
                    _ => time_clip(vm.to_number(&p)?),
                }
            }
        }
        _ => {
            let t = from_parts(vm, args)?;
            time_clip(utc(vm, t))
        }
    };
    let p = vm.proto_from_constructor(nt, I::DateProto)?;
    Ok(Value::Obj(vm.alloc(Some(p), Class::Date(tv))))
}

/// The time of (year, month, date, hours, minutes, seconds, ms) arguments (two-digit years are 19xx).
fn from_parts(vm: &mut Vm, args: &[Value]) -> Result<f64, Value> {
    let mut v = [f64::NAN, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0];
    for (i, a) in args.iter().take(7).enumerate() {
        v[i] = vm.to_number(a)?;
    }
    let mut y = v[0];
    if y.is_finite() {
        let yi = to_int(y);
        if (0.0..=99.0).contains(&yi) {
            y = 1900.0 + yi;
        }
    }
    Ok(make_date(make_day(y, v[1], v[2]), make_time(v[3], v[4], v[5], v[6])))
}

fn date_now(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Num(now(vm)))
}

fn date_parse(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let s = vm.to_string(&arg(args, 0))?.to_rust();
    Ok(Value::Num(parse(vm, &s)))
}

fn date_utc(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let t = from_parts(vm, args)?;
    Ok(Value::Num(time_clip(t)))
}

fn this_time(vm: &mut Vm, this: &Value) -> Result<f64, Value> {
    if let Value::Obj(g) = this
        && let Class::Date(t) = vm.heap.get(*g).class
    {
        return Ok(t);
    }
    Err(vm.type_error("this is not a Date object."))
}

fn set_this(vm: &mut Vm, this: &Value, t: f64) -> R {
    let t = time_clip(t);
    if let Value::Obj(g) = this
        && let Class::Date(x) = &mut vm.heap.get_mut(*g).class
    {
        *x = t;
    }
    Ok(Value::Num(t))
}

macro_rules! getter {
    ($name:ident, $local:expr, $f:expr) => {
        fn $name(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
            let t = this_time(vm, this)?;
            if t.is_nan() {
                return Ok(Value::Num(f64::NAN));
            }
            let t = if $local { local(vm, t) } else { t };
            Ok(Value::Num($f(t)))
        }
    };
}

getter!(get_date, true, date_from_time);
getter!(get_day, true, week_day);
getter!(get_full_year, true, year_from_time);
getter!(get_hours, true, hour_from_time);
getter!(get_ms, true, ms_from_time);
getter!(get_minutes, true, min_from_time);
getter!(get_month, true, month_from_time);
getter!(get_seconds, true, sec_from_time);
getter!(get_utc_date, false, date_from_time);
getter!(get_utc_day, false, week_day);
getter!(get_utc_full_year, false, year_from_time);
getter!(get_utc_hours, false, hour_from_time);
getter!(get_utc_ms, false, ms_from_time);
getter!(get_utc_minutes, false, min_from_time);
getter!(get_utc_month, false, month_from_time);
getter!(get_utc_seconds, false, sec_from_time);
getter!(get_year, true, |t| year_from_time(t) - 1900.0);

fn get_time(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Num(this_time(vm, this)?))
}

fn get_tz(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let t = this_time(vm, this)?;
    if t.is_nan() {
        return Ok(Value::Num(f64::NAN));
    }
    Ok(Value::Num(0.0 - super::intl::local_offset_at(vm, t) as f64))
}

/// The arguments as numbers (undefined for those not given).
fn nums(vm: &mut Vm, args: &[Value], n: usize) -> Result<Vec<Option<f64>>, Value> {
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        // (The first is always converted: missing, it's undefined.)
        out.push(match args.get(i) {
            Some(v) => Some(vm.to_number(v)?),
            None if i == 0 => Some(f64::NAN),
            None => None,
        });
    }
    Ok(out)
}

/// A setter of time fields (h, m, s, ms from index `first`) or date fields (y, m, d from `first`).
fn set_fields(vm: &mut Vm, this: &Value, args: &[Value], is_local: bool, date: bool, first: usize, count: usize) -> R {
    let t0 = this_time(vm, this)?;
    let a = nums(vm, args, count)?;
    let t = if t0.is_nan() {
        if date && first == 0 {
            0.0
        } else {
            return Ok(Value::Num(f64::NAN));
        }
    } else if is_local {
        local(vm, t0)
    } else {
        t0
    };
    let mut parts = if date {
        [year_from_time(t), month_from_time(t), date_from_time(t)]
            .into_iter()
            .chain([hour_from_time(t), min_from_time(t), sec_from_time(t), ms_from_time(t)])
            .collect::<Vec<f64>>()
    } else {
        Vec::from([year_from_time(t), month_from_time(t), date_from_time(t), hour_from_time(t), min_from_time(t), sec_from_time(t), ms_from_time(t)])
    };
    let base = if date { first } else { 3 + first };
    for (i, v) in a.iter().enumerate() {
        if let Some(x) = v {
            parts[base + i] = *x;
        }
    }
    let nt = make_date(make_day(parts[0], parts[1], parts[2]), make_time(parts[3], parts[4], parts[5], parts[6]));
    let nt = if is_local { utc(vm, nt) } else { nt };
    set_this(vm, this, nt)
}

fn set_date(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    set_fields(vm, this, args, true, true, 2, 1)
}
fn set_full_year(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    set_fields(vm, this, args, true, true, 0, 3.min(args.len().max(1)))
}
fn set_hours(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    set_fields(vm, this, args, true, false, 0, 4.min(args.len().max(1)))
}
fn set_ms(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    set_fields(vm, this, args, true, false, 3, 1)
}
fn set_minutes(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    set_fields(vm, this, args, true, false, 1, 3.min(args.len().max(1)))
}
fn set_month(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    set_fields(vm, this, args, true, true, 1, 2.min(args.len().max(1)))
}
fn set_seconds(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    set_fields(vm, this, args, true, false, 2, 2.min(args.len().max(1)))
}
fn set_utc_date(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    set_fields(vm, this, args, false, true, 2, 1)
}
fn set_utc_full_year(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    set_fields(vm, this, args, false, true, 0, 3.min(args.len().max(1)))
}
fn set_utc_hours(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    set_fields(vm, this, args, false, false, 0, 4.min(args.len().max(1)))
}
fn set_utc_ms(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    set_fields(vm, this, args, false, false, 3, 1)
}
fn set_utc_minutes(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    set_fields(vm, this, args, false, false, 1, 3.min(args.len().max(1)))
}
fn set_utc_month(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    set_fields(vm, this, args, false, true, 1, 2.min(args.len().max(1)))
}
fn set_utc_seconds(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    set_fields(vm, this, args, false, false, 2, 2.min(args.len().max(1)))
}

fn set_time(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    this_time(vm, this)?;
    let t = vm.to_number(&arg(args, 0))?;
    set_this(vm, this, t)
}

fn set_year(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let t0 = this_time(vm, this)?;
    let y = vm.to_number(&arg(args, 0))?;
    if y.is_nan() {
        return set_this(vm, this, f64::NAN);
    }
    let yi = to_int(y);
    let y = if (0.0..=99.0).contains(&yi) { 1900.0 + yi } else { y };
    let t = if t0.is_nan() { 0.0 } else { local(vm, t0) };
    let d = make_day(y, month_from_time(t), date_from_time(t));
    let nt = utc(vm, make_date(d, time_within_day(t)));
    set_this(vm, this, nt)
}

// ------------------------------------------------------------------------------------------------- the strings

fn year_str(y: f64) -> String {
    if y >= 0.0 { format!("{:04}", y as i64) } else { format!("-{:04}", -y as i64) }
}

fn date_part(t: f64) -> String {
    format!("{} {} {:02} {}", DAYS[week_day(t) as usize], MONTHS[month_from_time(t) as usize], date_from_time(t) as i64, year_str(year_from_time(t)))
}

/// A time's local hours, minutes and seconds, and its zone's offset (tv: milliseconds since 1970).
fn time_part(vm: &Vm, tv: f64) -> String {
    let off = super::intl::local_offset_at(vm, tv);
    let t = local(vm, tv);
    let sign = if off >= 0 { '+' } else { '-' };
    let a = off.unsigned_abs();
    format!("{:02}:{:02}:{:02} GMT{sign}{:02}{:02}", hour_from_time(t) as i64, min_from_time(t) as i64, sec_from_time(t) as i64, a / 60, a % 60)
}

/// Date.prototype.toString's form.
pub fn full_string(vm: &Vm, tv: f64) -> String {
    if tv.is_nan() {
        return String::from("Invalid Date");
    }
    format!("{} {}", date_part(local(vm, tv)), time_part(vm, tv))
}

fn to_string(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let t = this_time(vm, this)?;
    Ok(js_str(&full_string(vm, t)))
}

fn to_date_string(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let t = this_time(vm, this)?;
    if t.is_nan() {
        return Ok(js_str("Invalid Date"));
    }
    Ok(js_str(&date_part(local(vm, t))))
}

fn to_time_string(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let t = this_time(vm, this)?;
    if t.is_nan() {
        return Ok(js_str("Invalid Date"));
    }
    Ok(js_str(&time_part(vm, t)))
}

fn to_utc_string(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let t = this_time(vm, this)?;
    if t.is_nan() {
        return Ok(js_str("Invalid Date"));
    }
    Ok(js_str(&format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} GMT",
        DAYS[week_day(t) as usize],
        date_from_time(t) as i64,
        MONTHS[month_from_time(t) as usize],
        year_str(year_from_time(t)),
        hour_from_time(t) as i64,
        min_from_time(t) as i64,
        sec_from_time(t) as i64
    )))
}

pub fn iso_string(t: f64) -> String {
    let y = year_from_time(t);
    let ys = if (0.0..=9999.0).contains(&y) { format!("{:04}", y as i64) } else if y < 0.0 { format!("-{:06}", -y as i64) } else { format!("+{:06}", y as i64) };
    format!("{ys}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z", month_from_time(t) as i64 + 1, date_from_time(t) as i64, hour_from_time(t) as i64, min_from_time(t) as i64, sec_from_time(t) as i64, ms_from_time(t) as i64)
}

fn to_iso_string(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let t = this_time(vm, this)?;
    if t.is_nan() {
        return Err(vm.range_error("Invalid time value"));
    }
    Ok(js_str(&iso_string(t)))
}

fn to_json(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let o = vm.to_object(this)?;
    let tv = vm.to_primitive(&Value::Obj(o), crate::convert::Hint::Number)?;
    if let Value::Num(x) = tv
        && !x.is_finite()
    {
        return Ok(Value::Null);
    }
    let a = vm.atom("toISOString");
    let f = vm.get(o, Key::Str(a), &Value::Obj(o))?;
    vm.call(&f, Value::Obj(o), &[])
}

fn to_locale_date_string(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let t = this_time(vm, this)?;
    super::intl::date_to_locale(vm, t, &arg(args, 0), &arg(args, 1), 0, 0)
}

fn to_locale_time_string(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let t = this_time(vm, this)?;
    super::intl::date_to_locale(vm, t, &arg(args, 0), &arg(args, 1), 1, 1)
}

fn to_locale_string(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let t = this_time(vm, this)?;
    super::intl::date_to_locale(vm, t, &arg(args, 0), &arg(args, 1), 2, 2)
}

fn to_primitive(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    if !this.is_object() {
        return Err(vm.type_error("Date.prototype[Symbol.toPrimitive] called on non-object"));
    }
    let hint = match arg(args, 0) {
        Value::Str(s) if s.is("default") || s.is("string") => crate::convert::Hint::String,
        Value::Str(s) if s.is("number") => crate::convert::Hint::Number,
        _ => return Err(vm.type_error("Invalid hint")),
    };
    let Value::Obj(o) = this else { unreachable!() };
    let order = if hint == crate::convert::Hint::String { [crate::names::TO_STRING, crate::names::VALUE_OF] } else { [crate::names::VALUE_OF, crate::names::TO_STRING] };
    for name in order {
        let m = vm.get(*o, Key::Str(name), this)?;
        if vm.is_callable(&m) {
            let r = vm.call(&m, this.clone(), &[])?;
            if !r.is_object() {
                return Ok(r);
            }
        }
    }
    Err(vm.type_error("Cannot convert object to primitive value"))
}

// ------------------------------------------------------------------------------------------------------- parsing

/// Date.parse: the ISO format (UTC unless a date-time has no zone), else the loose forms.
pub fn parse(vm: &Vm, s: &str) -> f64 {
    let s = s.trim();
    if let Some(t) = parse_iso(vm, s) {
        return time_clip(t);
    }
    time_clip(parse_loose(vm, s).unwrap_or(f64::NAN))
}

fn digits(b: &[u8], i: &mut usize, n: usize) -> Option<f64> {
    if *i + n > b.len() || !b[*i..*i + n].iter().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let v = b[*i..*i + n].iter().fold(0.0, |a, c| a * 10.0 + (c - b'0') as f64);
    *i += n;
    Some(v)
}

fn parse_iso(vm: &Vm, s: &str) -> Option<f64> {
    let b = s.as_bytes();
    let mut i = 0;
    let year = match b.first()? {
        b'+' | b'-' => {
            let neg = b[0] == b'-';
            i = 1;
            let y = digits(b, &mut i, 6)?;
            if neg && y == 0.0 {
                return None;
            }
            if neg { -y } else { y }
        }
        _ => digits(b, &mut i, 4)?,
    };
    let (mut month, mut day_) = (1.0, 1.0);
    if i < b.len() && b[i] == b'-' {
        i += 1;
        month = digits(b, &mut i, 2)?;
        if i < b.len() && b[i] == b'-' {
            i += 1;
            day_ = digits(b, &mut i, 2)?;
        }
    }
    if !(1.0..=12.0).contains(&month) || !(1.0..=31.0).contains(&day_) {
        return None;
    }
    let (mut h, mut m, mut sec, mut ms) = (0.0, 0.0, 0.0, 0.0);
    let mut has_time = false;
    let mut offset: Option<f64> = None;
    if i < b.len() && (b[i] == b'T' || b[i] == b't') {
        has_time = true;
        i += 1;
        h = digits(b, &mut i, 2)?;
        if b.get(i) != Some(&b':') {
            return None;
        }
        i += 1;
        m = digits(b, &mut i, 2)?;
        if b.get(i) == Some(&b':') {
            i += 1;
            sec = digits(b, &mut i, 2)?;
            if b.get(i) == Some(&b'.') {
                i += 1;
                let start = i;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
                if i == start {
                    return None;
                }
                let frac = &s[start..i.min(start + 3)];
                ms = frac.parse::<f64>().ok()? * crate::math::pow(10.0, (3 - frac.len() as i32) as f64);
            }
        }
        if h > 24.0 || m > 59.0 || sec > 59.0 || (h == 24.0 && (m > 0.0 || sec > 0.0 || ms > 0.0)) {
            return None;
        }
        match b.get(i) {
            Some(b'Z') | Some(b'z') => {
                offset = Some(0.0);
                i += 1;
            }
            Some(b'+') | Some(b'-') => {
                let neg = b[i] == b'-';
                i += 1;
                let oh = digits(b, &mut i, 2)?;
                if b.get(i) != Some(&b':') {
                    return None;
                }
                i += 1;
                let om = digits(b, &mut i, 2)?;
                let o = (oh * 60.0 + om) * 60_000.0;
                offset = Some(if neg { -o } else { o });
            }
            _ => {}
        }
    }
    if i != b.len() {
        return None;
    }
    let dv = make_day(year, month - 1.0, day_);
    if date_from_time(make_date(dv, 0.0)) != day_ {
        return None;
    }
    let t = make_date(dv, make_time(h, m, sec, ms));
    Some(match offset {
        Some(o) => t - o,
        // A date-only form is UTC; a date-time form without a zone is local.
        None if has_time => utc(vm, t),
        None => t,
    })
}

/// The loose forms: "Tue Oct 20 2026 10:00:00 GMT+0000 (...)", "Tue, 20 Oct 2026 10:00:00 GMT", "Oct 20, 2026",
/// "10/20/2026 10:00", "2026/10/20", with an optional zone.
fn parse_loose(vm: &Vm, s: &str) -> Option<f64> {
    let s = match s.find('(') {
        Some(i) => &s[..i],
        None => s,
    };
    let mut nums: Vec<f64> = Vec::new();
    let mut month: Option<f64> = None;
    let mut time: Option<(f64, f64, f64)> = None;
    let mut offset: Option<f64> = None;
    let mut pm: Option<bool> = None;
    let tokens: Vec<&str> = s.split(|c: char| c.is_whitespace() || c == ',').filter(|t| !t.is_empty()).collect();
    for tok in tokens {
        let lower = tok.to_ascii_lowercase();
        if let Some(m) = MONTHS.iter().position(|m| lower.starts_with(&m.to_ascii_lowercase())) {
            month = Some(m as f64);
            continue;
        }
        if DAYS.iter().any(|d| lower.starts_with(&d.to_ascii_lowercase())) {
            continue;
        }
        if lower == "am" || lower == "pm" {
            pm = Some(lower == "pm");
            continue;
        }
        if lower.starts_with("gmt") || lower.starts_with("utc") || lower == "z" {
            let rest = &tok[3.min(tok.len())..];
            offset = Some(if rest.is_empty() { 0.0 } else { zone(rest)? });
            continue;
        }
        if (tok.starts_with('+') || tok.starts_with('-')) && tok.len() >= 3 && time.is_some() {
            offset = Some(zone(tok)?);
            continue;
        }
        if tok.contains(':') {
            let p: Vec<&str> = tok.split(':').collect();
            let h = p.first()?.parse::<f64>().ok()?;
            let m = p.get(1).map_or(Some(0.0), |x| x.parse::<f64>().ok())?;
            let sec = p.get(2).map_or(Some(0.0), |x| x.parse::<f64>().ok())?;
            time = Some((h, m, sec));
            continue;
        }
        if tok.contains('/') || (tok.contains('-') && !tok.starts_with('-')) {
            let p: Vec<f64> = tok.split(['/', '-']).map(|x| x.parse::<f64>().ok()).collect::<Option<Vec<f64>>>()?;
            if p.len() == 3 {
                if p[0] > 31.0 {
                    month = Some(p[1] - 1.0);
                    nums.extend([p[2], p[0]]);
                } else {
                    month = Some(p[0] - 1.0);
                    nums.extend([p[1], p[2]]);
                }
                continue;
            }
            return None;
        }
        nums.push(tok.parse::<f64>().ok()?);
    }
    let month = month?;
    let (d, y) = match nums.as_slice() {
        [a, b] if *a > 31.0 => (*b, *a),
        [a, b] => (*a, *b),
        [a] => (1.0, *a),
        _ => return None,
    };
    let y = if (0.0..50.0).contains(&y) {
        2000.0 + y
    } else if (50.0..100.0).contains(&y) {
        1900.0 + y
    } else {
        y
    };
    let (mut h, m, sec) = time.unwrap_or((0.0, 0.0, 0.0));
    if let Some(p) = pm {
        if p && h < 12.0 {
            h += 12.0;
        } else if !p && h == 12.0 {
            h = 0.0;
        }
    }
    let t = make_date(make_day(y, month, d), make_time(h, m, sec, 0.0));
    Some(match offset {
        Some(o) => t - o,
        None => utc(vm, t),
    })
}

/// "+0530", "+05:30", "-08": the offset in milliseconds.
fn zone(z: &str) -> Option<f64> {
    let neg = z.starts_with('-');
    let body: String = z.trim_start_matches(['+', '-']).chars().filter(|c| *c != ':').collect();
    let v: f64 = body.parse().ok()?;
    let (h, m) = if body.len() <= 2 { (v, 0.0) } else { (floor(v / 100.0), v % 100.0) };
    let o = (h * 60.0 + m) * 60_000.0;
    Some(if neg { -o } else { o })
}
