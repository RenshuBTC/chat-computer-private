//! Brotli (RFC 7932), decompressed: what WOFF2 web fonts are compressed with, and what sites send pages, style sheets
//! and scripts in (Content-Encoding: br).
//!
//! A stream is meta-blocks, each its bytes stored as they are, or set by commands: so many literal bytes (each from
//! one of the meta-block's prefix codes for literals, chosen by the two bytes before it and the block of literals it's
//! in), then a copy of so many bytes from so far back, or, past what's been written, a word from the built-in
//! dictionary of 13,504 English words and pieces of HTML and JavaScript, as one of 121 transforms makes it. As the RFC
//! describes it, and checked against google/brotli's test files; the dictionary and the transforms are its
//! (`brotli_dictionary.bin`, its context table and transforms below, MIT: see `os/zip/README.md`). Every number is
//! checked before it's used and what comes out is capped, so damaged input gives an error, never a crash.

use super::Error;
use alloc::vec;
use alloc::vec::Vec;

/// The built-in dictionary: words of 4 to 24 letters, those of each length together.
static DICTIONARY: &[u8; 122784] = include_bytes!("brotli_dictionary.bin");

/// How many words of each length (as a power of two) the dictionary has.
const WORD_BITS: [u8; 25] = [0, 0, 0, 0, 10, 10, 11, 11, 10, 10, 10, 10, 10, 9, 9, 8, 7, 7, 8, 7, 7, 6, 6, 5, 5];

/// Literals' context lookup (RFC 7932's section 7.1, as google/brotli's context.c has it): for each context mode
/// (LSB6, MSB6, UTF8, Signed) 512 entries, the last byte's 256 then the byte before's; a literal's context is the
/// two ORed.
static CONTEXT_LOOKUP: [u8; 2048] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
    32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63,
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
    32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63,
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
    32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63,
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
    32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 6, 6, 6, 6, 7, 7, 7, 7,
    8, 8, 8, 8, 9, 9, 9, 9, 10, 10, 10, 10, 11, 11, 11, 11, 12, 12, 12, 12, 13, 13, 13, 13, 14, 14, 14, 14, 15, 15, 15, 15,
    16, 16, 16, 16, 17, 17, 17, 17, 18, 18, 18, 18, 19, 19, 19, 19, 20, 20, 20, 20, 21, 21, 21, 21, 22, 22, 22, 22, 23, 23, 23, 23,
    24, 24, 24, 24, 25, 25, 25, 25, 26, 26, 26, 26, 27, 27, 27, 27, 28, 28, 28, 28, 29, 29, 29, 29, 30, 30, 30, 30, 31, 31, 31, 31,
    32, 32, 32, 32, 33, 33, 33, 33, 34, 34, 34, 34, 35, 35, 35, 35, 36, 36, 36, 36, 37, 37, 37, 37, 38, 38, 38, 38, 39, 39, 39, 39,
    40, 40, 40, 40, 41, 41, 41, 41, 42, 42, 42, 42, 43, 43, 43, 43, 44, 44, 44, 44, 45, 45, 45, 45, 46, 46, 46, 46, 47, 47, 47, 47,
    48, 48, 48, 48, 49, 49, 49, 49, 50, 50, 50, 50, 51, 51, 51, 51, 52, 52, 52, 52, 53, 53, 53, 53, 54, 54, 54, 54, 55, 55, 55, 55,
    56, 56, 56, 56, 57, 57, 57, 57, 58, 58, 58, 58, 59, 59, 59, 59, 60, 60, 60, 60, 61, 61, 61, 61, 62, 62, 62, 62, 63, 63, 63, 63,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 4, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    8, 12, 16, 12, 12, 20, 12, 16, 24, 28, 12, 12, 32, 12, 36, 12, 44, 44, 44, 44, 44, 44, 44, 44, 44, 44, 32, 32, 24, 40, 28, 12,
    12, 48, 52, 52, 52, 48, 52, 52, 52, 48, 52, 52, 52, 52, 52, 48, 52, 52, 52, 52, 52, 48, 52, 52, 52, 52, 52, 24, 12, 28, 12, 12,
    12, 56, 60, 60, 60, 56, 60, 60, 60, 56, 60, 60, 60, 60, 60, 56, 60, 60, 60, 60, 60, 56, 60, 60, 60, 60, 60, 24, 12, 28, 12, 0,
    0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1,
    0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1,
    2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3,
    2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1,
    1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1,
    1, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 1, 1, 1, 1, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
    0, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24,
    24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24,
    32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32,
    32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32, 32,
    40, 40, 40, 40, 40, 40, 40, 40, 40, 40, 40, 40, 40, 40, 40, 40, 40, 40, 40, 40, 40, 40, 40, 40, 40, 40, 40, 40, 40, 40, 40, 40,
    40, 40, 40, 40, 40, 40, 40, 40, 40, 40, 40, 40, 40, 40, 40, 40, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 56,
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
    3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
    3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
    4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
    4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 7,
];

