"""Writes src/unicode.rs: which characters are letters (L), marks (M), numbers (N) and white space (as the tokenizer's
splitting rule's \\s: U+0009-000D, U+0085 and the line, paragraph and space separators), as ranges, from Python's
own Unicode database."""
import sys
import unicodedata

def kind(cp):
    if 0x09 <= cp <= 0x0D or cp == 0x85:
        return "S"
    cat = unicodedata.category(chr(cp))
    if cat in ("Zs", "Zl", "Zp"):
        return "S"
    if cat[0] == "L":
        return "L"
    if cat[0] == "M":
        return "M"
    if cat[0] == "N":
        return "N"
    return None

ranges = []
start, cur = None, None
for cp in range(0x110000):
    if 0xD800 <= cp <= 0xDFFF:
        k = None
    else:
        k = kind(cp)
    if k != cur:
        if cur is not None:
            ranges.append((start, cp - 1, cur))
        start, cur = cp, k
if cur is not None:
    ranges.append((start, 0x10FFFF, cur))
out = [
    "//! Which characters are letters, marks, numbers and white space, as the tokenizer's splitting rule sees them",
    f"//! (Unicode {unicodedata.unidata_version}, written by tools/unicode_table.py from Python's own database).",
    "",
    "#[derive(Clone, Copy, PartialEq, Eq, Debug)]",
    "pub enum Class {",
    "    Letter,",
    "    Mark,",
    "    Number,",
    "    Space,",
    "    Other,",
    "}",
    "",
    "/// The ranges that aren't Other: first, last, class (0 letter, 1 mark, 2 number, 3 space), in order.",
    f"static RANGES: [(u32, u32, u8); {len(ranges)}] = [",
]
code = {"L": 0, "M": 1, "N": 2, "S": 3}
line = "   "
for a, b, k in ranges:
    item = f" (0x{a:X}, 0x{b:X}, {code[k]}),"
    if len(line) + len(item) > 118:
        out.append(line)
        line = "   "
    line += item
out.append(line)
out += [
    "];",
    "",
    "pub fn class(c: char) -> Class {",
    "    let cp = c as u32;",
    "    if cp < 0x80 {",
    "        return match c {",
    "            'a'..='z' | 'A'..='Z' => Class::Letter,",
    "            '0'..='9' => Class::Number,",
    "            '\\t' | '\\n' | '\\x0B' | '\\x0C' | '\\r' | ' ' => Class::Space,",
    "            _ => Class::Other,",
    "        };",
    "    }",
    "    match RANGES.binary_search_by(|&(a, b, _)| if b < cp { core::cmp::Ordering::Less } else if a > cp { core::cmp::Ordering::Greater } else { core::cmp::Ordering::Equal }) {",
    "        Ok(i) => match RANGES[i].2 {",
    "            0 => Class::Letter,",
    "            1 => Class::Mark,",
    "            2 => Class::Number,",
    "            _ => Class::Space,",
    "        },",
    "        Err(_) => Class::Other,",
    "    }",
    "}",
    "",
]
open(sys.argv[1], "w").write("\n".join(out))
print(len(ranges), "ranges, Unicode", unicodedata.unidata_version)
