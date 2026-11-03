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

/// A size at which a font unit is a 256th of a pixel: widths come out in font units.
const UNIT: i32 = 1000;

fn starts(ty: &Type, text: &str, face: Face) -> Vec<i32> {
    ty.line(text, face).map(|(x, _)| x).collect()
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