/// The prefixes and suffixes words are given (each its length, then its bytes).
static PREFIX_SUFFIX: [u8; 217] = [
    1, 32, 2, 44, 32, 8, 32, 111, 102, 32, 116, 104, 101, 32, 4, 32, 111, 102, 32, 2, 115, 32, 1, 46,
    5, 32, 97, 110, 100, 32, 4, 32, 105, 110, 32, 1, 34, 4, 32, 116, 111, 32, 2, 34, 62, 1, 10, 2,
    46, 32, 1, 93, 5, 32, 102, 111, 114, 32, 3, 32, 97, 32, 6, 32, 116, 104, 97, 116, 32, 1, 39, 6,
    32, 119, 105, 116, 104, 32, 6, 32, 102, 114, 111, 109, 32, 4, 32, 98, 121, 32, 1, 40, 6, 46, 32, 84,
    104, 101, 32, 4, 32, 111, 110, 32, 4, 32, 97, 115, 32, 4, 32, 105, 115, 32, 4, 105, 110, 103, 32, 2,
    10, 9, 1, 58, 3, 101, 100, 32, 2, 61, 34, 4, 32, 97, 116, 32, 3, 108, 121, 32, 1, 44, 2, 61,
    39, 5, 46, 99, 111, 109, 47, 7, 46, 32, 84, 104, 105, 115, 32, 5, 32, 110, 111, 116, 32, 3, 101, 114,
    32, 3, 97, 108, 32, 4, 102, 117, 108, 32, 4, 105, 118, 101, 32, 5, 108, 101, 115, 115, 32, 4, 101, 115,
    116, 32, 4, 105, 122, 101, 32, 2, 194, 160, 4, 111, 117, 115, 32, 5, 32, 116, 104, 101, 32, 2, 101, 32,
    0,
];

/// Where each prefix or suffix starts in PREFIX_SUFFIX.
static PREFIX_SUFFIX_AT: [u8; 50] = [0, 2, 5, 14, 19, 22, 24, 30, 35, 37, 42, 45, 47, 50, 52, 58, 62, 69, 71, 78, 85, 90, 92, 99, 104, 109, 114, 119, 122, 124, 128, 131, 136, 140, 142, 145, 151, 159, 165, 169, 173, 178, 183, 189, 194, 199, 202, 207, 213, 216];

