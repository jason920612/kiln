import json, collections, sys
K = {l.rsplit(" ", 1)[0] if False else " ".join(l.split(" ", 3)[:3]): l.split(" ", 3)[3] for l in json.load(open("/tmp/chunk_K.json"))}
V = {" ".join(l.split(" ", 3)[:3]): l.split(" ", 3)[3] for l in json.load(open("/tmp/chunk_V.json"))}
keys = set(K) | set(V)
diff = [(k, K.get(k, "air"), V.get(k, "air")) for k in keys if K.get(k, "air") != V.get(k, "air")]
print("positions", len(keys), "different", len(diff))
c = collections.Counter((a.split("[")[0].replace("minecraft:", ""), b.split("[")[0].replace("minecraft:", "")) for _, a, b in diff)
print(c.most_common(25))
print(sorted(diff)[:10])
