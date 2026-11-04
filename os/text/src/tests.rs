//! Lines set as HarfBuzz sets them (reference.py), kerning and all, and measured, wrapped and drawn the same way.

use super::*;

// Where HarfBuzz sets each glyph along the line (font units), at the regular weight or the bold, and the line's width.
const SET: [(&str, bool, &[i32], i32); 12] = [
    ("AVATAR", false, &[0, 587, 1196, 1798, 2414, 3040], 3668),
    ("AVATAR", true, &[0, 590, 1221, 1836, 2433, 3101], 3787),
    ("Today, Tokyo.", false, &[0, 591, 1176, 1774, 2280, 2753, 3006, 3269, 3860, 4445, 4982, 5500, 6085], 6331),
    ("Today, Tokyo.", true, &[0, 599, 1165, 1761, 2272, 2784, 3034, 3297, 3896, 4458, 5052, 5594, 6160], 6393),
    ("WAVY lines: P. F, y. r.", false, &[0, 951, 1538, 2250, 2884, 3147, 3419, 3700, 4307, 4849, 5312, 5562, 5825, 6347, 6593, 6856, 7331, 7584, 7847, 8338, 8584, 8847, 9238], 9484),
    ("WAVY lines: P. F, y. r.", true, &[0, 946, 1536, 2268, 2935, 3198, 3510, 3837, 4475, 5013, 5498, 5748, 6011, 6559, 6792, 7055, 7562, 7812, 8075, 8587, 8820, 9083, 9518], 9751),
    ("“Quoted” — LT AV To", false, &[0, 385, 1181, 1778, 2363, 2750, 3287, 3885, 4234, 4497, 5338, 5601, 6120, 6760, 7023, 7610, 8263, 8526, 9117], 9702),
    ("“Quoted” — LT AV To", true, &[0, 423, 1193, 1811, 2377, 2790, 3328, 3924, 4335, 4598, 5443, 5706, 6249, 6899, 7162, 7752, 8460, 8723, 9322], 9888),
    ("Hello, world", false, &[0, 758, 1300, 1572, 1844, 2429, 2682, 2945, 3734, 4319, 4748, 5020], 5618),
    ("Hello, world", true, &[0, 781, 1319, 1631, 1943, 2509, 2759, 3022, 3823, 4389, 4860, 5172], 5768),
    (" T A Y ", false, &[0, 263, 903, 1166, 1792, 2055, 2689], 2952),
    (" T A Y ", true, &[0, 263, 913, 1176, 1844, 2107, 2774], 3037),
];

// How HarfBuzz sets accents typed after their letters (reference.py): each glyph, where it is along the line, and how
// far up from the baseline.
const MARKS: [(&str, bool, &[(u16, i32, i32)]); 30] = [
    ("e\u{301}", false, &[(219, 0, 0)]),
    ("e\u{301}", true, &[(219, 0, 0)]),
    ("e\u{323}\u{302}", false, &[(224, 0, 0)]),
    ("e\u{323}\u{302}", true, &[(224, 0, 0)]),
    ("e\u{302}\u{323}", false, &[(224, 0, 0)]),
    ("e\u{302}\u{323}", true, &[(224, 0, 0)]),
    ("n\u{303}\u{323}", false, &[(283, 0, 0), (837, 182, 0)]),
    ("n\u{303}\u{323}", true, &[(283, 0, 0), (837, 240, 0)]),
    ("y\u{30a}", false, &[(357, 0, 0), (831, 128, 0)]),
    ("y\u{30a}", true, &[(357, 0, 0), (831, 116, 0)]),
    ("k\u{323}\u{307}", false, &[(268, 0, 0), (837, 147, 0), (824, 187, 255)]),
    ("k\u{323}\u{307}", true, &[(268, 0, 0), (837, 221, 0), (824, 217, 255)]),
    ("j\u{301}", false, &[(266, 0, 0), (826, 7, 0)]),
    ("j\u{301}", true, &[(266, 0, 0), (826, -93, 0)]),
    ("i\u{308}", false, &[(256, 0, 0)]),
    ("i\u{308}", true, &[(256, 0, 0)]),
    ("a\u{328}\u{301}", false, &[(203, 0, 0), (826, 125, 33)]),
    ("a\u{328}\u{301}", true, &[(203, 0, 0), (826, 0, 0)]),
    ("a\u{304}\u{301}", false, &[(202, 0, 0), (826, 125, 233)]),
    ("a\u{304}\u{301}", true, &[(202, 0, 0), (826, 0, 223)]),
    ("w\u{30a}", false, &[(351, 0, 0), (831, 241, 0)]),
    ("w\u{30a}", true, &[(351, 0, 0), (831, 239, 0)]),
    ("h\u{306}", false, &[(247, 0, 0), (830, 147, 203)]),
    ("h\u{306}", true, &[(247, 0, 0), (830, 148, 204)]),
    ("o\u{31b}\u{309}\u{323}", false, &[(300, 0, 0), (834, 58, 0)]),
    ("o\u{31b}\u{309}\u{323}", true, &[(300, 0, 0), (834, -9, 0)]),
    ("xe\u{301}x", false, &[(356, 0, 0), (219, 525, 0), (356, 1062, 0)]),
    ("xe\u{301}x", true, &[(356, 0, 0), (219, 534, 0), (356, 1072, 0)]),
    ("Te\u{301}", false, &[(139, 0, 0), (219, 591, 0)]),
    ("Te\u{301}", true, &[(139, 0, 0), (219, 599, 0)]),
];

