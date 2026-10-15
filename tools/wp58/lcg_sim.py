"""wp66 item 3: numpy model of vanilla's random tick position generator (randValue * 3 + 1013904223): per-position hit counts when each tick's start state is independent (Kiln before) or continues a chain (vanilla, Kiln now). usage: python lcg_sim.py CHUNKS  (sim3: sd of the per-position totals over 40 ticks of 2000 draws per section)"""
import numpy as np, sys
M=(1<<32)-1; a=3; c=1013904223
def tab(n):
    an=np.zeros(n,dtype=np.uint64); cn=np.zeros(n,dtype=np.uint64); xa,xc=1,0
    for i in range(n):
        xa=(xa*a)&M; xc=(xc*a+c)&M; an[i]=xa; cn[i]=xc
    return an,cn
def adv(n):
    A,Cc=1,0; pa,pc=a,c
    while n:
        if n&1: A,Cc=(pa*A)&M,(pa*Cc+pc)&M
        pc=(pa*pc+pc)&M; pa=(pa*pa)&M; n>>=1
    return A,Cc
speed=2000; an,cn=tab(speed)
def pos(st):
    s=st.astype(np.int64); s=np.where(s>=1<<31,s-(1<<32),s); i=s>>2
    return (i&15)|(((i>>8)&15)<<4)|(((i>>16)&15)<<8)
rng=np.random.default_rng(int(sys.argv[2]) if len(sys.argv)>2 else 1)
def run(model,chunks,secs,T,extra=0):
    # section 1 of every target chunk is the measured one
    tot=np.zeros((chunks,4096))
    s=int(rng.integers(0,1<<32))
    for t in range(T):
        if model=="iid":
            for ch in range(chunks):
                st=(an*np.uint64(int(rng.integers(0,1<<32)))+cn)&np.uint64(M)
                tot[ch]+=np.bincount(pos(st),minlength=4096)
            continue
        # chain: chunks in order, each: precip draws, then secs sections
        for ch in range(chunks+extra):
            p=rng.binomial(speed,1/48); A,C=adv(p); s=(A*s+C)&M
            for sec in range(secs):
                st=(an*np.uint64(s)+cn)&np.uint64(M)
                if sec==1 and ch<chunks: tot[ch]+=np.bincount(pos(st),minlength=4096)
                s=int(st[-1])
    return tot
chunks=int(sys.argv[1]); T=40
for model,extra in (("iid",0),("chain",0),("chain",25)):
    tot=run(model,chunks,2,T,extra)
    # a column = 1 position; sd across all positions of the hits total
    print(model,extra,"mean",round(tot.mean(),3),"sd",round(tot.std(),3),"poisson",round(np.sqrt(tot.mean()),3))
