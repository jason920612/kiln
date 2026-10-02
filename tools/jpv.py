"""Compact pseudo-Java view of vanilla bytecode (a javap reader, for porting behaviour).

usage: python tools/jpv.py <Class> [method-regex] [--sig] [--raw]

Runs `javap -c -p -v` on the class in the server jar (KILN_JAR, else <work>/versions/*/server-*.jar)
and prints every method as linear pseudo-Java: stack values folded into expressions, `goto`s
and labels kept, lambdas named by their implementation method. `--sig` lists fields and method
signatures only. Class is a simple name (`Brain`), an inner class (`Brain$Provider`) or a full name.
This only reads what `javap` shows; it is a reading aid, not a decompiler (branches stay gotos).
"""
import functools
import os
import re
import subprocess
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def jar():
    if os.environ.get("KILN_JAR"):
        return os.environ["KILN_JAR"]
    work = Path(os.environ.get("KILN_WORK") or ROOT / "work")
    for cand in [work, ROOT.parent.parent.parent / "work", Path.home() / "Documents/claudegame/5/work"]:
        js = sorted((cand / "versions").glob("*/server-*.jar"))
        if js:
            return str(js[-1])
    sys.exit("no server jar (set KILN_JAR)")


@functools.lru_cache(None)
def index():
    z = zipfile.ZipFile(jar())
    return [n[:-6].replace("/", ".") for n in z.namelist() if n.endswith(".class")]


def resolve(name):
    if name.startswith("net."):
        return name
    hits = [c for c in index() if c.split(".")[-1] == name]
    if not hits:
        hits = [c for c in index() if c.endswith("." + name) or c.endswith("$" + name)]
    if not hits:
        sys.exit("no class " + name)
    if len(hits) > 1:
        print("// ambiguous:", hits, file=sys.stderr)
    return hits[0]


def short(t):
    t = t.replace("/", ".")
    for p in ("net.minecraft.world.entity.ai.", "net.minecraft.world.entity.", "net.minecraft.world.level.", "net.minecraft.world.", "net.minecraft.", "java.util.", "java.lang.", "com.mojang.datafixers.util."):
        if t.startswith(p):
            t = t[len(p):]
    return t


def parse_desc(desc):
    """Argument descriptors and the return descriptor of a method descriptor."""
    m = re.match(r"\((.*)\)(.*)", desc)
    args, ret = m.group(1), m.group(2)
    out = []
    i = 0
    while i < len(args):
        j = i
        while args[j] == "[":
            j += 1
        if args[j] == "L":
            j = args.index(";", j)
        out.append(args[i:j + 1])
        i = j + 1
    return out, ret


def dtype(d):
    return {"J": "long", "D": "double", "I": "int", "F": "float", "Z": "bool", "B": "byte", "C": "char", "S": "short", "V": "void"}.get(d, d)


class Method:
    def __init__(self, header):
        self.header = header
        self.code = []  # (offset, op, operand)
        self.static = "static" in header.split("(")[0].split()


FIELD = re.compile(r"Field (?:([\w/$]+)\.)?([\w$]+):(.*)")
METH = re.compile(r"(?:Method|InterfaceMethod) (?:([\w/$\[;]+)\.)?\"?([\w$<>]+)\"?:(.*)")
INSN = re.compile(r"^\s+(\d+): (\w+)\s*(.*?)\s*$")


def load(cls, sig_only):
    cmd = ["javap", "-cp", jar(), "-p", "-constants"] + ([] if sig_only else ["-c", "-v"]) + [cls]
    out = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace").stdout
    return out


def bootstrap_table(text):
    """Bootstrap index -> lambda implementation method (Class.name)."""
    table = {}
    if "BootstrapMethods:" not in text:
        return table
    sect = text.split("BootstrapMethods:")[1]
    for m in re.finditer(r"^\s+(\d+): #\d+ REF_invokeStatic java/lang/invoke/(?:LambdaMetafactory|StringConcatFactory)\.(\w+):.*?(?=^\s+\d+: #|\Z)", sect, re.S | re.M):
        body = m.group(0)
        impl = re.search(r"#\d+ REF_(?:invokeStatic|invokeVirtual|invokeSpecial|invokeInterface|newInvokeSpecial) ([\w/$]+)\.([\w$<>]+):", body.split("Method arguments:")[-1]) if "Method arguments:" in body else None
        table[int(m.group(1))] = (m.group(2), impl.group(2) if impl else None, impl.group(1) if impl else None)
    return table