/// A size at which a font unit is a 256th of a pixel: widths come out in font units.
const UNIT: i32 = 1000;

fn starts(ty: &Type, text: &str, face: Face) -> Vec<i32> {
    ty.line(text, face).iter().map(|&(x, _, _)| x).collect()
}

#[test]
fn sets_letters_where_harfbuzz_does() {
    let ty = Type::builtin();
    for (text, bold, want, width) in SET {
        let face = if bold { Face::Bold } else { Face::Regular };
        assert_eq!(starts(&ty, text, face), want, "{text:?}, bold {bold}");
        assert_eq!(ty.width_in(text, face, UNIT), width, "{text:?}, bold {bold}");
        // Slanted, the same glyphs in the same places.
        let slanted = if bold { Face::BoldItalic } else { Face::Italic };
        assert_eq!(starts(&ty, text, slanted), want, "{text:?} slanted");
    }
    // Kerned closer than the letters' own widths.
    let plain: i32 = "AVATAR".chars().map(|c| ty.set(c, Face::Regular).unwrap().advance).sum();
    assert!(ty.width("AVATAR", UNIT) < plain - 100);
}

#[test]
fn kerns_only_within_one_font_and_face() {
    let ty = Type::builtin();
    let (r, b) = (Face::Regular, Face::Bold);
    assert!(ty.kern_between(('A', r), ('V', r), UNIT) < 0 && ty.kern_between(('A', b), ('V', b), UNIT) < 0);
    // Not across a change of face, nor in code (its font doesn't kern), nor with a fallback font's character.
    assert_eq!(ty.kern_between(('A', r), ('V', b), UNIT), 0);
    assert_eq!(ty.kern_between(('A', Face::Mono), ('V', Face::Mono), UNIT), 0);
    assert_eq!(ty.kern_between(('T', r), ('→', r), UNIT), 0);
    // Something that shows nothing doesn't come between letters that kern; a line break does.
    assert_eq!(starts(&ty, "A\u{200B}V", r), starts(&ty, "AV", r));
    assert_eq!(ty.width("A\u{FE0F}V", UNIT), ty.width("AV", UNIT));
}

