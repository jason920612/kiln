#!/bin/bash
# on the VM: run.sh script.py [args]  (TARGET=K|V selects the server the script drives)
cd ~/wp80 && KILN_PLAY_DIR=wp80 PYTHONPATH=. exec python3 "$@"