/// The 121 transforms: (prefix, kind, suffix): the kind 0 as it is, 1 to 9 its last 1 to 9 bytes left off, 10 its
/// first letter in upper case, 11 all of it, 12 to 20 its first 1 to 9 bytes left off.
static TRANSFORMS: [(u8, u8, u8); 121] = [
    (49, 0, 49), (49, 0, 0), (0, 0, 0), (49, 12, 49), (49, 10, 0), (49, 0, 47),
    (0, 0, 49), (4, 0, 0), (49, 0, 3), (49, 10, 49), (49, 0, 6), (49, 13, 49),
    (49, 1, 49), (1, 0, 0), (49, 0, 1), (0, 10, 0), (49, 0, 7), (49, 0, 9),
    (48, 0, 0), (49, 0, 8), (49, 0, 5), (49, 0, 10), (49, 0, 11), (49, 3, 49),
    (49, 0, 13), (49, 0, 14), (49, 14, 49), (49, 2, 49), (49, 0, 15), (49, 0, 16),
    (0, 10, 49), (49, 0, 12), (5, 0, 49), (0, 0, 1), (49, 15, 49), (49, 0, 18),
    (49, 0, 17), (49, 0, 19), (49, 0, 20), (49, 16, 49), (49, 17, 49), (47, 0, 49),
    (49, 4, 49), (49, 0, 22), (49, 11, 49), (49, 0, 23), (49, 0, 24), (49, 0, 25),
    (49, 7, 49), (49, 1, 26), (49, 0, 27), (49, 0, 28), (0, 0, 12), (49, 0, 29),
    (49, 20, 49), (49, 18, 49), (49, 6, 49), (49, 0, 21), (49, 10, 1), (49, 8, 49),
    (49, 0, 31), (49, 0, 32), (47, 0, 3), (49, 5, 49), (49, 9, 49), (0, 10, 1),
    (49, 10, 8), (5, 0, 21), (49, 11, 0), (49, 10, 10), (49, 0, 30), (0, 0, 5),
    (35, 0, 49), (47, 0, 2), (49, 10, 17), (49, 0, 36), (49, 0, 33), (5, 0, 0),
    (49, 10, 21), (49, 10, 5), (49, 0, 37), (0, 0, 30), (49, 0, 38), (0, 11, 0),
    (49, 0, 39), (0, 11, 49), (49, 0, 34), (49, 11, 8), (49, 10, 12), (0, 0, 21),
    (49, 0, 40), (0, 10, 12), (49, 0, 41), (49, 0, 42), (49, 11, 17), (49, 0, 43),
    (0, 10, 5), (49, 11, 10), (0, 0, 34), (49, 10, 33), (49, 0, 44), (49, 11, 5),
    (45, 0, 49), (0, 0, 33), (49, 10, 30), (49, 11, 30), (49, 0, 46), (49, 11, 1),
    (49, 10, 34), (0, 10, 33), (0, 11, 30), (0, 11, 1), (49, 11, 33), (49, 11, 21),
    (49, 11, 12), (0, 11, 5), (49, 11, 34), (0, 11, 12), (0, 10, 30), (0, 11, 34),
    (0, 10, 34),
];

/// Insert lengths' codes: each its base and how many extra bits (RFC 7932's section 5).
const INSERT_BASE: [u32; 24] = [0, 1, 2, 3, 4, 5, 6, 8, 10, 14, 18, 26, 34, 50, 66, 98, 130, 194, 322, 578, 1090, 2114, 6210, 22594];
const INSERT_EXTRA: [u32; 24] = [0, 0, 0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 7, 8, 9, 10, 12, 14, 24];
/// Copy lengths' codes.
const COPY_BASE: [u32; 24] = [2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 14, 18, 22, 30, 38, 54, 70, 102, 134, 198, 326, 582, 1094, 2118];
const COPY_EXTRA: [u32; 24] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 7, 8, 9, 10, 24];
/// Block lengths' codes.
const BLOCK_BASE: [u32; 26] = [
    1, 5, 9, 13, 17, 25, 33, 41, 49, 65, 81, 97, 113, 145, 177, 209, 241, 305, 369, 497, 753, 1265, 2289, 4337, 8433, 16625,
];
const BLOCK_EXTRA: [u32; 26] = [2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 6, 6, 7, 8, 9, 10, 11, 12, 13, 24];
/// An insert-and-copy command's cell (its code over 64): where its insert code and copy code start, and whether its
/// distance is the last one, with no distance code of its own.
const CELLS: [(u32, u32, bool); 11] = [
    (0, 0, true),
    (0, 8, true),
    (0, 0, false),
    (0, 8, false),
    (8, 0, false),
    (8, 8, false),
    (0, 16, false),
    (16, 0, false),
    (8, 16, false),
    (16, 8, false),
    (16, 16, false),
];
/// The order the code length code's own lengths come in.
const CODE_LENGTH_ORDER: [usize; 18] = [1, 2, 3, 4, 0, 5, 17, 6, 16, 7, 8, 9, 10, 11, 12, 13, 14, 15];
/// The code a code length code's length is read with, by the next four bits: how many it takes, and the length.
const CODE_LENGTH_BITS: [u8; 16] = [2, 2, 2, 3, 2, 2, 2, 4, 2, 2, 2, 3, 2, 2, 2, 4];
const CODE_LENGTH_VALUE: [u8; 16] = [0, 4, 3, 2, 0, 4, 3, 1, 0, 4, 3, 2, 0, 4, 3, 5];

