#!/usr/bin/env python3

import fileinput
import sys
import re

tolerance = .35

prev = None


for line in fileinput.input():
    if re.match('^\d+$', line) is not None:
        current = int(line)
        if prev is not None:
            diff = (current - prev) / 1_000_000_000
            if diff > tolerance:
                print('>>> Threshold exceeded: ' + str(diff))
        prev = current
    else:
        print(line, end='')
