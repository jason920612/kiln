import sys,re
# usage: cond.py <javap txt> <method-substring> : condensed instruction listing
f,m=sys.argv[1],sys.argv[2]
txt=open(f,encoding='utf-8',errors='replace').read().split('\n')
on=False;out=[]
for l in txt:
    if re.match(r'  \S.*\(.*\).*;\s*$',l) or re.match(r'  (static )?\{\};',l):
        on = m in l.split('(')[0]
        if on: out.append('\n## '+l.strip())
        continue
    if not on: continue
    mm=re.match(r'\s*(\d+): (\w+)\s*(.*)',l)
    if not mm:
        if 'Exception table' in l or re.match(r'\s+\d+\s+\d+\s+\d+\s+(Class|any)',l): out.append('   '+l.strip())
        continue
    n,op,rest=mm.groups()
    c=rest.split('//',1)[1].strip() if '//' in rest else rest.strip()
    if op in('aload_0','aload_1','aload_2','aload_3','dup','pop','nop','checkcast') : continue
    if op.startswith('iconst'): c=op[7:]; op='i'
    if op.startswith(('invoke')):
        c=re.sub(r'(Interface)?Method (\S+?):\(.*',r'\2()',c)
        c=c.replace('net/minecraft/','')
    if op.startswith(('get','put')):
        c=c.replace('Field ','').replace('net/minecraft/','')
    c=c.replace('String ','S:')
    out.append(f'{n}:{op} {c}')
print('\n'.join(out))