/// Bits, each byte read from its lowest bit up.
struct Bits<'a> {
    input: &'a [u8],
    at: usize,
    buf: u64,
    count: u32,
}

impl Bits<'_> {
    fn fill(&mut self) {
        while self.count <= 56 && self.at < self.input.len() {
            self.buf |= (self.input[self.at] as u64) << self.count;
            self.at += 1;
            self.count += 8;
        }
    }

    /// The next `n` bits (at most 32), the first lowest.
    fn read(&mut self, n: u32) -> Result<u32, Error> {
        if n == 0 {
            return Ok(0);
        }
        self.fill();
        if self.count < n {
            return Err(Error::Damaged);
        }
        let v = (self.buf & ((1u64 << n) - 1)) as u32;
        self.buf >>= n;
        self.count -= n;
        Ok(v)
    }

    /// The next `n` bits without taking them (0s past the end).
    fn peek(&mut self, n: u32) -> u32 {
        self.fill();
        (self.buf & ((1u64 << n) - 1)) as u32
    }

    fn skip(&mut self, n: u32) -> Result<(), Error> {
        self.read(n).map(|_| ())
    }

    /// On to the next byte's start (the bits left over in this one must be 0).
    fn align(&mut self) -> Result<(), Error> {
        let n = self.count % 8;
        if self.read(n)? != 0 { Err(Error::Damaged) } else { Ok(()) }
    }

    fn byte(&mut self) -> Result<u8, Error> {
        Ok(self.read(8)? as u8)
    }

    /// A number 0 to 255 as RFC 7932 writes those that are often small (its VarLenUint8).
    fn var_u8(&mut self) -> Result<u32, Error> {
        if self.read(1)? == 0 {
            return Ok(0);
        }
        let n = self.read(3)?;
        if n == 0 { Ok(1) } else { Ok((1 << n) + self.read(n)?) }
    }
}

/// A prefix code (canonical: the codes in order of their length, then their symbol), decoded a bit at a time, as
/// zlib's puff decodes them.
struct Code {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Code {
    /// From each symbol's code length (0: not used).
    fn new(lengths: &[u8]) -> Result<Code, Error> {
        let mut counts = [0u16; 16];
        for &l in lengths {
            counts[l as usize] += 1;
        }
        counts[0] = 0;
        let mut left: i32 = 1;
        for &c in &counts[1..] {
            left <<= 1;
            left -= c as i32;
            if left < 0 {
                return Err(Error::Damaged);
            }
        }
        let mut offsets = [0u16; 16];
        for l in 1..15 {
            offsets[l + 1] = offsets[l] + counts[l];
        }
        let used = counts.iter().map(|&c| c as usize).sum::<usize>();
        let mut symbols = vec![0u16; used];
        for (s, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[offsets[l as usize] as usize] = s as u16;
                offsets[l as usize] += 1;
            }
        }
        Ok(Code { counts, symbols })
    }

    /// One symbol only: it's read with no bits.
    fn single(symbol: u16) -> Code {
        Code { counts: [0; 16], symbols: vec![symbol] }
    }

