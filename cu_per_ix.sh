#!/usr/bin/env bash

# gather logs from tests
cargo test-bpf  > test.log 2<&1

# filter mango instructions and logging of consumed compute units
rg -oNI "(Mango:|Instruction: |Program 4uQeVj5tqViQh7yWWGStvkEG1Zmhx6uasJtWCJziofM consumed).*$" test.log \
  | rg -U 'Mango: .*\nProgram 4uQeVj5tqViQh7yWWGStvkEG1Zmhx6uasJtWCJziofM.*' \
  | awk 'NR % 2 == 1 { o=$0 ; next } { print o " " $0 }' \
  | sort | uniq -c | sort > consumed_per_instruction.log

rg -N 'Mango: (\w+) .* consumed (\d+) .*' consumed_per_instruction.log -r '$1,$2' \
  | uniq | xsv sort -s 2 -N -R \
  | sort -t ',' -k 1,1 -u \
  | sort > consumed_per_instruction_uniq.log

cat consumed_per_instruction_uniq.log| awk '{print $2}' | sort > consumed_per_instruction_uniq.log

rm test.log
rm consumed_per_instruction.log