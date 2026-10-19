# Reading the vanilla 26.3 classes (wp86)

`javap` output condensed into one line per instruction, to read what the vanilla dedicated server does:

    # once: unpack the classes of the server jar (needs work/versions/26.3/server-26.3.jar)
    python -X utf8 -I -c "import zipfile; z=zipfile.ZipFile('work/versions/26.3/server-26.3.jar'); z.extractall('/tmp/w86/jar', [n for n in z.namelist() if n.startswith('net/minecraft/')])"
    # a class, e.g. the management server's
    javap -c -p -constants -cp /tmp/w86/jar net.minecraft.server.jsonrpc.ManagementServer > /tmp/w86/out/ManagementServer.txt
    python -X utf8 -I tools/wp86/jtools/cond.py /tmp/w86/out/ManagementServer.txt ''          # all methods
    python -X utf8 -I tools/wp86/jtools/cond.py /tmp/w86/out/ManagementServer.txt 'tick'       # one method
    python3 tools/jpv.py net.minecraft.server.jsonrpc.ManagementServer                         # the older pseudo-Java view (fails on some methods)

`ldc S:...` are string constants, `invokedynamic` string concatenations show only their arguments (use
`javap -v` for the recipe), lambdas are separate `lambda$...` methods.