    fn decode(&self, bits: &mut Bits) -> Result<u16, Error> {
        if self.symbols.len() == 1 && self.counts.iter().all(|&c| c == 0) {
            return Ok(self.symbols[0]);
        }
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= bits.read(1)? as i32;
            let count = self.counts[len] as i32;
            if code - first < count {
                return Ok(self.symbols[(index + code - first) as usize]);
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err(Error::Damaged)
    }
}

/// A prefix code for an alphabet of `size` symbols, as the stream describes it: simple (one to four symbols) or
/// complex (each symbol's length, themselves coded).
fn read_code(bits: &mut Bits, size: usize) -> Result<Code, Error> {
    let kind = bits.read(2)?;
    if kind == 1 {
        let n = bits.read(2)? as usize + 1;
        let width = 32 - (size.max(2) as u32 - 1).leading_zeros();
        let mut symbols = [0u16; 4];
        for s in symbols.iter_mut().take(n) {
            *s = bits.read(width)? as u16;
            if *s as usize >= size {
                return Err(Error::Damaged);
            }
        }
        if (0..n).any(|i| (i + 1..n).any(|j| symbols[i] == symbols[j])) {
            return Err(Error::Damaged);
        }
        let lengths_in_order: &[u8] = match n {
            1 => return Ok(Code::single(symbols[0])),
            2 => &[1, 1],
            3 => &[1, 2, 2],
            _ => {
                if bits.read(1)? == 0 {
                    &[2, 2, 2, 2]
                } else {
                    &[1, 2, 3, 3]
                }
            }
        };
        let mut lengths = vec![0u8; size];
        for i in 0..n {
            lengths[symbols[i] as usize] = lengths_in_order[i];
        }
        return Code::new(&lengths);
    }
    // Complex: the code length code's lengths (from the `kind`th on), then each symbol's length in it.
    let mut cl = [0u8; 18];
    let mut space: i32 = 32;
    let mut used = 0;
    for &sym in &CODE_LENGTH_ORDER[kind as usize..] {
        let p = bits.peek(4) as usize;
        bits.skip(CODE_LENGTH_BITS[p] as u32)?;
        let v = CODE_LENGTH_VALUE[p];
        cl[sym] = v;
        if v != 0 {
            space -= 32 >> v;
            used += 1;
            if space <= 0 {
                break;
            }
        }
    }
    if used != 1 && space != 0 {
        return Err(Error::Damaged);
    }
    let cl_code = if used == 1 {
        Code::single(cl.iter().position(|&l| l != 0).unwrap_or(0) as u16)
    } else {
        Code::new(&cl)?
    };
    let mut lengths = vec![0u8; size];
    let (mut prev, mut repeat, mut repeat_len) = (8u8, 0u32, 0u8);
    let mut space: i32 = 32768;
    let mut i = 0;
    while i < size && space > 0 {
        let s = cl_code.decode(bits)?;
        if s < 16 {
            repeat = 0;
            lengths[i] = s as u8;
            i += 1;
            if s != 0 {
                prev = s as u8;
                space -= 32768 >> s;
            }
        } else {
            let (extra, len) = if s == 16 { (2, prev) } else { (3, 0) };
            if repeat_len != len {
                repeat = 0;
                repeat_len = len;
            }
            let old = repeat;
            if repeat > 0 {
                repeat = (repeat - 2) << extra;
            }
            repeat += bits.read(extra)? + 3;
            let delta = (repeat - old) as usize;
            if i + delta > size {
                return Err(Error::Damaged);
            }
            for l in &mut lengths[i..i + delta] {
                *l = repeat_len;
            }
            i += delta;
            if repeat_len != 0 {
                space -= delta as i32 * (32768 >> repeat_len);
            }
        }
    }
    if space != 0 {
        return Err(Error::Damaged);
    }
    Code::new(&lengths)
}

/// A block length, by its code.
fn block_length(bits: &mut Bits, code: &Code) -> Result<u32, Error> {
    let s = code.decode(bits)? as usize;
    if s >= 26 {
        return Err(Error::Damaged);
    }
    Ok(BLOCK_BASE[s] + bits.read(BLOCK_EXTRA[s])?)
}

/// A category's (literals', commands' or distances') blocks: how many types, the codes for switching, the type now and
/// the one before, and how much of the block now is left.
struct Blocks {
    types: u32,
    type_code: Option<Code>,
    length_code: Option<Code>,
    now: u32,
    before: u32,
    left: u32,
}

impl Blocks {
    fn read(bits: &mut Bits) -> Result<Blocks, Error> {
        let types = bits.var_u8()? + 1;
        if types < 2 {
            return Ok(Blocks { types, type_code: None, length_code: None, now: 0, before: 1, left: 1 << 24 });
        }
        let type_code = read_code(bits, types as usize + 2)?;
        let length_code = read_code(bits, 26)?;
        let left = block_length(bits, &length_code)?;
        Ok(Blocks { types, type_code: Some(type_code), length_code: Some(length_code), now: 0, before: 1, left })
    }