class Sim:
    def __init__(self, method, boots):
        self.method = method
        self.boots = boots
        self.stack = []  # (expr, wide)
        self.lines = []
        self.tmp = 0
        self.newid = 0
        self.labels = set()
        self.layouts = {}  # label offset -> stack layout names

    def p(self, e, wide=False):
        self.stack.append((e, wide))

    def pop(self):
        return self.stack.pop()[0]

    def local(self, n):
        m = self.method
        if n == 0 and not m.static:
            return "this"
        return f"l{n}"

    def emit(self, s):
        self.lines.append(s)

    def run(self):
        code = self.method.code
        for off, op, arg in code:
            if op.startswith(("if", "goto")) and re.match(r"\d+$", arg.split(" ")[0] if arg else ""):
                self.labels.add(int(arg.split(" ")[0]))
            elif op in ("tableswitch", "lookupswitch"):
                pass
        for off, op, arg in code:
            if off in self.labels:
                if off in self.layouts:
                    lay = self.layouts[off]
                    if self.stack and [s for s, _ in self.stack] != lay:
                        for (s, _), name in zip(self.stack, lay):
                            if s != name:
                                self.emit(f"{name} = {s};")
                    self.stack = [(n, False) for n in lay]
                self.emit(f"L{off}:")
            self.step(off, op, arg)
        return self.lines

    def cmp_jump(self, off, cond, target):
        # spill non-empty stack so the merge point sees names
        self.spill(target)
        self.emit(f"if ({cond}) goto L{target};")

    def spill(self, target):
        if self.stack:
            names = []
            for i, (s, w) in enumerate(self.stack):
                name = f"$s{i}"
                if s != name:
                    self.emit(f"{name} = {s};")
                names.append(name)
            self.stack = [(n, False) for n in names]
            self.layouts.setdefault(target, names)

    def step(self, off, op, arg):
        p, pop = self.p, self.pop
        a = arg.split("//")[0].strip()
        comment = arg.split("//", 1)[1].strip() if "//" in arg else ""
        if op == "aconst_null":
            p("null")
        elif op.startswith("iconst_"):
            v = op[7:]
            p("-1" if v == "m1" else v)
        elif op in ("lconst_0", "lconst_1"):
            p(op[-1] + "L", True)
        elif op.startswith("fconst_"):
            p(op[-1] + "f")
        elif op.startswith("dconst_"):
            p(op[-1] + "d", True)
        elif op in ("bipush", "sipush"):
            p(a)
        elif op in ("ldc", "ldc_w"):
            p(re.sub(r"^(String|int|float|class) ", lambda m: {"String": "", "int": "", "float": "", "class": "class "}[m.group(1)], comment) if not comment.startswith("String") else '"' + comment[7:] + '"')
        elif op == "ldc2_w":
            p(comment.replace("double ", "").replace("long ", ""), True)
        elif op.endswith("load") and op[0] in "ilfda" and op != "aaload":
            p(self.local(int(a)), op[0] in "ld")
        elif re.match(r"[ilfda]load_\d", op):
            p(self.local(int(op[-1])), op[0] in "ld")
        elif op.endswith("store") and op[0] in "ilfda" and op not in ("iastore",):
            v = pop()
            self.emit(f"{self.local(int(a))} = {v};")
        elif re.match(r"[ilfda]store_\d", op):
            v = pop()
            self.emit(f"{self.local(int(op[-1]))} = {v};")
        elif op in ("iaload", "laload", "faload", "daload", "aaload", "baload", "caload", "saload"):
            i = pop(); ar = pop(); p(f"{ar}[{i}]", op[0] in "ld")
        elif op in ("iastore", "lastore", "fastore", "dastore", "aastore", "bastore", "castore", "sastore"):
            v = pop(); i = pop(); ar = pop(); self.emit(f"{ar}[{i}] = {v};")
        elif op == "pop":
            v = pop()
            if not re.match(r"^[\w.$]+$", v):
                self.emit(f"{v};")
        elif op == "pop2":
            v = self.stack.pop()
            if not v[1]:
                self.stack.pop()
        elif op == "dup":
            self.materialize(-1)
            self.stack.append(self.stack[-1])
        elif op == "dup_x1":
            self.materialize(-1)
            v1 = self.stack.pop(); v2 = self.stack.pop(); self.stack += [v1, v2, v1]
        elif op == "dup_x2":
            self.materialize(-1)
            v1 = self.stack.pop(); v2 = self.stack.pop()
            if v2[1]:
                self.stack += [v1, v2, v1]
            else:
                v3 = self.stack.pop(); self.stack += [v1, v3, v2, v1]
        elif op == "dup2":
            self.materialize(-1)
            if not self.stack[-1][1] and len(self.stack) > 1:
                self.materialize(-2)
            v1 = self.stack[-1]
            if v1[1]:
                self.stack.append(v1)
            else:
                self.stack += [self.stack[-2], v1]
        elif op == "dup2_x1":
            self.materialize(-1)
            v1 = self.stack.pop()
            if v1[1]:
                v2 = self.stack.pop(); self.stack += [v1, v2, v1]
            else:
                v2 = self.stack.pop(); v3 = self.stack.pop(); self.stack += [v2, v1, v3, v2, v1]
        elif op == "dup2_x2":
            v1 = self.stack.pop(); v2 = self.stack.pop()
            if v1[1] and v2[1]:
                self.stack += [v1, v2, v1]
            else:
                v3 = self.stack.pop(); v4 = self.stack.pop() if not v3[1] else None
                self.stack += [v2, v1] + ([v4] if v4 else []) + [v3, v2, v1]
        elif op == "swap":
            v1 = self.stack.pop(); v2 = self.stack.pop(); self.stack += [v1, v2]
        elif re.match(r"[ilfd](add|sub|mul|div|rem|shl|shr|ushr|and|or|xor)$", op):
            sym = {"add": "+", "sub": "-", "mul": "*", "div": "/", "rem": "%", "shl": "<<", "shr": ">>", "ushr": ">>>", "and": "&", "or": "|", "xor": "^"}[op[1:]]
            b = pop(); x = pop(); p(f"({x} {sym} {b})", op[0] in "ld")
        elif re.match(r"[ilfd]neg$", op):
            x = pop(); p(f"(-{x})", op[0] in "ld")
        elif op == "iinc":
            n, c = a.split(", ")
            self.emit(f"{self.local(int(n))} += {c};")
        elif re.match(r"[ilfd]2[ilfdbcs]$", op):
            x = pop(); t = {"i": "int", "l": "long", "f": "float", "d": "double", "b": "byte", "c": "char", "s": "short"}[op[2]]
            p(f"(({t}){x})", op[2] in "ld")
        elif op in ("lcmp", "fcmpl", "fcmpg", "dcmpl", "dcmpg"):
            b = pop(); x = pop(); p(f"cmp({x}, {b})")
        elif op in ("ifeq", "ifne", "iflt", "ifge", "ifgt", "ifle"):
            x = pop(); sym = {"eq": "==", "ne": "!=", "lt": "<", "ge": ">=", "gt": ">", "le": "<="}[op[2:]]
            cm = re.match(r"cmp\((.*), (.*)\)$", x)
            if cm and self.balanced(cm):
                self.cmp_jump(off, f"{cm.group(1)} {sym} {cm.group(2)}", int(a))
            else:
                self.cmp_jump(off, f"{x} {sym} 0", int(a))
        elif op in ("ifnull", "ifnonnull"):
            x = pop(); self.cmp_jump(off, f"{x} {'==' if op == 'ifnull' else '!='} null", int(a))
        elif op.startswith("if_icmp") or op.startswith("if_acmp"):
            b = pop(); x = pop(); sym = {"eq": "==", "ne": "!=", "lt": "<", "ge": ">=", "gt": ">", "le": "<="}[op[-2:]]
            self.cmp_jump(off, f"{x} {sym} {b}", int(a))
        elif op in ("goto", "goto_w"):
            self.spill(int(a))
            self.emit(f"goto L{a};")
            self.stack = []
        elif op in ("ireturn", "lreturn", "freturn", "dreturn", "areturn"):
            self.emit(f"return {pop()};")
            self.stack = []
        elif op == "return":
            self.emit("return;")
            self.stack = []
        elif op == "athrow":
            self.emit(f"throw {pop()};")
            self.stack = []
        elif op in ("tableswitch", "lookupswitch"):
            self.emit(f"switch ({pop()}) {{ ... }}  // {a}")
        elif op == "getstatic":
            m = FIELD.match(comment)
            p(f"{short(m.group(1) or '')}.{m.group(2)}" if m else comment, m and m.group(3) in ("J", "D"))
        elif op == "putstatic":
            m = FIELD.match(comment)
            self.emit(f"{short(m.group(1) or '')}.{m.group(2)} = {pop()};")
        elif op == "getfield":
            m = FIELD.match(comment)
            o = pop(); p(f"{o}.{m.group(2)}", m.group(3) in ("J", "D"))
        elif op == "putfield":
            m = FIELD.match(comment)
            v = pop(); o = pop(); self.emit(f"{o}.{m.group(2)} = {v};")
        elif op.startswith("invoke"):
            self.invoke(op, comment)
        elif op == "new":
            self.newid += 1
            p(f"NEW#{self.newid}:{short(comment.replace('class ', ''))}")
        elif op in ("newarray", "anewarray"):
            n = pop(); p(f"new {a}[{n}]")
        elif op == "arraylength":
            p(f"{pop()}.length")
        elif op == "checkcast":
            x = pop(); p(f"(({short(comment.replace('class ', ''))}){x})")
        elif op == "instanceof":
            x = pop(); p(f"({x} instanceof {short(comment.replace('class ', ''))})")
        elif op in ("monitorenter", "monitorexit"):
            self.emit(f"{op}({pop()});")
        elif op == "nop":
            pass
        else:
            self.emit(f"// ?? {op} {arg}")

    def materialize(self, i):
        """Give a non-trivial stack value a name so a duplicated copy is not re-evaluated later."""
        e, w = self.stack[i]
        if re.match(r"^[\w.$\-]+$", e) and not e.startswith("NEW#") or e.startswith("NEW#"):
            return
        self.tmp += 1
        name = f"t{self.tmp}"
        self.emit(f"{name} = {e};")
        self.stack[i] = (name, w)

    @staticmethod
    def balanced(cm):
        return True

    def invoke(self, op, comment):
        if op == "invokedynamic":
            m = re.match(r"InvokeDynamic #(\d+):([\w$<>]+):(.*)", comment)
            idx, name, desc = int(m.group(1)), m.group(2), m.group(3)
            args, ret = parse_desc(desc)
            vals = [self.pop() for _ in args][::-1]
            kind, impl, cls = self.boots.get(idx, (None, None, None))
            if kind == "makeConcatWithConstants":
                p_expr = "concat(" + ", ".join(vals) + ")"
            elif impl:
                p_expr = f"lam<{impl}>({', '.join(vals)})"
            else:
                p_expr = f"dyn:{name}({', '.join(vals)})"
            self.p(p_expr)
            return
        m = METH.match(comment)
        if not m:
            self.emit(f"// ?? {op} {comment}")
            return
        owner, name, desc = short(m.group(1) or ""), m.group(2), m.group(3)
        args, ret = parse_desc(desc)
        vals = [self.pop() for _ in args][::-1]
        wide = ret in ("J", "D")
        if op == "invokestatic":
            call = f"{owner}.{name}({', '.join(vals)})"
        else:
            recv = self.pop()
            if name == "<init>":
                # constructor: replace the NEW placeholder wherever it sits on the stack
                cls = re.match(r"NEW#\d+:(.*)", recv)
                if cls:
                    expr = f"new {cls.group(1)}({', '.join(vals)})"
                    self.stack = [(expr if s == recv else s, w) for s, w in self.stack]
                    return
                self.emit(f"{recv}.<init>({', '.join(vals)});")
                return
            call = f"{recv}.{name}({', '.join(vals)})"
            if owner not in ("Object",) and False:
                pass
        if ret == "V":
            self.emit(call + ";")
        else:
            self.p(call, wide)


