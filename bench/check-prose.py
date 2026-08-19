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
# Exit non-zero when a rule is broken, so the gate command in AGENTS.md can actually fail.
#
# This script returned 0 whatever it found, so `set -e` and any exit-code check were worthless
# against it, and it was the only gate command that could not fail. CI caught two violations that a
# local gate run had reported and discarded, because CI greps the output instead of trusting the
# status. `check-ids.py` already exits with its count, and now so does this.
#
# CI is unaffected: it pipes into `tee`, so the pipeline status comes from `tee` and the grep still
# decides.
raise SystemExit(1 if bad else 0)