    /// One more of the category: a new block (and its type) first, if this one's done.
    fn next(&mut self, bits: &mut Bits) -> Result<(), Error> {
        if self.left == 0 {
            let (Some(tc), Some(lc)) = (&self.type_code, &self.length_code) else { return Err(Error::Damaged) };
            let code = tc.decode(bits)? as u32;
            let mut t = match code {
                0 => self.before,
                1 => self.now + 1,
                n => n - 2,
            };
            if t >= self.types {
                t -= self.types;
            }
            self.before = self.now;
            self.now = t;
            self.left = block_length(bits, lc)?;
        }
        self.left = self.left.saturating_sub(1);
        Ok(())
    }
}

/// A context map: which of `trees` prefix codes each of `size` contexts uses (zeros run-length coded, then perhaps
/// moved to the front).
fn context_map(bits: &mut Bits, size: usize, trees: u32) -> Result<Vec<u8>, Error> {
    if trees < 2 {
        return Ok(vec![0; size]);
    }
    let rle = if bits.read(1)? == 1 { bits.read(4)? + 1 } else { 0 };
    let code = read_code(bits, (trees + rle) as usize)?;
    let mut map = vec![0u8; size];
    let mut i = 0;
    while i < size {
        let c = code.decode(bits)? as u32;
        if c == 0 {
            i += 1;
        } else if c <= rle {
            let run = (1usize << c) + bits.read(c)? as usize;
            if i + run > size {
                return Err(Error::Damaged);
            }
            i += run;
        } else {
            map[i] = (c - rle) as u8;
            i += 1;
        }
    }
    if bits.read(1)? == 1 {
        let mut mtf: [u8; 256] = core::array::from_fn(|i| i as u8);
        for v in map.iter_mut() {
            let at = *v as usize;
            let value = mtf[at];
            *v = value;
            mtf.copy_within(0..at, 1);
            mtf[0] = value;
        }
    }
    Ok(map)
}

/// A dictionary word, transformed, onto `out`.
fn word(out: &mut Vec<u8>, len: usize, index: usize, transform: usize) -> Result<usize, Error> {
    let bits = WORD_BITS[len] as usize;
    let at: usize = (4..len).map(|l| l << WORD_BITS[l]).sum::<usize>() + index * len;
    let word = DICTIONARY.get(at..at + len).ok_or(Error::Damaged)?;
    let _ = bits;
    let (pre, kind, suf) = TRANSFORMS.get(transform).copied().ok_or(Error::Damaged)?;
    let affix = |id: u8| -> &'static [u8] {
        let at = PREFIX_SUFFIX_AT[id as usize] as usize;
        let n = PREFIX_SUFFIX[at] as usize;
        &PREFIX_SUFFIX[at + 1..at + 1 + n]
    };
    let start = out.len();
    out.extend_from_slice(affix(pre));
    let kind = kind as usize;
    let body = if kind <= 9 {
        &word[..len.saturating_sub(kind)]
    } else if (12..=20).contains(&kind) {
        &word[(kind - 11).min(len)..]
    } else {
        word
    };
    let from = out.len();
    out.extend_from_slice(body);
    if kind == 10 || kind == 11 {
        let mut i = from;
        while i < out.len() {
            let w = &mut out[i..];
            let step = if w[0] < 0xC0 {
                if w[0].is_ascii_lowercase() {
                    w[0] ^= 32;
                }
                1
            } else if w[0] < 0xE0 {
                if w.len() > 1 {
                    w[1] ^= 32;
                }
                2
            } else {
                if w.len() > 2 {
                    w[2] ^= 5;
                }
                3
            };
            if kind == 10 {
                break;
            }
            i += step;
        }
    }
    out.extend_from_slice(affix(suf));
    Ok(out.len() - start)
}

