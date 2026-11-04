//! Against the main Chat Computer's chimes (chime.py's formula, worked out by numpy in double precision at 48 kHz).

use super::*;

const AT: [usize; 18] = [0, 1, 7, 100, 239, 240, 241, 1000, 4321, 12345, 20159, 20160, 23519, 23520, 23521, 30000, 43679, 43680];
// (kind, samples, loudest, the samples at AT that it has)
const NUMPY: [(Kind, usize, i16, &[i16]); 3] = [
    (Kind::Reminder, 47040, 8209, &[0, 6, 230, -1371, -7036, -6256, -5247, 3656, -1398, -1054, 884, 0, 0, 0, 8, 2962, -37, 0]),
    (Kind::Timer, 47040, 8194, &[0, 8, 245, 3369, 7273, 6673, 6005, -7101, 5051, 2641, -37, 0, 0, 0, 10, -126, -913, 0]),
    (Kind::Alarm, 70560, 8207, &[0, 7, 239, -3442, 2938, 2574, 2172, 3656, 4836, 1321, -336, 0, 0, 0, 9, -2283, -494, 0]),
];

#[test]
fn rings_as_the_main_chat_computer_does() {
    for (kind, len, loudest, samples) in NUMPY {
        let c = chime(kind, 48_000, 0.32);
        assert_eq!(c.len(), len, "{kind:?}");
        let max = c.iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!((max as i32 - loudest as i32).abs() <= 1, "{kind:?}: loudest {max}, numpy {loudest}");
        for (&i, &want) in AT.iter().zip(samples) {
            assert!((c[i] as i32 - want as i32).abs() <= 1, "{kind:?} sample {i}: {}, numpy {want}", c[i]);
        }
    }
}

#[test]
fn its_own_sine_and_exp_are_close() {
    for k in -2000..=2000 {
        let x = k as f64 * 0.01;
        assert!((sin(x) - x.sin()).abs() < 1e-12, "sin {x}");
    }
    for k in -100..=0 {
        let x = k as f64 * 1e-4;
        assert!((exp(x) - x.exp()).abs() < 1e-15, "exp {x}");
    }
}

#[test]
fn any_rate_and_loudness() {
    let c = chime(Kind::Timer, 44_100, 1.0);
    assert_eq!(c.len(), 2 * (18522 + 3087));
    let max = c.iter().map(|s| s.unsigned_abs()).max().unwrap();
    assert!((25_000..=32_767).contains(&max), "{max}");
    assert!(chime(Kind::Timer, 48_000, 0.0).iter().all(|&s| s == 0));
}
