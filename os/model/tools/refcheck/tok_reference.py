"""Strings for the tokenizer (code, many languages, odd whitespace, contractions, random characters), and a check
of the engine's tokens against the models' own tokenizer.

    python -I tok_reference.py make TOKENIZER IN.json EXPECTED.json
    (cargo run --release --features std --example tok_check -- MODEL.ccm IN.json > OURS.json)
    python -I tok_reference.py compare IN.json EXPECTED.json OURS.json

TOKENIZER: a folder with a model's tokenizer.json and tokenizer_config.json.
"""
import json, random, sys

SAMPLES = [
    "Hello world", " leading space", "trailing space ", "  two  spaces  ", "\n\nnewlines\n\n\n", "\r\nwindows\r\n",
    "I'm sure you'll see it's what they'd've DONE'S. 'S 'LL 'Re", "don’t curly", "tabs\t\tand\tmore\t",
    "numbers 1234567890 3.14159 -42 1e10 0x1F", "日本語のテキストです。漢字とかなカナ", "中文：你好，世界！",
    "한국어 텍스트 합니다", "Русский текст, ёлки-палки", "العربية النص", "עברית", "हिन्दी पाठ", "ไทย ภาษา",
    "Ελληνικά κείμενο", "emoji 😀👍🏽👨‍👩‍👧‍👦🇯🇵 done", "math ∑∫√∞ ≤≥ ±×÷", "fn main() {\n    println!(\"hi\");\n}\n",
    "def f(x):\n\treturn x  # comment\n\n\n", "<html><body class=\"a\">&amp;</body></html>", "    indented\n        more",
    "a\u00a0b\u2003c\u3000d", "zero\u200bwidth\u200djoiner", "combining e\u0301 a\u0308 n\u0303", "\x00\x01\x1f\x7f controls",
    "URL https://example.com/path?q=1&r=2#frag", "email a.b@c.de", "$PATH ~/x/y.txt C:\\Users\\Sam\\file.txt",
    "ALLCAPS and MiXeD CaSe", "punctuation!!! ??? ... ,,, ;;; :::", "''''\"\"\"\"", "a'b'c'd", " 's 't 're 've 'm 'll 'd",
    "x\n \n y", "x \n\n y", "  \n  ", " ", "", "\n", "a\r\rb", "\u2028line\u2029para", "１２３ ｆｕｌｌ ｗｉｄｔｈ",
]

def make(tokenizer_dir, inp, exp):
    from transformers import AutoTokenizer
    tok = AutoTokenizer.from_pretrained(tokenizer_dir)
    rng = random.Random(11)
    out = list(SAMPLES)
    pool = "".join(SAMPLES) + "abc XYZ 012 .,;:'\"!?()[]{}<>/\\|-_=+*&^%$#@~` \t\n\r"
    for _ in range(3000):
        n = rng.randint(1, 40)
        out.append("".join(rng.choice(pool) for _ in range(n)))
    for _ in range(1000):
        n = rng.randint(1, 20)
        s = []
        for _ in range(n):
            c = rng.randint(0, 0x2FFFF)
            if 0xD800 <= c <= 0xDFFF:
                c = 0x20
            s.append(chr(c))
        out.append("".join(s))
    ids = [tok.encode(s, add_special_tokens=False, split_special_tokens=True) for s in out]
    json.dump(out, open(inp, "w"), ensure_ascii=False)
    json.dump(ids, open(exp, "w"))
    print(len(out), "strings,", sum(map(len, ids)), "tokens")

def compare(inp, exp, ours):
    a = json.load(open(inp)); e = json.load(open(exp)); o = json.load(open(ours))
    bad = [(s, x, y) for s, x, y in zip(a, e, o) if x != y]
    for s, x, y in bad[:8]:
        print("DIFF", repr(s), x, y)
    print(len(a) - len(bad), "of", len(a), "the same")
    return not bad

if __name__ == "__main__":
    if sys.argv[1] == "make": make(sys.argv[2], sys.argv[3], sys.argv[4])
    else: sys.exit(0 if compare(sys.argv[2], sys.argv[3], sys.argv[4]) else 1)