#[test]
fn wraps_kerned_text_as_wide_as_it_is_drawn() {
    let ty = Type::builtin();
    // (At this size widths are font units, exactly: no rounding between measuring and wrapping.)
    let size = UNIT;
    // Kerned, it fits in its own width (it wouldn't, unkerned); a unit less and it breaks inside the word.
    let word = "AVAVAVAV";
    let w = ty.width(word, size);
    assert_eq!(ty.breaks(word, size, w), vec![0..8]);
    assert_eq!(ty.breaks(word, size, w - 1).len(), 2);
    let styled = |t: &str| t.chars().map(|c| (c, Face::Regular)).collect::<Vec<_>>();
    assert_eq!(ty.breaks_styled(&styled(word), size, w), vec![0..8]);
    // Broken at a space, the next line starts with the word after it: "ToT" is a unit too wide for the last line.
    // (Lora kerns no letter with a space, so nothing of a space's carries over to the next line.)
    let text = "To To ToT";
    let max = ty.width("ToT", size) - 1;
    assert_eq!(ty.kern_between((' ', Face::Regular), ('T', Face::Regular), size), 0);
    assert_eq!(ty.breaks(text, size, max), vec![0..2, 3..5, 6..8, 8..9]);
    assert_eq!(ty.breaks_styled(&styled(text), size, max), vec![0..2, 3..5, 6..8, 8..9]);
    assert_eq!(ty.breaks(text, size, max + 1), vec![0..2, 3..5, 6..9]);
    // A line with a space in it is as wide as its letters and the space, and fits in that width.
    let spaced = ty.width("T A", size);
    assert_eq!(spaced, ty.width("T", size) + ty.width(" ", size) + ty.width("A", size));
    assert_eq!(ty.breaks("T A", size, spaced), vec![0..3]);
}

#[test]
fn the_caret_goes_where_the_letter_starts() {
    let ty = Type::builtin();
    let text = "AVATAR";
    let want = starts(&ty, text, Face::Regular);
    for (i, x) in want.iter().enumerate() {
        assert_eq!(ty.x_at(text, i, UNIT), *x, "before letter {i}");
    }
    assert_eq!(ty.x_at(text, text.len(), UNIT), ty.width(text, UNIT));
    assert_eq!(ty.x_at(text, 99, UNIT), ty.width(text, UNIT));
    assert_eq!(ty.x_at("", 0, UNIT), 0);
}

#[test]
fn draws_kerned_lines_as_wide_as_measured() {
    let ty = Type::builtin();
    let style = Style { size: 24 * ONE, line_height: 36 * ONE };
    for face in [Face::Regular, Face::Bold] {
        let ink = ty.ink_in("AVATAR", face, style, 0);
        assert_eq!(ink.width, ((ty.width_in("AVATAR", face, style.size) + ONE - 1) / ONE) as usize + 2 * PAD);
        // Ink in the last column a letter can reach, none past the pad.
        let column = |x: usize| (0..ink.height).map(|y| ink.coverage[y * ink.width + x] as u32).sum::<u32>();
        assert!(column(ink.width - PAD - 2) > 0 && column(ink.width - 1) == 0, "{face:?}");
    }
}

#[test]
fn chinese_japanese_and_korean_are_drawn_from_their_font() {
    let ty = Type::builtin();
    for c in ['中', '文', '說', '気', 'あ', 'カ', '한', '글', '。', '，', '「', 'Ａ', '\u{3000}'] {
        for face in [Face::Regular, Face::Bold, Face::Italic, Face::Mono] {
            let s = ty.set(c, face).unwrap();
            assert!(s.source == Source::Cjk && s.glyph != 0, "{c:?} in {face:?}");
            // Ideographs, kana and their punctuation an em wide; Hangul a little narrower, as Noto Sans KR has it.
            assert_eq!(s.advance, if ('가'..='힣').contains(&c) { 920 } else { 1000 }, "{c:?}");
        }
    }
    // Not kerned, nor with what's beside them: a line as wide as its characters.
    assert_eq!(ty.width("中文한", UNIT), 2920);
    assert_eq!(starts(&ty, "A中V", Face::Regular), vec![0, ty.set('A', Face::Regular).unwrap().advance, ty.set('A', Face::Regular).unwrap().advance + 1000]);
    // Drawn: ink where each is, none past the line.
    let style = Style { size: 24 * ONE, line_height: 36 * ONE };
    let ink = ty.ink("中文", style, 0);
    let column = |ink: &Ink, x: usize| (0..ink.height).map(|y| ink.coverage[y * ink.width + x] as u32).sum::<u32>();
    assert!(column(&ink, PAD + 12) > 0 && column(&ink, PAD + 36) > 0 && column(&ink, ink.width - 1) == 0);
    // Bold: drawn twice a little apart (the font has no bold), so more ink in the same width.
    let bold = ty.ink_in("中文", Face::Bold, style, 0);
    let total = |ink: &Ink| ink.coverage.iter().map(|&v| v as u32).sum::<u32>();
    assert_eq!(bold.width, ink.width);
    assert!(total(&bold) > total(&ink) * 11 / 10, "{} {}", total(&bold), total(&ink));
    assert_eq!(made_up_bold(9 * ONE), 9 * ONE / 24);
    assert_eq!(made_up_bold(36 * ONE), 36 * ONE / 32);
}

