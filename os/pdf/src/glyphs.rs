//! Glyph names as text (Adobe's glyph list's names, as fonts' /Differences give them): letters and digits, Latin-1's
//! and Latin Extended's letters, punctuation, ligatures, Greek, common symbols, "uniXXXX" and "uXXXX[XX]" names,
//! ligatures written "f_i", and names with a suffix (".sc", ".alt", ".001") as the name before it.

use alloc::string::String;

pub fn glyph(name: &[u8]) -> Option<String> {
    let name = core::str::from_utf8(name).ok()?;
    if let Some(dot) = name.find('.')
        && dot > 0
    {
        return glyph(name[..dot].as_bytes());
    }
    if name.contains('_') {
        let mut out = String::new();
        for part in name.split('_') {
            out.push_str(&glyph(part.as_bytes())?);
        }
        return Some(out);
    }
    if name.len() == 1 && name.as_bytes()[0].is_ascii_alphabetic() {
        return Some(String::from(name));
    }
    if let Some(hex) = name.strip_prefix("uni")
        && hex.len() >= 4
        && hex.len() % 4 == 0
        && hex.bytes().all(|b| b.is_ascii_hexdigit())
    {
        let units: alloc::vec::Vec<u16> = (0..hex.len() / 4).filter_map(|i| u16::from_str_radix(&hex[i * 4..i * 4 + 4], 16).ok()).collect();
        return Some(String::from_utf16_lossy(&units));
    }
    if let Some(hex) = name.strip_prefix('u')
        && (4..=6).contains(&hex.len())
        && hex.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return char::from_u32(u32::from_str_radix(hex, 16).ok()?).map(String::from);
    }
    named(name).map(String::from)
}

