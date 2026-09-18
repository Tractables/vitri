#!/bin/bash
# Wait for one sf job to leave RUNNING, then print the tail of its log.
job="$1"
while sf job "$job" 2>&1 | head -3 | grep -q "STATE=RUNNING"; do
  sleep 60
done
sf job "$job" 2>&1 | tail -70
