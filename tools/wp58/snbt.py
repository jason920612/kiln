"""wp66: a small SNBT reader for the console's `data get` output (values keep their suffix as text)."""
import re

def parse(s):
    p = _P(s); v = p.value(); return v

class _P:
    def __init__(self, s): self.s = s; self.i = 0
    def ws(self):
        while self.i < len(self.s) and self.s[self.i] in " \t\n": self.i += 1
    def value(self):
        self.ws(); c = self.s[self.i]
        if c == "{": return self.compound()
        if c == "[": return self.list()
        if c in "\"'": return self.string()
        m = re.compile(r"[^,}\]\s]+").match(self.s, self.i); self.i = m.end(); return m.group(0)
    def string(self):
        q = self.s[self.i]; self.i += 1; out = []
        while self.s[self.i] != q:
            if self.s[self.i] == "\\": self.i += 1
            out.append(self.s[self.i]); self.i += 1
        self.i += 1; return "".join(out)
    def key(self):
        self.ws()
        if self.s[self.i] in "\"'": return self.string()
        m = re.compile(r"[^:\s]+").match(self.s, self.i); self.i = m.end(); return m.group(0)
    def compound(self):
        self.i += 1; d = {}
        while True:
            self.ws()
            if self.s[self.i] == "}": self.i += 1; return d
            k = self.key(); self.ws(); assert self.s[self.i] == ":"; self.i += 1
            d[k] = self.value(); self.ws()
            if self.s[self.i] == ",": self.i += 1
    def list(self):
        self.i += 1; out = []
        while True:
            self.ws()
            if self.s[self.i] == "]": self.i += 1; return out
            # typed arrays like [I; 1, 2]
            if self.s[self.i] in "BIL" and self.s[self.i + 1] == ";": self.i += 2; self.ws()
            out.append(self.value()); self.ws()
            if self.s[self.i] == ",": self.i += 1

def nbt_of(line):
    """The NBT of a `data get block/entity` output line (the part from the first '{' at the 'data:' / 'block data:' / '[...]' mark)."""
    i = line.find("{")
    # the key form `commands.data.block.query[x, y, z, {..}]` ends with ']'
    j = line.rfind("}")
    return parse(line[i:j + 1])
