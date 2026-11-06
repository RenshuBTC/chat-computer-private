# Makes browser/src/entities_data.rs: HTML's named character references (from Python's html.entities.html5, which
# is WHATWG HTML's list). Usage: python3 -I entgen.py OUT.rs
import sys, html.entities
d = html.entities.html5
names = sorted({k.rstrip(';') for k in d})
legacy = sorted(k for k in d if not k.endswith(';'))
def lit(s):
    out = []
    for ch in s:
        c = ord(ch)
        if ch in '"\\':
            out.append('\\' + ch)
        elif 32 <= c < 127:
            out.append(ch)
        else:
            out.append('\\u{%X}' % c)
    return '"' + ''.join(out) + '"'
with open(sys.argv[1], 'w', encoding='utf-8') as o:
    o.write('// Made by entgen.py from the WHATWG HTML standard\'s named character references (as Python\'s\n')
    o.write('// html.entities.html5 has them). Not to be edited.\n\n')
    o.write('/// Each name (without its ";"), sorted, and what it stands for.\n')
    o.write('pub(crate) static NAMED: [(&str, &str); %d] = [\n' % len(names))
    for n in names:
        v = d.get(n + ';', d.get(n))
        o.write('    (%s, %s),\n' % (lit(n), lit(v)))
    o.write('];\n\n/// The names that stand for their characters without a ";" too (sorted).\n')
    o.write('pub(crate) static LEGACY: [&str; %d] = [\n' % len(legacy))
    for i in range(0, len(legacy), 10):
        o.write('    ' + ', '.join(lit(n) for n in legacy[i:i + 10]) + ',\n')
    o.write('];\n')
print(len(names), len(legacy))
