#!/usr/bin/env python3

import fileinput
import sys
import re
import datetime

tolerance = .20

prev = None
prev_line = None

for line in fileinput.input():
    if re.match('^\d+\.\d+ ', line) is not None:
        current = float(line.split(' ')[0])
        if prev is not None:
            diff = current - prev
            if diff > tolerance:
                timestamp = str(datetime.datetime.now())
                print('>>> Threshold exceeded at ' + timestamp + ': ' + str(diff))
                print(prev_line)
                print(line)
        prev = current
        prev_line = line