def main():
    argv = sys.argv[1:]
    flags = {a for a in argv if a.startswith("--")}
    args = [a for a in argv if not a.startswith("--")]
    if not args:
        sys.exit(__doc__)
    cls = resolve(args[0])
    pat = re.compile(args[1]) if len(args) > 1 else None
    text = load(cls, "--sig" in flags)
    if "--sig" in flags:
        for ln in text.splitlines():
            if pat is None or pat.search(ln):
                print(short_line(ln))
        return
    boots = bootstrap_table(text)
    # split the javap -v output into member blocks
    body = text.split("{", 1)[1] if "{" in text else text
    body = body.split("SourceFile:")[0]
    blocks = re.split(r"\n(?=  [a-zA-Z<].*\(.*\).*;\n|  [a-zA-Z].*;\n)", body)
    for blk in blocks:
        lines = blk.split("\n")
        head = lines[0].strip()
        if not head.endswith(";"):
            continue
        h = short_line(head)
        if "(" not in head and not head.startswith("static {"):
            if pat is None or pat.search(h):
                print(h)
            continue
        if pat is not None and not pat.search(h):
            continue
        meth = Method(head)
        in_code = False
        for ln in lines[1:]:
            if ln.strip().startswith("Code:"):
                in_code = True
                continue
            if not in_code:
                continue
            mi = INSN.match(ln)
            if mi:
                meth.code.append((int(mi.group(1)), mi.group(2), mi.group(3)))
            elif ln.strip().startswith(("LineNumberTable", "Exception table", "StackMapTable", "LocalVariable", "MethodParameters")):
                if ln.strip().startswith("Exception table"):
                    pass
                in_code = ln.strip().startswith("Exception table") and False
        print(f"\n{h}")
        if "--raw" in flags:
            for o, op, a in meth.code:
                print(f"  {o}: {op} {a}")
            continue
        sim = Sim(meth, boots)
        try:
            for l in sim.run():
                ind = "" if l.startswith("L") and l.endswith(":") else "  "
                print(ind + l)
        except Exception as ex:  # keep going on odd bytecode
            print(f"  // decode error: {ex}")


def short_line(ln):
    ln = ln.strip()
    return re.sub(r"[\w.$]+(?:\.[\w$]+)+", lambda m: short(m.group(0)), ln)


if __name__ == "__main__":
    main()
