# Makes text/src/breaks_data.rs from the Unicode Character Database (15.1): each code point's line breaking class
# (UAX #14), with the distinctions the algorithm needs folded in: OP and CP that are East Asian wide (LB30), QU that
# are initial or final punctuation (LB15a, LB15b), SA split by general category (LB1).
# Usage: python3 -I lbgen.py UNICODE_DATA_DIR OUT.rs
import sys, os, re

src, out = sys.argv[1], sys.argv[2]

def ranges(name):
    for line in open(os.path.join(src, name), encoding='utf-8'):
        line = line.split('#', 1)[0].strip()
        if not line:
            continue
        cp, val = [x.strip() for x in line.split(';')[:2]]
        if '..' in cp:
            a, b = cp.split('..')
            yield int(a, 16), int(b, 16), val
        else:
            yield int(cp, 16), int(cp, 16), val

N = 0x110000
lb = ['XX'] * N
# Defaults for unlisted code points (the file's header).
for a, b in [(0x3400, 0x4DBF), (0x4E00, 0x9FFF), (0xF900, 0xFAFF), (0x20000, 0x2FFFD), (0x30000, 0x3FFFD), (0x1F000, 0x1FAFF), (0x1FC00, 0x1FFFD)]:
    for c in range(a, b + 1):
        lb[c] = 'ID'
for c in range(0x20A0, 0x20D0):
    lb[c] = 'PR'
for a, b, v in ranges('LineBreak.txt'):
    for c in range(a, b + 1):
        lb[c] = v

ea = ['N'] * N
for a, b, v in ranges('EastAsianWidth.txt'):
    for c in range(a, b + 1):
        ea[c] = v

gc = ['Cn'] * N
first = None
for line in open(os.path.join(src, 'UnicodeData.txt'), encoding='utf-8'):
    f = line.split(';')
    c = int(f[0], 16)
    if f[1].endswith(', First>'):
        first = c
        continue
    if f[1].endswith(', Last>') and first is not None:
        for x in range(first, c + 1):
            gc[x] = f[2]
        first = None
        continue
    gc[c] = f[2]

def cls(c):
    v = lb[c]
    if v in ('OP', 'CP') and ea[c] in ('F', 'W', 'H'):
        return v + 'W'
    if v == 'QU' and gc[c] == 'Pi':
        return 'QUPI'
    if v == 'QU' and gc[c] == 'Pf':
        return 'QUPF'
    if v == 'SA':
        return 'SACM' if gc[c] in ('Mn', 'Mc') else 'SAAL'
    return v

NAMES = ['XX', 'BK', 'CR', 'LF', 'NL', 'SP', 'ZW', 'WJ', 'GL', 'CM', 'ZWJ', 'OP', 'OPW', 'CL', 'CP', 'CPW', 'QU', 'QUPI',
         'QUPF', 'BA', 'BB', 'HY', 'B2', 'IS', 'NS', 'EX', 'SY', 'AL', 'HL', 'NU', 'PR', 'PO', 'ID', 'EB', 'EM', 'IN', 'H2',
         'H3', 'JL', 'JV', 'JT', 'RI', 'CB', 'AI', 'CJ', 'SACM', 'SAAL', 'SG', 'AK', 'AP', 'AS', 'VF', 'VI']
idx = {n: i for i, n in enumerate(NAMES)}

starts, classes = [], []
prev = None
for c in range(N):
    k = cls(c)
    if k != prev:
        starts.append(c)
        classes.append(idx[k])
        prev = k

with open(out, 'w', encoding='utf-8') as o:
    o.write('// Made by lbgen.py from the Unicode Character Database 15.1 (LineBreak.txt, EastAsianWidth.txt,\n')
    o.write('// UnicodeData.txt; © 2023 Unicode, Inc., under the Unicode License V3: see the README). Not to be edited.\n\n')
    o.write('/// Where each run of code points of one class starts (the run ends where the next starts).\n')
    o.write('pub(crate) static STARTS: [u32; %d] = [\n' % len(starts))
    for i in range(0, len(starts), 12):
        o.write('    ' + ', '.join('0x%X' % s for s in starts[i:i + 12]) + ',\n')
    o.write('];\n\n/// Each run\'s class (`Class as u8`).\n')
    o.write('pub(crate) static CLASSES: [u8; %d] = [\n' % len(classes))
    for i in range(0, len(classes), 24):
        o.write('    ' + ', '.join(str(x) for x in classes[i:i + 24]) + ',\n')
    o.write('];\n')
print(len(starts), 'runs')
print('enum order:', ' '.join(NAMES))
