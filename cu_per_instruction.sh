#!/usr/bin/env bash

# gather logs from tests
cargo test-bpf  > test_bpf.log 2<&1

# filter mango instructions and logging of consumed compute units
rg -oNI "(Mango:|Instruction: |Program 4uQeVj5tqViQh7yWWGStvkEG1Zmhx6uasJtWCJziofM consumed).*$" test_bpf.log \
  # grab lines where this is consecutive
  | rg -U 'Mango: .*\nProgram 4uQeVj5tqViQh7yWWGStvkEG1Zmhx6uasJtWCJziofM.*' \
  # combine consecutive lines
  | awk 'NR % 2 == 1 { o=$0 ; next } { print o " " $0 }' \
  # sort and filter for uniqueness
  | sort | uniq -c | sort > consumed_per_instruction.log