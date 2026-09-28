"""Differential tests of Kiln's weather cycle, sky darkening, precipitation, night skip and
getting out of bed against vanilla 26.3.

1. Runs tools/WeatherVectors.java: a vanilla dedicated server started in-process (in a `server`
   directory next to the output, port 25596) that records the scenarios to
   <work>/wx-weather/vectors.jsonl.
2. Runs `cargo test -p kiln-sim --lib weather_parity` with KILN_WEATHER_VECTORS set, which
   replays each scenario through Kiln's code and compares the results.

usage: python tools/weather_vectors.py [--skip-java] [--out FILE]
"""

import argparse
import os
import subprocess
import sys
from pathlib import Path

sys.dont_write_bytecode = True
from vanilla_decode import ROOT, WORK, classpath  # noqa: E402


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--skip-java", action="store_true", help="reuse the existing vectors")
    ap.add_argument("--out", default=str(WORK / "wx-weather" / "vectors.jsonl"))
    args = ap.parse_args()
    out = Path(args.out).resolve()
    if not args.skip_java:
        server_dir = out.parent / "server"
        server_dir.mkdir(parents=True, exist_ok=True)
        cmd = ["java", "--add-opens", "java.base/java.lang=ALL-UNNAMED", "-cp", classpath(),
               str(ROOT / "tools" / "WeatherVectors.java"), str(out)]
        print("$ java ... WeatherVectors.java", out, flush=True)
        p = subprocess.run(cmd, cwd=server_dir, capture_output=True, text=True, encoding="utf-8", errors="replace")
        lines = [l for l in (p.stdout + p.stderr).splitlines()
                 if "WeatherVectors" in l or "Exception" in l or "Error" in l or "\tat " in l or "error:" in l or "symbol" in l or "location" in l or "^" in l]
        print("\n".join(lines[-60:]))
        if p.returncode != 0:
            sys.exit(f"WeatherVectors failed ({p.returncode})")
    env = dict(os.environ, KILN_WEATHER_VECTORS=str(out))
    sys.exit(subprocess.call(["cargo", "test", "-p", "kiln-sim", "--lib", "weather_parity", "--", "--nocapture",
                              "--test-threads=1"], cwd=ROOT, env=env))


if __name__ == "__main__":
    main()