/// Lines of `text` (each a character an em wide, at a size where units are 1/256 pixel) at most `chars` wide.
fn lines_of(ty: &Type, text: &str, max: i32) -> Vec<String> {
    ty.breaks(text, UNIT, max).into_iter().map(|r| text[r].to_string()).collect::<Vec<_>>()
}

/// (Each of these breaks is where Chromium broke the same line, in the same fonts and widths, checked by hand; but
/// where Chromium lets a line too long to break run past its width, this breaks it, as it breaks a long word.)
#[test]
fn chinese_japanese_and_korean_break_between_characters() {
    let ty = Type::builtin();
    // Between any two, without spaces.
    assert_eq!(lines_of(&ty, "你好世界你好世界", 3000), ["你好世", "界你好", "世界"]);
    // Not before closing punctuation, nor after opening: the character before it goes with it.
    assert_eq!(lines_of(&ty, "你好，世界", 2000), ["你", "好，", "世界"]);
    assert_eq!(lines_of(&ty, "我说「你好」", 3000), ["我说", "「你", "好」"]);
    assert_eq!(lines_of(&ty, "東京へ行った。", 3000), ["東京へ", "行っ", "た。"]);
    // Small kana and the long vowel mark can start a line, as browsers break by default.
    assert_eq!(lines_of(&ty, "コーヒー", 3500), ["コーヒ", "ー"]);
    // Not before or after a quotation mark.
    assert_eq!(lines_of(&ty, "她说“好”了", 4000), ["她", "说“好”了"]);
    // Latin words in it aren't broken inside: before and after them it can break.
    let python = ty.width("Python", UNIT);
    assert_eq!(lines_of(&ty, "我用Python写代码", 1000 + python), ["我用", "Python写", "代码"]);
    // Nor at a straight quote, which might open or close.
    let quote = ty.width("\"", UNIT);
    assert_eq!(lines_of(&ty, "你说\"好\"", 2000 + 2 * quote), ["你", "说\"好\""]);
    // Korean breaks between syllables too, as browsers break it; at a space, the space goes.
    assert_eq!(lines_of(&ty, "안녕하세요 세계", 3 * 920), ["안녕하", "세요", "세계"]);
    // Right after a space, the break is the space's (the space goes with it); after the next character, a
    // character's.
    let hello = ty.width("Hello ", UNIT);
    assert_eq!(lines_of(&ty, "Hello 你好", hello + 500), ["Hello", "你好"]);
    assert_eq!(lines_of(&ty, "Hello 你好", hello + 1500), ["Hello 你", "好"]);
    // An ideographic space is a space: a line breaks after it, and it goes.
    assert_eq!(lines_of(&ty, "第三章\u{3000}ここから", 3500), ["第三章", "ここか", "ら"]);
    // Breaks by character count are the same for styled text.
    let styled: Vec<(char, Face)> = "你好，世界".chars().map(|c| (c, Face::Bold)).collect();
    assert_eq!(ty.breaks_styled(&styled, UNIT, 2000), vec![0..1, 1..3, 3..5]);
}

#[test]
fn sets_accents_typed_as_two_characters_as_harfbuzz_does() {
    let ty = Type::builtin();
    for (text, bold, want) in MARKS {
        let face = if bold { Face::Bold } else { Face::Regular };
        let got: Vec<(u16, i32, i32)> = ty.line(text, face).iter().map(|&(x, y, s)| (s.glyph, x, y)).collect();
        assert_eq!(got, want, "{text:?}, bold {bold}");
        assert!(ty.line(text, face).iter().all(|&(_, _, s)| s.source == Source::Text), "{text:?}");
    }
    // Composed, the same as the one character: as wide, drawn the same.
    let style = Style { size: 24 * ONE, line_height: 36 * ONE };
    for (two, one) in [("e\u{301}", "é"), ("Cafe\u{301} au lait", "Café au lait"), ("e\u{302}\u{323}", "ệ"), ("A\u{30a}", "Å")] {
        for face in [Face::Regular, Face::Bold, Face::Italic] {
            assert_eq!(ty.width_in(two, face, UNIT), ty.width_in(one, face, UNIT), "{two:?}");
            let (a, b) = (ty.ink_in(two, face, style, 0), ty.ink_in(one, face, style, 0));
            assert!(a.width == b.width && a.coverage == b.coverage, "{two:?} in {face:?}");
        }
    }
    // Marks take no room, and don't come between letters that kern.
    assert_eq!(ty.width("q\u{301}", UNIT), ty.width("q", UNIT));
    assert_eq!(starts(&ty, "AV", Face::Regular)[1], starts(&ty, "A\u{30a}V", Face::Regular)[1]);
}

