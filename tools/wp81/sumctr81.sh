#!/bin/bash
# wp81: sumctr81.sh FILE...   the spawn counters of one server per tick: ticks, active ticks of the MONSTER category, chunk visits per tick (a category with no cap), per active tick: monster chunk visits,
# picks, valid positions and mobs (validMob true), and the local cap's refusals. (K: ~/wp81/K/counters.out, V: ~/wp81/V/agent.out)
for f in "$@"; do
awk -F'\t' -v f="$f" '
/^global MONSTER chunks=289/ { if ($1 ~ /true$/) A+=$2; else F+=$2 }
/^validMob [a-z_]+ -> true/ && !/IN-BOX/ {m+=$2}
/^chunkAttempt MONSTER all/ {ca=$2}
/^chunkAttempt AMBIENT all/ {amb=$2}
/^localCap MONSTER -> true/ {lt=$2}
/^localCap MONSTER -> false/ {lf=$2}
/^validPosition [a-z_]+ -> true/ && !/IN-BOX/ {vt+=$2}
/^pick / && !/none/ {pk+=$2}
/^spawned [a-z_]+$/ {sp+=$2}
END {t=A+F; printf "%s: ticks=%d active=%d (%.2f%%) ambient visits/tick=%.1f | per active tick: monster visits=%.1f (cap refused %.1f%%) picks=%.1f validPos=%.3f validMob=%.3f\n", f, t, A, 100*A/t, amb/t, ca/A, 100*lf/(lt+lf), pk/A, vt/A, m/A}' "$f"
done
