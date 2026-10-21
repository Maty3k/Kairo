import re, sys
lines = open(sys.argv[1], encoding='utf-8').read().split('\n')
in_code = False; header_cells = None
def cells(l):
    # split on unescaped pipes (GFM treats pipes inside code spans as separators unless escaped)
    parts = re.split(r'(?<!\\)\|', l.strip())
    return len(parts) - 2
for i, l in enumerate(lines, 1):
    if l.strip().startswith('```'):
        in_code = not in_code; continue
    if in_code: continue
    if l.startswith('|'):
        n = cells(l)
        if header_cells is None:
            header_cells = n
        elif re.match(r'^\|[-:| ]+\|$', l.strip()):
            pass
        elif n != header_cells:
            print(f"{i}: {n} cells vs {header_cells}: {l[:140]}")
    else:
        header_cells = None
