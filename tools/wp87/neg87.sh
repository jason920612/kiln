#!/bin/bash
# wp87 negative checks: ~/kt wp87-gaps bash tools/wp87/neg87.sh [case ...]   (each fix switched off by a one-line edit must make its tests fail)
cd "$(dirname "$0")/../.." && exec python3 tools/wp87/neg87.py "$@"
