import re,sys,pathlib
def prose(t):
    out=[];incode=False
    for i,l in enumerate(t.splitlines(),1):
        if l.strip().startswith('```'): incode=not incode; continue
        if incode: continue
        out.append((i,l))
    return out
bad=0
for p in sys.argv[1:]:
    t=pathlib.Path(p).read_text()
    for ln,l in prose(t):
        cells=[c for c in l.split('|')] if l.strip().startswith('|') else [l]
        for c in cells:
            c=re.sub(r'`[^`]*`','X',c)
            c=re.sub(r'\[([^\]]*)\]\([^)]*\)',r'\1',c)
            c=re.sub(r'^\s*[-*#>\d.]+\s*','',c)
            if set(c.strip())<=set('-: '): continue
            for s in re.split(r'(?<=[.!?])\s+',c):
                w=[x for x in s.split() if re.search(r'[A-Za-z]',x)]
                if len(w)>20:
                    print(f"{p}:{ln} {len(w)}w: {s.strip()[:110]}"); bad+=1
print("VIOLATIONS", bad)
