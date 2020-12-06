#!/bin/bash

while true; do curl -f -s http://127.0.0.1:8099 > /dev/null && date +%s%N;  sleep .2; done | ./analyze_nanos.py
