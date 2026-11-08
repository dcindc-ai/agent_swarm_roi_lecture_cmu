#!/usr/bin/env bash
# Serve the orbit monitor and the live event log on one port, so the page can poll runs.jsonl.
set -e
cd "$(dirname "$0")"
mkdir -p out serve
ln -sfn ../monitor/orbits-monitor.html serve/orbits-monitor.html
ln -sfn ../out/runs.jsonl serve/runs.jsonl
touch out/runs.jsonl
echo "open http://localhost:8787/orbits-monitor.html and connect to http://localhost:8787/runs.jsonl"
cd serve && python3 -m http.server 8787