fn named(name: &str) -> Option<&'static str> {
    Some(match name {
        "space" | "nbspace" | "nonbreakingspace" => " ",
        "exclam" => "!", "quotedbl" => "\"", "numbersign" => "#", "dollar" => "$", "percent" => "%",
        "ampersand" => "&", "quotesingle" => "'", "parenleft" => "(", "parenright" => ")", "asterisk" => "*",
        "plus" => "+", "comma" => ",", "hyphen" | "sfthyphen" | "softhyphen" => "-", "period" => ".", "slash" => "/",
        "zero" => "0", "one" => "1", "two" => "2", "three" => "3", "four" => "4", "five" => "5", "six" => "6",
        "seven" => "7", "eight" => "8", "nine" => "9", "colon" => ":", "semicolon" => ";", "less" => "<",
        "equal" => "=", "greater" => ">", "question" => "?", "at" => "@", "bracketleft" => "[", "backslash" => "\\",
        "bracketright" => "]", "asciicircum" => "^", "underscore" => "_", "grave" => "`", "braceleft" => "{",
        "bar" => "|", "braceright" => "}", "asciitilde" => "~",
        "exclamdown" => "¡", "cent" => "¢", "sterling" => "£", "currency" => "¤", "yen" => "¥", "brokenbar" => "¦",
        "section" => "§", "dieresis" => "¨", "copyright" => "©", "ordfeminine" => "ª", "guillemotleft" => "«",
        "logicalnot" => "¬", "registered" => "®", "macron" => "¯", "degree" => "°", "plusminus" => "±",
        "twosuperior" => "²", "threesuperior" => "³", "acute" => "´", "mu" | "mu1" => "µ", "paragraph" => "¶",
        "periodcentered" | "middot" => "·", "cedilla" => "¸", "onesuperior" => "¹", "ordmasculine" => "º",
        "guillemotright" => "»", "onequarter" => "¼", "onehalf" => "½", "threequarters" => "¾",
        "questiondown" => "¿",
        "Agrave" => "À", "Aacute" => "Á", "Acircumflex" => "Â", "Atilde" => "Ã", "Adieresis" => "Ä", "Aring" => "Å",
        "AE" => "Æ", "Ccedilla" => "Ç", "Egrave" => "È", "Eacute" => "É", "Ecircumflex" => "Ê", "Edieresis" => "Ë",
        "Igrave" => "Ì", "Iacute" => "Í", "Icircumflex" => "Î", "Idieresis" => "Ï", "Eth" => "Ð", "Ntilde" => "Ñ",
        "Ograve" => "Ò", "Oacute" => "Ó", "Ocircumflex" => "Ô", "Otilde" => "Õ", "Odieresis" => "Ö",
        "multiply" => "×", "Oslash" => "Ø", "Ugrave" => "Ù", "Uacute" => "Ú", "Ucircumflex" => "Û",
        "Udieresis" => "Ü", "Yacute" => "Ý", "Thorn" => "Þ", "germandbls" => "ß",
        "agrave" => "à", "aacute" => "á", "acircumflex" => "â", "atilde" => "ã", "adieresis" => "ä", "aring" => "å",
        "ae" => "æ", "ccedilla" => "ç", "egrave" => "è", "eacute" => "é", "ecircumflex" => "ê", "edieresis" => "ë",
        "igrave" => "ì", "iacute" => "í", "icircumflex" => "î", "idieresis" => "ï", "eth" => "ð", "ntilde" => "ñ",
        "ograve" => "ò", "oacute" => "ó", "ocircumflex" => "ô", "otilde" => "õ", "odieresis" => "ö", "divide" => "÷",
        "oslash" => "ø", "ugrave" => "ù", "uacute" => "ú", "ucircumflex" => "û", "udieresis" => "ü", "yacute" => "ý",
        "thorn" => "þ", "ydieresis" => "ÿ", "Ydieresis" => "Ÿ",
        "Amacron" => "Ā", "amacron" => "ā", "Abreve" => "Ă", "abreve" => "ă", "Aogonek" => "Ą", "aogonek" => "ą",
        "Cacute" => "Ć", "cacute" => "ć", "Ccaron" => "Č", "ccaron" => "č", "Dcaron" => "Ď", "dcaron" => "ď",
        "Dcroat" => "Đ", "dcroat" => "đ", "Emacron" => "Ē", "emacron" => "ē", "Edotaccent" => "Ė",
        "edotaccent" => "ė", "Eogonek" => "Ę", "eogonek" => "ę", "Ecaron" => "Ě", "ecaron" => "ě", "Gbreve" => "Ğ",
        "gbreve" => "ğ", "Idotaccent" => "İ", "dotlessi" => "ı", "dotlessj" => "ȷ", "IJ" => "Ĳ", "ij" => "ĳ",
        "Lacute" => "Ĺ", "lacute" => "ĺ", "Lcaron" => "Ľ", "lcaron" => "ľ", "Lslash" => "Ł", "lslash" => "ł",
        "Nacute" => "Ń", "nacute" => "ń", "Ncaron" => "Ň", "ncaron" => "ň", "Eng" => "Ŋ", "eng" => "ŋ",
        "Omacron" => "Ō", "omacron" => "ō", "Ohungarumlaut" => "Ő", "ohungarumlaut" => "ő", "OE" => "Œ",
        "oe" => "œ", "Racute" => "Ŕ", "racute" => "ŕ", "Rcaron" => "Ř", "rcaron" => "ř", "Sacute" => "Ś",
        "sacute" => "ś", "Scedilla" => "Ş", "scedilla" => "ş", "Scaron" => "Š", "scaron" => "š", "Tcaron" => "Ť",
        "tcaron" => "ť", "Umacron" => "Ū", "umacron" => "ū", "Uring" => "Ů", "uring" => "ů",
        "Uhungarumlaut" => "Ű", "uhungarumlaut" => "ű", "Uogonek" => "Ų", "uogonek" => "ų", "Zacute" => "Ź",
        "zacute" => "ź", "Zdotaccent" => "Ż", "zdotaccent" => "ż", "Zcaron" => "Ž", "zcaron" => "ž",
        "florin" => "ƒ", "circumflex" => "ˆ", "caron" => "ˇ", "breve" => "˘", "dotaccent" => "˙", "ring" => "˚",
        "ogonek" => "˛", "tilde" => "˜", "hungarumlaut" => "˝",
        "endash" => "–", "emdash" => "—", "quoteleft" => "‘", "quoteright" => "’", "quotesinglbase" => "‚",
        "quotedblleft" => "“", "quotedblright" => "”", "quotedblbase" => "„", "dagger" => "†", "daggerdbl" => "‡",
        "bullet" => "•", "ellipsis" => "…", "perthousand" => "‰", "guilsinglleft" => "‹", "guilsinglright" => "›",
        "fraction" => "⁄", "Euro" | "euro" => "€", "trademark" => "™", "minus" => "−", "prime" => "′",
        "minute" => "′", "second" => "″",
        "ff" => "ff", "fi" => "fi", "fl" => "fl", "ffi" => "ffi", "ffl" => "ffl", "st" => "st",
        "Alpha" => "Α", "Beta" => "Β", "Gamma" => "Γ", "Delta" => "Δ", "Epsilon" => "Ε", "Zeta" => "Ζ", "Eta" => "Η",
        "Theta" => "Θ", "Iota" => "Ι", "Kappa" => "Κ", "Lambda" => "Λ", "Mu" => "Μ", "Nu" => "Ν", "Xi" => "Ξ",
        "Omicron" => "Ο", "Pi" => "Π", "Rho" => "Ρ", "Sigma" => "Σ", "Tau" => "Τ", "Upsilon" => "Υ", "Phi" => "Φ",
        "Chi" => "Χ", "Psi" => "Ψ", "Omega" => "Ω",
        "alpha" => "α", "beta" => "β", "gamma" => "γ", "delta" => "δ", "epsilon" => "ε", "zeta" => "ζ", "eta" => "η",
        "theta" => "θ", "iota" => "ι", "kappa" => "κ", "lambda" => "λ", "nu" => "ν", "xi" => "ξ",
        "omicron" => "ο", "pi" => "π", "rho" => "ρ", "sigma" => "σ", "sigma1" => "ς", "tau" => "τ",
        "upsilon" => "υ", "phi" => "φ", "phi1" => "ϕ", "chi" => "χ", "psi" => "ψ", "omega" => "ω",
        "infinity" => "∞", "partialdiff" => "∂", "summation" => "∑", "product" => "∏", "integral" => "∫",
        "radical" => "√", "approxequal" => "≈", "notequal" => "≠", "lessequal" => "≤", "greaterequal" => "≥",
        "element" => "∈", "notelement" => "∉", "arrowleft" => "←", "arrowright" => "→", "arrowup" => "↑",
        "arrowdown" => "↓", "arrowboth" => "↔", "lozenge" => "◊", "checkmark" => "✓", "similar" => "∼",
        "proportional" => "∝", "equivalence" => "≡", "intersection" => "∩", "union" => "∪", "nabla" => "∇",
        "visiblespace" => "␣",
        _ => return None,
    })
}
