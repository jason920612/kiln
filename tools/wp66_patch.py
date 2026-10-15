"""wp66 helper: insert the text file's content before a marker line of a source file.
usage: wp66_patch.py FILE MARKER TEXTFILE"""
import sys
p, marker, text = sys.argv[1:4]
s = open(p, encoding="utf-8").read()
t = open(text, encoding="utf-8").read()
assert marker in s, marker
s = s.replace(marker, t + marker, 1)
open(p, "w", encoding="utf-8").write(s)
