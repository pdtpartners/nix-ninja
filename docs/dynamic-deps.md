# Dynamic dependency inference

> [!IMPORTANT]
> Please pre-read [dynamic derivations] and [design notes] as I'll assume you
> already understand Nix dynamic derivations and the existing nix-ninja
> architecture.

## The problem

Many real-world build systems generate source files at build-time that introduce
dependencies which cannot be known until after the generation step completes.
This creates a fundamental challenge: how do you handle dependencies that only
exist after a build step runs?

For example, consider this typical pattern:

```build.ninja
# Generate a C source file
build generated.c: CUSTOM_COMMAND generate.sh
  COMMAND = bash generate.sh

# Compile the generated source
build main.p/generated.c.o: CC generated.c
  ARGS = gcc -c generated.c
```

When `generate.sh` runs and produces `generated.c`, the file might contain:

```c
#include <stdio.h>
#include "config.h"

const char* get_example_name() {
    return EXAMPLE_NAME;
}
```

The dependency on `config.h` cannot be discovered until after `generated.c` is
created. When one generates a derivation for `main.p/generated.c.o` that lacks
the `config.h` input, the compilation will fail with  "config.h: No such file or
directory".

### Prior art

Previously, we had a temporary workaround with `$NIX_NINJA_EXTRA_INPUTS`:

```nix
nixNinjaExtraInputs = [
  "main.p/generated.c.o:config.h"
];
```

This let us avoid the problem while more fundamental pieces were taking shape,
but needed to be removed entirely when we had proper handling of dynamic
dependencies.

## Architecture

nix-ninja solves this problem through **dynamic task derivations** - a two-phase
approach that handles dependency discovery for tasks with `deps = gcc`. The
implementation is conditional and operates differently depending on whether
nix-ninja is running inside a Nix derivation (via `mkMesonPackage`) or
interactively outside Nix.

### Two execution modes

**Derivation mode (`is_output_derivation = true`):**
- Tasks with built inputs and `deps = gcc` generates a wrapper derivation
  known as a **dynamic task derivations**.
- They are passed a mostly complete derivation in JSON form and built inputs
  that require dependency discovery.
- In the dynamic task derivation, `nix-ninja -t dynamic-task` (a subtool) is
  executed to perform dependency discovery.
- Update task derivation to include all discovered dependencies.

**Interactive mode (`is_output_derivation = false`):**
- Tasks with built inputs and `deps = gcc` trigger local dynamic dependency
  discovery.
- `nix-ninja` builds these inputs locally, symlinks them to the build
  directory, and discovers dependencies.
  - This is different than derivation mode because we have no access to a
    `$src` attribute. Just like how `nix-ninja` in interactive mode has direct
    access to source files, so should dynamic dependency discovery.
- Update task derivation to include all discovered dependencies.

### Key insight: when dynamic dependencies are needed

Dynamic dependency handling is triggered when **both** conditions are met:
1. The task has `deps = "gcc"` (indicating it needs C include scanning)
2. The task depends on outputs from built derivations (`SingleDerivedPath::Built`)

Static tasks with only opaque file inputs (`SingleDerivedPath::Opaque`) can perform
dependency discovery directly during derivation generation, while dynamic tasks
need the two-phase approach because their inputs are not available until build time.

## Implementation details

### Core functions: `build_task_derivation` and `handle_derivation_result`

The implementation centers around two key functions in `crates/nix-ninja/src/task.rs`:

1. **`build_task_derivation`** - Generates the base task derivation with static
   dependencies and discovered dependencies for opaque inputs.
2. **`handle_derivation_result`** - Decides whether to use the derivation
   directly or wrap it with a dynamic task derivation.

## Example walkthrough

Let's trace through how nix-ninja handles a build graph with dynamic
dependencies:

```build.ninja
# Generate source file at build-time
build generated.c: GENERATE
  command = echo '#include "config.h"' > generated.c

# Compile generated source (this will need config.h)
build main.p/generated.c.o: CC generated.c
  deps = gcc

# Link final executable
build main: LINK main.c main.p/generated.c.o
```

### Derivation mode execution (`is_output_derivation = true`)

**Step 1:** nix-ninja generates task derivations

```
generated.c.drv:
  command: nix-ninja-task "echo '#include \"config.h\"' > generated.c"
  inputs: []
  outputs:
    generated.c: generated.c

generated.c.o.drv.drv: (dynamic task derivation)
  command: nix-ninja -t dynamic-task /nix/store/generated.c.o.drv.json
  inputs:
    generated.c.drv^generated.c (built input to scan)
    /nix/store/generated.c.o.drv.json
  outputs:
    out: generated.c.o.drv

main.drv: (static task - no gcc deps)
  command: nix-ninja-task "gcc -o main main.c main.p/generated.c.o"
  inputs:
    main.c
    generated.c.o.drv.drv^out^main.p-generated.c.o
  outputs:
    main: main
```

**Step 2:** Nix builds the derivation graph

1. `generated.c.drv` builds → produces `generated.c` with `#include "config.h"`
2. `generated.c.o.drv.drv` builds:
   - Scans `generated.c` → discovers dependency on `config.h`
   - Updates `generated.c.o.drv` JSON with `config.h` dependency
   - Outputs final `generated.c.o.drv` derivation
3. `generated.c.o.drv` builds → compiles `generated.c.o`
4. `main.drv` builds → links final executable

### Interactive mode execution (`is_output_derivation = false`)

**Step 1:** nix-ninja generates task derivations but handles discovery locally

```
generated.c.drv: (same as derivation mode)

generated.c.o.drv.json (same as derivation mode)
```

**Step 2:** Local dependency discovery

1. nix-ninja builds `generated.c.drv` and symlinks result to local build directory
2. Scans `generated.c` → discovers dependency on `config.h`
3. Updates `generated.c.o.drv` JSON with `config.h` dependency
4. Resume building task derivation out of modified JSON

### Key differences

- **Derivation mode** uses two-phase approach with intermediate dynamic task derivations
- **Interactive mode** performs discovery locally and updates derivations before submission
- Both modes use the same dependency discovery logic

[dynamic derivations]: ./dynamic-derivations.md
[design notes]: ./design.md
