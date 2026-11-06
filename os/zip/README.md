# zip

Chat Computer's own decompression: ZIP archives and DEFLATE (`src/lib.rs`: what Word, Excel and PowerPoint files are
made of, and what PNG pictures are compressed with), and Brotli (`src/brotli.rs`, RFC 7932: what WOFF2 web fonts are
compressed with, and what sites send pages in).

## Brotli's dictionary and transforms

Brotli's built-in dictionary (`src/brotli_dictionary.bin`, 122,784 bytes of English words and pieces of HTML and
JavaScript), its literals' context table and its 121 word transforms are RFC 7932's, as google/brotli has them, at
commit `d89c4b1c52e25ddafe047bc78bbee5c1e24b1809` (`c/common/dictionary.bin`, `c/common/context.c`,
`c/common/transform.c`; the table and the transforms written out as Rust in `src/brotli.rs`). The decoder was checked
against that repository's test files (`tests/testdata`: all ten of x, ukkonooa, quickfox_repeated, random_org_10k.bin,
compressed_file, alice29.txt, asyoulik.txt, lcet10.txt, plrabn12.txt and mapsdatazrh decompress to their originals);
the three small ones are in `src/tests.rs`.

```
20e42eb1b511c21806d4d227d07e5dd06877d8ce7b3a817f378f313653f35c70  src/brotli_dictionary.bin
```

Those are under the MIT licence:

```
Copyright (c) 2009, 2010, 2013-2016 by the Brotli Authors.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.  IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
```
