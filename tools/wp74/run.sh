#!/bin/bash
# on the VM: run.sh script.py [args]  (TARGET=K|V selects the server the script drives)
cd ~/wp74 && KILN_PLAY_DIR=wp74 PYTHONPATH=. exec python3 "$@"