/// A Brotli stream decompressed: an error if it's damaged, or if it would come out bigger than `most` bytes.
pub fn brotli(input: &[u8], most: usize) -> Result<Vec<u8>, Error> {
    let mut bits = Bits { input, at: 0, buf: 0, count: 0 };
    // The window: 2^wbits − 16 bytes back at most.
    let wbits = if bits.read(1)? == 0 {
        16
    } else {
        match bits.read(3)? {
            0 => match bits.read(3)? {
                0 => 17,
                1 => return Err(Error::Damaged),
                m => 8 + m,
            },
            n => 17 + n,
        }
    };
    let window = (1usize << wbits) - 16;
    let mut out: Vec<u8> = Vec::new();
    let mut ring = [16usize, 15, 11, 4];
    let mut ring_at = 3;
    loop {
        let last = bits.read(1)? == 1;
        if last && bits.read(1)? == 1 {
            break;
        }
        let nibbles = match bits.read(2)? {
            3 => 0,
            n => n + 4,
        };
        if nibbles == 0 {
            // Metadata: skipped.
            if bits.read(1)? != 0 {
                return Err(Error::Damaged);
            }
            let skip_bytes = bits.read(2)?;
            let mut skip = 0usize;
            for i in 0..skip_bytes {
                skip |= (bits.read(8)? as usize) << (8 * i);
            }
            bits.align()?;
            if skip_bytes > 0 {
                for _ in 0..skip + 1 {
                    bits.byte()?;
                }
            }
            if last {
                break;
            }
            continue;
        }
        let mut len = 0usize;
        for i in 0..nibbles {
            len |= (bits.read(4)? as usize) << (4 * i);
        }
        let mut left = len + 1;
        if out.len() + left > most {
            return Err(Error::TooBig);
        }
        if !last && bits.read(1)? == 1 {
            // Stored as it is.
            bits.align()?;
            for _ in 0..left {
                out.push(bits.byte()?);
            }
            continue;
        }
        let mut lit_blocks = Blocks::read(&mut bits)?;
        let mut cmd_blocks = Blocks::read(&mut bits)?;
        let mut dist_blocks = Blocks::read(&mut bits)?;
        let postfix = bits.read(2)?;
        let direct = bits.read(4)? << postfix;
        let mut modes = Vec::with_capacity(lit_blocks.types as usize);
        for _ in 0..lit_blocks.types {
            modes.push(bits.read(2)? as usize);
        }
        let lit_trees = bits.var_u8()? + 1;
        let lit_map = context_map(&mut bits, 64 * lit_blocks.types as usize, lit_trees)?;
        let dist_trees = bits.var_u8()? + 1;
        let dist_map = context_map(&mut bits, 4 * dist_blocks.types as usize, dist_trees)?;
        let mut lit_codes = Vec::with_capacity(lit_trees as usize);
        for _ in 0..lit_trees {
            lit_codes.push(read_code(&mut bits, 256)?);
        }
        let mut cmd_codes = Vec::with_capacity(cmd_blocks.types as usize);
        for _ in 0..cmd_blocks.types {
            cmd_codes.push(read_code(&mut bits, 704)?);
        }
        let dist_size = 16 + direct as usize + (48usize << postfix);
        let mut dist_codes = Vec::with_capacity(dist_trees as usize);
        for _ in 0..dist_trees {
            dist_codes.push(read_code(&mut bits, dist_size)?);
        }
        'commands: while left > 0 {
            cmd_blocks.next(&mut bits)?;
            let cmd = cmd_codes[cmd_blocks.now as usize].decode(&mut bits)? as u32;
            let (ins_from, copy_from, last_distance) = CELLS[(cmd >> 6) as usize];
            let ins = (ins_from + ((cmd >> 3) & 7)) as usize;
            let copy = (copy_from + (cmd & 7)) as usize;
            let insert_len = (INSERT_BASE[ins] + bits.read(INSERT_EXTRA[ins])?) as usize;
            let copy_len = (COPY_BASE[copy] + bits.read(COPY_EXTRA[copy])?) as usize;
            for _ in 0..insert_len {
                lit_blocks.next(&mut bits)?;
                let n = out.len();
                let p1 = if n > 0 { out[n - 1] as usize } else { 0 };
                let p2 = if n > 1 { out[n - 2] as usize } else { 0 };
                let mode = modes[lit_blocks.now as usize] * 512;
                let ctx = (CONTEXT_LOOKUP[mode + p1] | CONTEXT_LOOKUP[mode + 256 + p2]) as usize;
                let tree = lit_map[lit_blocks.now as usize * 64 + ctx] as usize;
                out.push(lit_codes.get(tree).ok_or(Error::Damaged)?.decode(&mut bits)? as u8);
                left -= 1;
                if left == 0 {
                    break 'commands;
                }
            }
            // The distance: the last again, or one the command's code gives.
            let (distance, push) = if last_distance {
                (ring[ring_at], false)
            } else {
                dist_blocks.next(&mut bits)?;
                let ctx = if copy_len > 4 { 3 } else { copy_len - 2 };
                let tree = dist_map[dist_blocks.now as usize * 4 + ctx] as usize;
                let code = dist_codes.get(tree).ok_or(Error::Damaged)?.decode(&mut bits)? as usize;
                let last1 = ring[ring_at];
                let back = |k: usize| ring[(ring_at + 4 - k) & 3];
                let d: i64 = match code {
                    0 => last1 as i64,
                    1 => back(1) as i64,
                    2 => back(2) as i64,
                    3 => back(3) as i64,
                    4..=9 => {
                        let delta = [-1i64, 1, -2, 2, -3, 3][code - 4];
                        last1 as i64 + delta
                    }
                    10..=15 => {
                        let delta = [-1i64, 1, -2, 2, -3, 3][code - 10];
                        back(1) as i64 + delta
                    }
                    c if c < 16 + direct as usize => (c - 15) as i64,
                    c => {
                        let c = c - direct as usize - 16;
                        let ndistbits = 1 + (c >> (postfix + 1)) as u32;
                        let hcode = c >> postfix;
                        let lcode = c & ((1 << postfix) - 1);
                        let offset = ((2 + (hcode & 1)) << ndistbits) - 4;
                        let extra = bits.read(ndistbits)? as usize;
                        (((offset + extra) << postfix) + lcode + direct as usize + 1) as i64
                    }
                };
                if d <= 0 {
                    return Err(Error::Damaged);
                }
                (d as usize, code != 0)
            };
            let reach = out.len().min(window);
            if distance > reach {
                // A dictionary word.
                if !(4..=24).contains(&copy_len) {
                    return Err(Error::Damaged);
                }
                let id = distance - reach - 1;
                let bits_for = WORD_BITS[copy_len] as usize;
                let (index, transform) = (id & ((1 << bits_for) - 1), id >> bits_for);
                let made = word(&mut out, copy_len, index, transform)?;
                if made > left {
                    return Err(Error::Damaged);
                }
                left -= made;
            } else {
                if copy_len > left {
                    return Err(Error::Damaged);
                }
                let from = out.len() - distance;
                for k in 0..copy_len {
                    let b = out[from + k];
                    out.push(b);
                }
                left -= copy_len;
                if push {
                    ring_at = (ring_at + 1) & 3;
                    ring[ring_at] = distance;
                }
            }
        }
        if last {
            break;
        }
    }
    Ok(out)
}
