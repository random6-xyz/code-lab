# code-lab

A Linux-only command-line coding lab. Problem statements and test cases are stored as individual YAML files; solutions are single-file Python, C, or Rust programs using standard input and standard output.

## Requirements

- Linux with user, mount, PID, and network namespaces available to the current user
- Rust toolchain and Cargo
- `nsjail`
- Python 3, GCC, and `rustc` for the corresponding submission languages

Submitted programs and C/Rust compilation run inside nsjail. If nsjail cannot establish the sandbox, code-lab fails closed and does not retry the program on the host.

## Build and run

```sh
cargo build --release
./target/release/code-lab list
./target/release/code-lab show sum-two
./target/release/code-lab run sum-two examples/solutions/sum_two.py
./target/release/code-lab run sum-two examples/solutions/sum_two.c
./target/release/code-lab run sum-two examples/solutions/sum_two.rs
```

The language is selected from the source extension: `.py`, `.c`, or `.rs`. C submissions are compiled as C17. Rust submissions are compiled as Rust 2021. Each test case runs in a fresh jailed process.

Use `--problems-dir PATH` to load problems from another directory:

```sh
code-lab --problems-dir ./my-problems list
code-lab --problems-dir ./my-problems run sum-two ./solution.py
```

## Problem YAML format

Problems are stored as `.yaml` or `.yml` files anywhere under the problems directory; nested directories are searched recursively. Each file's stem must exactly match its `id`.

```yaml
schema_version: 1
id: sum-two
title: Sum Two
statement: |
  Read two integers from standard input and print their sum.
tags:
  - math
  - beginner
time_limit_ms: 1000
memory_limit_mb: 128
test_cases:
  - name: positive-numbers
    input: |
      2 3
    expected_output: |
      5
  - name: negative-number
    input: "-4 9\n"
    expected_output: "5\n"
```

All fields except `tags` are required; tags default to an empty list and must be non-empty and unique when provided. Test case names must be unique within a problem. Time limits must be between 1 and 300,000 milliseconds; memory limits must be between 1 and 8,192 megabytes. Each limit applies independently to every test case. Compilation has separate fixed limits of 120 seconds and 2,048 MB.

Output comparison is byte-exact except that one final line ending (`LF` or `CRLF`) is ignored. Other whitespace, including extra trailing spaces or additional blank lines, is significant.

Use `code-lab show <id>` to view a problem's tags. The `list`, `show`, and `run` commands load and validate YAML problems recursively from the configured problems directory.

## Sandbox notes

- Submitted code runs as UID/GID 65534. `/usr`, `/lib`, `/lib64`, and the solution file are read-only mounts. The Rust sysroot is mounted read-only only while compiling Rust submissions.
- Writable runtime paths use bounded tmpfs mounts: 64 MiB for the workspace, 16 MiB for `/tmp`, and 64 MiB for artifacts. Compilation uses a separate 512 MiB workspace and artifact tmpfs, with a 64 MiB `/tmp`.
- The sandbox uses nsjail's isolated network and process namespaces, a read-only process view, resource limits, and a seccomp deny list for selected high-risk syscalls. Submission runtime code cannot create additional processes or threads; compilation has a separate policy that permits compiler subprocesses.
- Memory is constrained with `RLIMIT_AS`, which limits virtual address space rather than resident memory. Recognized allocation failures are reported as memory-limit exceeded; other memory failures may appear as runtime errors.
- Captured stdout and stderr are each limited to 1 MiB. Source files are limited to 1 MiB.
- nsjail and Linux namespaces are defense in depth, not a guarantee against kernel vulnerabilities. This MVP is not an audited security boundary for a public, multi-tenant service. Review and harden the sandbox before exposing it to arbitrary users.

## Tests

```sh
cargo test
```

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for the full text.
