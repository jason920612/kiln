from lock import *
both_("tick rate 100"); time.sleep(1)
a={t:gametime(t) for t in "KV"}; time.sleep(10); b={t:gametime(t) for t in "KV"}
print({t:(b[t]-a[t])/10 for t in "KV"})
both_("tick rate 20")