#[test]
fn places_marks_without_anchors_over_and_under_their_letters() {
    let ty = Type::builtin();
    // Lora has no anchors on x: the dot goes under its ink, the circumflex over it, each centred on its width.
    let set = ty.line("x\u{323}\u{302}", Face::Regular);
    let [(_, _, x), (dx1, dy1, below), (dx2, dy2, above)] = set[..] else { panic!("{} glyphs", set.len()) };
    let bounds = |s: Set| ty.font_of(s.source).bounds(s.glyph).unwrap();
    let (x_ink, b, a) = (bounds(x), bounds(below), bounds(above));
    // A 16th of an em from the ink, as HarfBuzz does it: a mark below never moved up (Lora's dot is lower already),
    // one above moved down only half as far as that would take it (Lora's circumflex is higher).
    let gap = 1000 / 16;
    let down = x_ink.3 + gap - a.1;
    assert_eq!((dy1, dy2), ((x_ink.1 - gap - b.3).min(0), if down < 0 { down / 2 } else { down }));
    assert!(dy1 + b.3 < x_ink.1 && dy2 + a.1 > x_ink.3, "under and over the letter's ink");
    for (dx, m) in [(dx1, b), (dx2, a)] {
        let centre = dx + (m.0 + m.2) / 2;
        assert!((centre - x.advance / 2).abs() <= 1, "centred: {centre} of {}", x.advance);
    }
    // A mark from another font (Lora has no macron below; the code font's fallback has): under the letter's ink,
    // centred on it, taking no room.
    assert_eq!(ty.width("e\u{331}", UNIT), ty.width("e", UNIT));
    let set = ty.line("e\u{331}", Face::Regular);
    let [(_, _, e), (dx, dy, m)] = set[..] else { panic!() };
    let (e_ink, m_ink) = (bounds(e), bounds(m));
    assert!(m.source == Source::FallbackMono && dy + m_ink.3 < e_ink.1);
    assert!((dx + (m_ink.0 + m_ink.2) / 2 - e.advance / 2).abs() <= 1);
    // A mark no font has (a box): still under its letter, a 16th of an em below its ink.
    let set = ty.line("e\u{1DCA}", Face::Regular);
    let [(_, _, e), (_, dy, m)] = set[..] else { panic!() };
    assert!(m.glyph == 0 && dy + bounds(m).3 == bounds(e).1 - gap);
    // After a space (no ink), where the pen is.
    let set = ty.line(" \u{301}", Face::Regular);
    assert_eq!((set[1].0, set[1].1), (ty.width(" ", UNIT), 0));
}

#[test]
fn a_letter_and_its_marks_stay_together() {
    let ty = Type::builtin();
    // A word longer than the line breaks between letters, never before a mark.
    let word = "e\u{301}e\u{301}e\u{301}e\u{301}";
    let one = ty.width("é", UNIT);
    let lines = ty.breaks(word, UNIT, 2 * one + 10);
    assert_eq!(lines, vec![0..6, 6..12]);
    // The caret before a mark goes after its letter.
    assert_eq!(ty.x_at("ae\u{301}b", 2, UNIT), ty.x_at("ae\u{301}b", 1, UNIT) + ty.width("é", UNIT));
    assert_eq!(ty.x_at("ae\u{301}b", 4, UNIT), ty.width("aé", UNIT) + ty.kern_between(('é', Face::Regular), ('b', Face::Regular), UNIT));
    // Japanese written as a Mac names files: the voicing mark composed into its kana.
    assert_eq!(ty.line("\u{30AB}\u{3099}", Face::Regular).len(), 1);
    assert_eq!(ty.width("\u{30AB}\u{3099}", UNIT), ty.width("ガ", UNIT));
}

