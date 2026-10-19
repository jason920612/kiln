#!/bin/bash
# on the VM: run.sh script.py [args]  (TARGET=K|V selects the server the script drives)
cd ~/wp84 && KILN_PLAY_DIR=wp84 PYTHONPATH=. exec python3 "$@"
